use crate::commands::db::drop_db;
use crate::ui::Ui;
use crate::utils::{
    ensure_odoo_conf_local, ensure_venv, execute_command_streaming_with_env,
    execute_command_with_env, find_project_root, find_python_command, require_odoo_bin,
    StreamSource,
};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn execute(
    ui: &Ui,
    tags: &[String],
    heartbeat_seconds: u64,
    log_file: Option<&str>,
    no_log_file: bool,
    odoo_log_level: &str,
) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let python = find_python_command()?;

    // Find all modules in custom_addons
    let custom_addons_path = project_root.join("custom_addons");
    let modules = find_custom_modules(&custom_addons_path)?;

    if modules.is_empty() {
        return Err("No modules found in custom_addons".to_string());
    }

    // Generate unique database name using timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let db_name = format!("test_odoo_{}", timestamp);
    let run_id = timestamp.to_string();

    ui.heading("Running tests");
    ui.info(format!("Creating temporary database: {}", db_name));
    ui.info(format!("Found {} modules to install", modules.len()));

    let odoo_bin = require_odoo_bin(&project_root)?;

    let config_file = project_root.join("odoo.conf.local");
    let odoo_bin_str = odoo_bin.to_string_lossy().to_string();
    let config_file_str = config_file.to_string_lossy().to_string();

    // Preflight wkhtmltopdf to avoid hanging tests.
    let (wkhtml_path, wkhtml_dir) = detect_wkhtmltopdf()?;
    let path_env = build_path_env(&wkhtml_dir)?;
    let envs = [("PATH", path_env.as_str()), ("PYTHONUNBUFFERED", "1")];
    ui.info(format!("wkhtmltopdf: {}", wkhtml_path));

    // Always attempt to drop the temporary database.
    let dropped = Arc::new(AtomicBool::new(false));
    let cleanup_db = {
        let db_name = db_name.clone();
        let dropped = dropped.clone();
        let ui = ui.clone();
        move || {
            if dropped.swap(true, Ordering::SeqCst) {
                return;
            }
            ui.warn("Step 4: Cleaning up temporary database...");
            if let Err(drop_err) = drop_db(&ui, &db_name, true, true) {
                ui.warn(format!(
                    "Failed to drop temporary database {}: {}",
                    db_name, drop_err
                ));
                ui.info(format!(
                    "You may need to manually drop it: odx db drop --force --if-exists {}",
                    db_name
                ));
            } else {
                ui.success(format!("Temporary database {} dropped", db_name));
            }
        }
    };

    struct DropGuard<F: Fn()>(F);
    impl<F: Fn()> Drop for DropGuard<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }
    let _guard = DropGuard(cleanup_db.clone());

    // Ctrl+C cleanup (cross-platform)
    {
        let cleanup = cleanup_db.clone();
        ctrlc::set_handler(move || {
            eprintln!("Received Ctrl+C. Attempting to drop temporary database...");
            cleanup();
            std::process::exit(130);
        })
        .map_err(|e| format!("Failed to set Ctrl+C handler: {}", e))?;
    }

    // SIGTERM cleanup (unix)
    #[cfg(unix)]
    {
        use signal_hook::consts::signal::SIGTERM;
        use signal_hook::iterator::Signals;
        let cleanup = cleanup_db.clone();
        let mut signals =
            Signals::new([SIGTERM]).map_err(|e| format!("Failed to register SIGTERM: {}", e))?;
        std::thread::spawn(move || {
            if signals.forever().next().is_some() {
                eprintln!("Received SIGTERM. Attempting to drop temporary database...");
                cleanup();
                std::process::exit(143);
            }
        });
    }

    // Step 1: Create database and install base module
    println!("Step 1: Creating database and installing base module...");
    let args = vec![
        odoo_bin_str.as_str(),
        "-c",
        config_file_str.as_str(),
        "-d",
        db_name.as_str(),
        "--init",
        "base",
        "--no-http",
        "--stop-after-init",
        "--without-demo",
        "all",
    ];
    if let Err(e) = execute_command_with_env(&python, &args, Some(&project_root), &envs) {
        cleanup_db();
        return Err(e);
    }

    let tags_to_run = normalize_specs(tags);
    let tags_arg = tags_to_run.join(",");
    let session = resolve_test_session(
        &project_root,
        &run_id,
        &db_name,
        &tags_to_run,
        &modules,
        log_file,
        no_log_file,
    )?;

    let hb = if heartbeat_seconds == 0 {
        None
    } else {
        Some(Duration::from_secs(heartbeat_seconds))
    };

    let mut runs: Vec<TagRunResult> = Vec::new();
    let mut warnings: BTreeSet<String> = BTreeSet::new();
    let mut run_index: usize = 0;

    let modules_str = modules.join(",");
    println!(
        "Step 2: Installing {} modules and running tests with tags: {} (first log lines may take several minutes)...",
        modules.len(),
        tags_arg
    );
    run_one_selector(
        &python,
        &project_root,
        &envs,
        &odoo_bin_str,
        &config_file_str,
        &db_name,
        &modules_str,
        &tags_arg,
        odoo_log_level,
        hb,
        session.as_ref(),
        &mut run_index,
        &mut warnings,
        &mut runs,
        heartbeat_seconds,
    );

    if let Some(ref s) = session {
        finalize_test_session(s, &runs, &warnings)?;
    }

    print_summary(&runs, &warnings, session.as_ref());

    cleanup_db();

    if runs.iter().any(|r| !r.passed) {
        return Err("One or more test runs failed".to_string());
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_one_selector(
    python: &str,
    project_root: &Path,
    envs: &[(&str, &str)],
    odoo_bin_str: &str,
    config_file_str: &str,
    db_name: &str,
    modules_init: &str,
    test_tags: &str,
    odoo_log_level: &str,
    hb: Option<Duration>,
    session: Option<&TestSession>,
    run_index: &mut usize,
    warnings: &mut BTreeSet<String>,
    runs: &mut Vec<TagRunResult>,
    heartbeat_seconds: u64,
) {
    println!("===== Executing: {} =====", test_tags);

    let mut parser = OutputParser::new();
    let heartbeat_msg = if heartbeat_seconds == 0 {
        "===== Still running tests (no heartbeat configured) =====".to_string()
    } else {
        format!(
            "===== Still running tests: {} (no output for {}s) =====",
            test_tags, heartbeat_seconds
        )
    };

    let (combined_log, run_log_rel, mut run_log_file) = match session {
        Some(s) => {
            let filename = sanitize_selector_filename(*run_index, test_tags);
            *run_index += 1;
            let run_log_path = s.runs_dir.join(&filename);
            let rel = format!("runs/{}", filename);
            if let Some(parent) = run_log_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let mut f = match fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&run_log_path)
            {
                Ok(file) => Some(file),
                Err(e) => {
                    eprintln!(
                        "Warning: failed to open run log {}: {}",
                        run_log_path.display(),
                        e
                    );
                    None
                }
            };
            if let Some(file) = f.as_mut() {
                let _ = writeln!(file, "===== selector: {} =====", test_tags);
            }
            let _ =
                append_session_line(&s.combined_log, &format!("===== run: {} =====", test_tags));
            (Some(s.combined_log.as_path()), Some(rel), f)
        }
        None => (None, None, None),
    };

    let log_level_arg = format!("--log-level={}", odoo_log_level);
    let args = vec![
        odoo_bin_str,
        "-c",
        config_file_str,
        "-d",
        db_name,
        "--init",
        modules_init,
        "--test-enable",
        "--test-tags",
        test_tags,
        "--no-http",
        "--stop-after-init",
        "--without-demo",
        "all",
        log_level_arg.as_str(),
    ];

    let result = execute_command_streaming_with_env(
        python,
        &args,
        Some(project_root),
        envs,
        |src, line| {
            match src {
                StreamSource::Stdout => println!("{}", line),
                StreamSource::Stderr => eprintln!("{}", line),
            }
            parser.ingest(line);
            if let Some(file) = run_log_file.as_mut() {
                let _ = writeln!(file, "{}", line);
            }
        },
        combined_log,
        hb,
        &heartbeat_msg,
    );

    for w in parser.warnings.iter() {
        if warnings.len() < 200 {
            warnings.insert(w.to_string());
        }
    }

    if let Some(s) = session {
        if !parser.warnings.is_empty() {
            let _ = append_session_line(&s.warnings_log, &format!("=== {} ===", test_tags));
            for w in &parser.warnings {
                let _ = append_session_line(&s.warnings_log, w);
            }
            let _ = append_session_line(&s.warnings_log, "");
        }
    }

    parser.flush_failure_block();
    let log_file = run_log_rel.unwrap_or_default();
    match result {
        Ok(_) => {
            println!("===== Test passed: {} =====", test_tags);
            runs.push(TagRunResult::from_parser(
                test_tags.to_string(),
                true,
                None,
                log_file,
                &parser,
            ));
        }
        Err(err) => {
            eprintln!("===== Test failed: {} =====", test_tags);
            eprintln!("{}", err);
            runs.push(TagRunResult::from_parser(
                test_tags.to_string(),
                false,
                Some(err),
                log_file,
                &parser,
            ));
        }
    }
}

fn detect_wkhtmltopdf() -> Result<(String, PathBuf), String> {
    let p = which::which("wkhtmltopdf").map_err(|_| {
        "wkhtmltopdf not found on this system. Install wkhtmltopdf 0.12.6.1 (with patched qt)."
            .to_string()
    })?;
    let path_str = p.to_string_lossy().to_string();

    // Best-effort version check
    let out = std::process::Command::new(&path_str)
        .arg("-V")
        .output()
        .ok();
    if let Some(o) = out {
        let text = format!(
            "{} {}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        if !text.contains("0.12.6.1") || !text.to_lowercase().contains("patched qt") {
            eprintln!(
                "Warning: wkhtmltopdf version does not look like 0.12.6.1 (with patched qt). Output: {}",
                text.trim()
            );
        }
    }

    let dir = p
        .parent()
        .ok_or_else(|| format!("Invalid wkhtmltopdf path: {}", path_str))?
        .to_path_buf();
    Ok((path_str, dir))
}

fn build_path_env(extra_dir: &Path) -> Result<String, String> {
    let current = std::env::var("PATH").unwrap_or_default();
    let extra = extra_dir.to_string_lossy();
    // Put wkhtmltopdf directory first to ensure Odoo finds it.
    Ok(format!("{}:{}", extra, current))
}

fn normalize_specs(raw: &[String]) -> Vec<String> {
    let mut out = Vec::<String>::new();
    for t in raw {
        let s = t.trim();
        if !s.is_empty() {
            out.push(s.to_string());
        }
    }
    if out.is_empty() {
        out.push("+standard".to_string());
    }
    out
}

#[derive(Debug, Clone)]
struct TestSession {
    dir: PathBuf,
    run_id: String,
    db_name: String,
    combined_log: PathBuf,
    warnings_log: PathBuf,
    runs_dir: PathBuf,
}

#[derive(Debug, Serialize)]
struct SessionMeta {
    run_id: String,
    db_name: String,
    tags: Vec<String>,
    modules: Vec<String>,
    started_at: u64,
    finished_at: Option<u64>,
    total_runs: usize,
    passed: usize,
    failed: usize,
}

#[derive(Debug, Serialize)]
struct TestReport {
    session: SessionMeta,
    runs: Vec<TagRunResult>,
    warnings_unique: usize,
}

fn resolve_test_session(
    project_root: &Path,
    run_id: &str,
    db_name: &str,
    tags: &[String],
    modules: &[String],
    log_file: Option<&str>,
    no_log_file: bool,
) -> Result<Option<TestSession>, String> {
    if no_log_file {
        return Ok(None);
    }

    let dir = project_root.join(".testing").join("sessions").join(run_id);
    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "Failed to create session directory {}: {}",
            dir.display(),
            e
        )
    })?;

    let runs_dir = dir.join("runs");
    fs::create_dir_all(&runs_dir).map_err(|e| {
        format!(
            "Failed to create runs directory {}: {}",
            runs_dir.display(),
            e
        )
    })?;

    let combined_log = if let Some(p) = log_file {
        let pb = PathBuf::from(p);
        if pb.is_absolute() {
            pb
        } else {
            project_root.join(pb)
        }
    } else {
        dir.join("combined.log")
    };

    if let Some(parent) = combined_log.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create combined log directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }

    let warnings_log = dir.join("warnings.log");
    let session = TestSession {
        dir: dir.clone(),
        run_id: run_id.to_string(),
        db_name: db_name.to_string(),
        combined_log,
        warnings_log,
        runs_dir,
    };

    let meta = SessionMeta {
        run_id: run_id.to_string(),
        db_name: db_name.to_string(),
        tags: tags.to_vec(),
        modules: modules.to_vec(),
        started_at: run_id.parse().unwrap_or(0),
        finished_at: None,
        total_runs: 0,
        passed: 0,
        failed: 0,
    };
    let session_path = dir.join("session.json");
    let json = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("Failed to serialize session.json: {}", e))?;
    fs::write(&session_path, json)
        .map_err(|e| format!("Failed to write {}: {}", session_path.display(), e))?;

    Ok(Some(session))
}

fn finalize_test_session(
    session: &TestSession,
    runs: &[TagRunResult],
    warnings: &BTreeSet<String>,
) -> Result<(), String> {
    let passed = runs.iter().filter(|r| r.passed).count();
    let failed = runs.len() - passed;
    let finished_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let meta = SessionMeta {
        run_id: session.run_id.clone(),
        db_name: session.db_name.clone(),
        tags: read_session_tags(&session.dir)?,
        modules: read_session_modules(&session.dir)?,
        started_at: session.run_id.parse().unwrap_or(0),
        finished_at: Some(finished_at),
        total_runs: runs.len(),
        passed,
        failed,
    };

    let session_path = session.dir.join("session.json");
    let json = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("Failed to serialize session.json: {}", e))?;
    fs::write(&session_path, json)
        .map_err(|e| format!("Failed to write {}: {}", session_path.display(), e))?;

    let report = TestReport {
        session: meta,
        runs: runs.to_vec(),
        warnings_unique: warnings.len(),
    };
    let report_path = session.dir.join("report.json");
    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("Failed to serialize report.json: {}", e))?;
    fs::write(&report_path, report_json)
        .map_err(|e| format!("Failed to write {}: {}", report_path.display(), e))?;

    link_latest_session(session)?;
    Ok(())
}

fn read_session_tags(dir: &Path) -> Result<Vec<String>, String> {
    let path = dir.join("session.json");
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let v: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    Ok(v.get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default())
}

fn read_session_modules(dir: &Path) -> Result<Vec<String>, String> {
    let path = dir.join("session.json");
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let v: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    Ok(v.get("modules")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default())
}

fn link_latest_session(session: &TestSession) -> Result<(), String> {
    let sessions_root = session
        .dir
        .parent()
        .ok_or_else(|| "Invalid session directory".to_string())?;
    let latest = sessions_root.join("latest");
    let _ = fs::remove_file(&latest);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            session.dir.file_name().ok_or("Invalid session dir name")?,
            &latest,
        )
        .map_err(|e| format!("Failed to create latest symlink: {}", e))?;
    }
    #[cfg(windows)]
    {
        let _ = session;
        let _ = latest;
    }
    Ok(())
}

fn append_session_line(path: &Path, line: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create log directory {}: {}", parent.display(), e))?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    writeln!(f, "{}", line).map_err(|e| format!("Failed to write to {}: {}", path.display(), e))?;
    Ok(())
}

fn sanitize_selector_filename(index: usize, selector: &str) -> String {
    let re = Regex::new(r"_+").unwrap();
    let sanitized: String = selector
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let collapsed = re.replace_all(&sanitized, "_");
    let trimmed = collapsed.trim_matches('_');
    let body = if trimmed.is_empty() {
        "selector"
    } else if trimmed.len() > 80 {
        &trimmed[..80]
    } else {
        trimmed
    };
    format!("{:03}__{}.log", index, body)
}

#[derive(Debug)]
struct OutputParser {
    warnings: Vec<String>,
    failure_blocks: Vec<String>,
    ran_tests: Option<u32>,
    failed: bool,
    skipped: Option<u32>,
    failures: Option<u32>,
    errors: Option<u32>,
    in_failure_block: bool,
    current_block: Option<Vec<String>>,
    warning_re: Regex,
    ran_re: Regex,
    failed_re: Regex,
    fail_block_re: Regex,
    separator_re: Regex,
}

impl OutputParser {
    fn new() -> Self {
        Self {
            warnings: Vec::new(),
            failure_blocks: Vec::new(),
            ran_tests: None,
            failed: false,
            skipped: None,
            failures: None,
            errors: None,
            in_failure_block: false,
            current_block: None,
            warning_re: Regex::new(
                r"\b(DeprecationWarning|PendingDeprecationWarning|FutureWarning|UserWarning|RuntimeWarning|ResourceWarning|SyntaxWarning|ImportWarning)\b",
            )
            .unwrap(),
            ran_re: Regex::new(r"^Ran\s+(\d+)\s+tests?\s+in\s+").unwrap(),
            failed_re: Regex::new(r"^FAILED\s*\((.+)\)\s*$").unwrap(),
            fail_block_re: Regex::new(r"^(FAIL|ERROR):").unwrap(),
            separator_re: Regex::new(r"^-{10,}$").unwrap(),
        }
    }

    fn ingest(&mut self, line: &str) {
        self.collect_warning(line);
        self.parse_failure_block(line);
        self.parse_unittest_summary(line);
    }

    fn flush_failure_block(&mut self) {
        if let Some(block) = self.current_block.take() {
            if !block.is_empty() && self.failure_blocks.len() < 20 {
                self.failure_blocks.push(block.join("\n"));
            }
        }
        self.in_failure_block = false;
    }

    fn parse_failure_block(&mut self, line: &str) {
        let trimmed = line.trim();
        if self.fail_block_re.is_match(trimmed) {
            self.flush_failure_block();
            self.in_failure_block = true;
            self.current_block = Some(vec![line.to_string()]);
            return;
        }
        if self.in_failure_block {
            if self.separator_re.is_match(trimmed) || self.ran_re.is_match(line) {
                self.flush_failure_block();
                return;
            }
            if let Some(block) = &mut self.current_block {
                if block.len() < 100 {
                    block.push(line.to_string());
                }
            }
        }
    }

    fn collect_warning(&mut self, line: &str) {
        if self.warning_re.is_match(line) {
            self.warnings.push(line.to_string());
        }
        if line.contains(" WARNING ") || line.starts_with("WARNING") {
            self.warnings.push(line.to_string());
        }
    }

    fn parse_unittest_summary(&mut self, line: &str) {
        if let Some(caps) = self.ran_re.captures(line) {
            if let Ok(n) = caps.get(1).unwrap().as_str().parse::<u32>() {
                self.ran_tests = Some(n);
            }
        }

        if line.trim() == "OK" {
            self.failed = false;
        }

        if let Some(caps) = self.failed_re.captures(line.trim()) {
            self.failed = true;
            let inside = caps.get(1).unwrap().as_str();
            let mut map: HashMap<&str, u32> = HashMap::new();
            for part in inside.split(',') {
                let part = part.trim();
                if let Some((k, v)) = part.split_once('=') {
                    if let Ok(n) = v.trim().parse::<u32>() {
                        map.insert(k.trim(), n);
                    }
                }
            }
            self.failures = map.get("failures").copied();
            self.errors = map.get("errors").copied();
            self.skipped = map.get("skipped").copied();
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct TagRunResult {
    selector: String,
    passed: bool,
    exit_error: Option<String>,
    ran_tests: Option<u32>,
    failures: Option<u32>,
    errors: Option<u32>,
    skipped: Option<u32>,
    log_file: String,
    failure_blocks: Vec<String>,
    warnings: Vec<String>,
}

impl TagRunResult {
    fn from_parser(
        selector: String,
        passed: bool,
        exit_error: Option<String>,
        log_file: String,
        parser: &OutputParser,
    ) -> Self {
        Self {
            selector,
            passed,
            exit_error,
            ran_tests: parser.ran_tests,
            failures: parser.failures,
            errors: parser.errors,
            skipped: parser.skipped,
            log_file,
            failure_blocks: parser.failure_blocks.clone(),
            warnings: parser.warnings.clone(),
        }
    }
}

fn print_summary(
    runs: &[TagRunResult],
    warnings: &BTreeSet<String>,
    session: Option<&TestSession>,
) {
    let total = runs.len();
    let passed = runs.iter().filter(|r| r.passed).count();
    let failed = total - passed;

    println!();
    println!("================= Test Summary =================");
    println!("Total runs: {}", total);
    println!("Passed: {}", passed);
    println!("Failed: {}", failed);

    let total_skipped: u32 = runs.iter().filter_map(|r| r.skipped).sum();
    let have_skipped = runs.iter().any(|r| r.skipped.is_some());
    if have_skipped {
        println!("Skipped (parsed): {}", total_skipped);
    }

    if failed > 0 {
        println!();
        println!("Failures:");
        for r in runs.iter().filter(|r| !r.passed) {
            println!(
                "- {} (ran={:?}, failures={:?}, errors={:?}, skipped={:?})",
                r.selector, r.ran_tests, r.failures, r.errors, r.skipped
            );
            if !r.log_file.is_empty() {
                println!("  log: {}", r.log_file);
            }
            if let Some(err) = &r.exit_error {
                println!("  exit: {}", err);
            }
            if let Some(block) = r.failure_blocks.first() {
                let preview: String = block.lines().take(8).collect::<Vec<_>>().join("\n");
                println!("  failure preview:\n{}", indent_lines(&preview, "    "));
            }
        }
    }

    if !warnings.is_empty() {
        println!();
        println!("Warnings (unique, console capped): {}", warnings.len());
        for w in warnings.iter().take(50) {
            println!("- {}", w);
        }
        if warnings.len() > 50 {
            println!("... ({} more)", warnings.len() - 50);
        }
    }

    if let Some(s) = session {
        println!();
        println!("Session artifacts: {}", s.dir.display());
        println!("  report.json");
        println!("  combined.log: {}", s.combined_log.display());
        println!("  warnings.log: {}", s.warnings_log.display());
        println!(
            "  runs/ ({} files)",
            runs.iter().filter(|r| !r.log_file.is_empty()).count()
        );
    }

    println!("================================================");
}

fn indent_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|l| format!("{}{}", prefix, l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_custom_modules(custom_addons_path: &std::path::Path) -> Result<Vec<String>, String> {
    if !custom_addons_path.exists() {
        return Ok(vec![]);
    }

    let mut modules = Vec::new();

    for entry in std::fs::read_dir(custom_addons_path)
        .map_err(|e| format!("Failed to read custom_addons directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();

        if path.is_dir() {
            // Look for __manifest__.py in this directory
            let manifest = path.join("__manifest__.py");
            if manifest.exists() {
                if let Some(module_name) = path.file_name().and_then(|n| n.to_str()) {
                    modules.push(module_name.to_string());
                }
            } else {
                // Check subdirectories (for namespace packages)
                if let Ok(subdirs) = std::fs::read_dir(&path) {
                    for subentry in subdirs.flatten() {
                        let subpath = subentry.path();
                        if subpath.is_dir() {
                            let submanifest = subpath.join("__manifest__.py");
                            if submanifest.exists() {
                                if let Some(module_name) =
                                    subpath.file_name().and_then(|n| n.to_str())
                                {
                                    modules.push(module_name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(modules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_specs_defaults_to_standard() {
        assert_eq!(normalize_specs(&[]), vec!["+standard".to_string()]);
    }

    #[test]
    fn normalize_specs_joins_multiple_tags() {
        let specs = normalize_specs(&["intn".to_string(), "/mod:Test.test_x".to_string()]);
        assert_eq!(specs, vec!["intn", "/mod:Test.test_x"]);
        assert_eq!(specs.join(","), "intn,/mod:Test.test_x");
    }

    #[test]
    fn normalize_specs_preserves_full_selector_with_tag() {
        let specs = normalize_specs(&["intn,/partner_vat_unique:TestX.test_y".to_string()]);
        assert_eq!(specs, vec!["intn,/partner_vat_unique:TestX.test_y"]);
        assert_eq!(specs.join(","), "intn,/partner_vat_unique:TestX.test_y");
    }

    #[test]
    fn sanitize_selector_filename_replaces_special_chars() {
        let name = sanitize_selector_filename(1, "+standard,/mod:Cls.test");
        assert!(name.starts_with("001__"));
        assert!(name.ends_with(".log"));
        assert!(!name.contains('/'));
        assert!(!name.contains(':'));
    }

    #[test]
    fn output_parser_captures_fail_block() {
        let mut parser = OutputParser::new();
        parser.ingest("FAIL: test_foo (mod.TestBar)");
        parser.ingest("Traceback (most recent call last):");
        parser.ingest("AssertionError: boom");
        parser.ingest("----------------------------------------------------------------------");
        parser.ingest("Ran 1 tests in 0.001s");
        parser.ingest("FAILED (failures=1)");
        parser.flush_failure_block();

        assert_eq!(parser.failure_blocks.len(), 1);
        assert!(parser.failure_blocks[0].contains("FAIL: test_foo"));
        assert!(parser.failure_blocks[0].contains("AssertionError"));
        assert_eq!(parser.ran_tests, Some(1));
        assert_eq!(parser.failures, Some(1));
    }

    #[test]
    fn output_parser_captures_error_block() {
        let mut parser = OutputParser::new();
        parser.ingest("ERROR: test_baz (mod.TestQux)");
        parser.ingest("Exception: kaboom");
        parser.flush_failure_block();

        assert_eq!(parser.failure_blocks.len(), 1);
        assert!(parser.failure_blocks[0].contains("ERROR: test_baz"));
    }
}
