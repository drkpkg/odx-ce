use crate::commands::db::drop_db;
use crate::ui::Ui;
use crate::utils::{
    ensure_odoo_conf_local, execute_command_with_env, execute_command_streaming_with_env,
    find_project_root, find_python_command, ensure_venv, StreamSource,
};
use regex::Regex;
use std::collections::{BTreeSet, HashMap};
use std::fs;
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

    ui.heading("Running tests");
    ui.info(format!("Creating temporary database: {}", db_name));
    ui.info(format!("Found {} modules to install", modules.len()));

    let odoo_bin = project_root.join("src/odoo/odoo-bin");
    if !odoo_bin.exists() {
        return Err(format!("odoo-bin not found: {}", odoo_bin.display()));
    }

    let config_file = project_root.join("odoo.conf.local");
    let odoo_bin_str = odoo_bin.to_string_lossy().to_string();
    let config_file_str = config_file.to_string_lossy().to_string();

    // Preflight wkhtmltopdf to avoid hanging tests.
    let (wkhtml_path, wkhtml_dir) = detect_wkhtmltopdf()?;
    let path_env = build_path_env(&wkhtml_dir)?;
    let envs = [("PATH", path_env.as_str())];
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
            for _ in signals.forever() {
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
        "--init", "base",
        "--stop-after-init",
        "--without-demo", "all",
    ];
    if let Err(e) = execute_command_with_env(&python, &args, Some(&project_root), &envs) {
        cleanup_db();
        return Err(e);
    }

    // Step 2: Install all custom_addons modules
    println!("Step 2: Installing custom_addons modules...");
    let modules_str = modules.join(",");
    let args = vec![
        odoo_bin_str.as_str(),
        "-c",
        config_file_str.as_str(),
        "-d",
        db_name.as_str(),
        "--init", modules_str.as_str(),
        "--stop-after-init",
        "--without-demo", "all",
    ];
    if let Err(e) = execute_command_with_env(&python, &args, Some(&project_root), &envs) {
        cleanup_db();
        return Err(e);
    }

    // Step 3: Run tests
    println!("Step 3: Running tests...");
    let tags_to_run = normalize_specs(tags);
    let log_path = resolve_log_path(&project_root, &db_name, log_file, no_log_file)?;

    let hb = if heartbeat_seconds == 0 {
        None
    } else {
        Some(Duration::from_secs(heartbeat_seconds))
    };

    let mut runs: Vec<TagRunResult> = Vec::new();
    let mut warnings: BTreeSet<String> = BTreeSet::new();

    // Discovery (always): find test methods under each module tests/ tree.
    // Hybrid execution: per-method for small classes, per-class otherwise.
    let methods = discover_test_methods(&project_root, &modules)?;
    let class_map = group_methods_by_module_class(&methods);

    for spec in tags_to_run {
        println!("===== Executing tag: {} =====", spec);
        let effective_spec = effective_spec_for_execution(&spec);
        let target = parse_structural_selector(&effective_spec);
        let module_filter = extract_module_filter(&effective_spec);

        for module in modules.iter() {
            if let Some(mf) = &module_filter {
                if mf != module {
                    continue;
                }
            }

            let classes = match class_map.get(module) {
                Some(c) => c,
                None => continue,
            };

            println!("===== Module: {} =====", module);

            let mut class_names: Vec<&String> = classes.keys().collect();
            class_names.sort();

            // If the user requested a specific class (and optionally method),
            // only execute that class within this module.
            if let Some(t) = &target {
                if let Some(target_module) = &t.module {
                    if target_module != module {
                        continue;
                    }
                }
                if let Some(target_class) = &t.class_name {
                    class_names.retain(|c| c.as_str() == target_class);
                }
            }

            // If the user requested a specific method, we will only execute that method
            // in its class and skip per-method discovery expansion.
            if let Some(t) = &target {
                if let Some(target_method) = &t.method_name {
                    if let Some(target_class) = &t.class_name {
                        // Ensure the class exists for this module.
                        if let Some(method_names) = classes.get(target_class) {
                            if method_names.iter().any(|m| m == target_method) {
                                println!(
                                    "===== Class: {} (1 method) =====",
                                    target_class
                                );
                                println!("===== Method 1/1: {} =====", target_method);

                                // Run exactly the effective selector (already class.method-specific).
                                run_one_selector(
                                    &python,
                                    &project_root,
                                    &envs,
                                    &odoo_bin_str,
                                    &config_file_str,
                                    &db_name,
                                    &effective_spec,
                                    odoo_log_level,
                                    hb,
                                    log_path.as_deref(),
                                    &mut warnings,
                                    &mut runs,
                                    heartbeat_seconds,
                                );
                            } else {
                                eprintln!(
                                    "Warning: requested method {}.{} not discovered in module {}",
                                    target_class, target_method, module
                                );
                            }
                        } else {
                            eprintln!(
                                "Warning: requested class {} not discovered in module {}",
                                target_class, module
                            );
                        }
                    } else {
                        // Method without class is not supported by Odoo selector grammar;
                        // fall back to existing behavior.
                    }
                    // Done with this module for this spec.
                    continue;
                }
            }

            for class_name in class_names {
                let method_names = &classes[class_name];
                if method_names.is_empty() {
                    continue;
                }

                println!(
                    "===== Class: {} ({} methods) =====",
                    class_name,
                    method_names.len()
                );

                let method_threshold: usize = 20;
                if method_names.len() <= method_threshold {
                    for (idx, method_name) in method_names.iter().enumerate() {
                        println!(
                            "===== Method {}/{}: {} =====",
                            idx + 1,
                            method_names.len(),
                            method_name
                        );
                        let selector =
                            inject_selector_method(&effective_spec, module, class_name, method_name);
                        run_one_selector(
                            &python,
                            &project_root,
                            &envs,
                            &odoo_bin_str,
                            &config_file_str,
                            &db_name,
                            &selector,
                            odoo_log_level,
                            hb,
                            log_path.as_deref(),
                            &mut warnings,
                            &mut runs,
                            heartbeat_seconds,
                        );
                    }
                } else {
                    let selector = inject_selector_class(&effective_spec, module, class_name);
                    run_one_selector(
                        &python,
                        &project_root,
                        &envs,
                        &odoo_bin_str,
                        &config_file_str,
                        &db_name,
                        &selector,
                        odoo_log_level,
                        hb,
                        log_path.as_deref(),
                        &mut warnings,
                        &mut runs,
                        heartbeat_seconds,
                    );
                }
            }
        }
    }

    print_summary(&runs, &warnings, log_path.as_deref());

    cleanup_db();

    if runs.iter().any(|r| !r.passed) {
        return Err("One or more test runs failed".to_string());
    }

    Ok(())
}

fn run_one_selector(
    python: &str,
    project_root: &Path,
    envs: &[(&str, &str)],
    odoo_bin_str: &str,
    config_file_str: &str,
    db_name: &str,
    selector: &str,
    odoo_log_level: &str,
    hb: Option<Duration>,
    log_path: Option<&Path>,
    warnings: &mut BTreeSet<String>,
    runs: &mut Vec<TagRunResult>,
    heartbeat_seconds: u64,
) {
    println!("===== Executing: {} =====", selector);

    let mut parser = OutputParser::new();
    let heartbeat_msg = if heartbeat_seconds == 0 {
        "===== Still running tests (no heartbeat configured) =====".to_string()
    } else {
        format!(
            "===== Still running tests: {} (no output for {}s) =====",
            selector, heartbeat_seconds
        )
    };

    let log_level_arg = format!("--log-level={}", odoo_log_level);
    let args = vec![
        odoo_bin_str,
        "-c",
        config_file_str,
        "-d",
        db_name,
        "--test-tags",
        selector,
        "--stop-after-init",
        log_level_arg.as_str(),
    ];

    let result = execute_command_streaming_with_env(
        python,
        &args,
        Some(project_root),
        envs,
        |src, line| match src {
            StreamSource::Stdout => {
                println!("{}", line);
                parser.ingest(line);
            }
            StreamSource::Stderr => {
                eprintln!("{}", line);
                parser.ingest(line);
            }
        },
        log_path,
        hb,
        &heartbeat_msg,
    );

    for w in parser.warnings.iter() {
        if warnings.len() < 200 {
            warnings.insert(w.to_string());
        }
    }

    match result {
        Ok(_) => {
            println!("===== Test passed: {} =====", selector);
            runs.push(TagRunResult::from_parser(selector.to_string(), true, &parser));
        }
        Err(err) => {
            eprintln!("===== Test failed: {} =====", selector);
            eprintln!("{}", err);
            runs.push(TagRunResult::from_parser(selector.to_string(), false, &parser));
        }
    }
}

fn detect_wkhtmltopdf() -> Result<(String, PathBuf), String> {
    let p = which::which("wkhtmltopdf")
        .map_err(|_| "wkhtmltopdf not found on this system. Install wkhtmltopdf 0.12.6.1 (with patched qt).".to_string())?;
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

#[derive(Debug, Clone)]
struct MethodSpec {
    module: String,
    class_name: String,
    method_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuralSelector {
    module: Option<String>,
    class_name: Option<String>,
    method_name: Option<String>,
}

fn parse_structural_selector(spec: &str) -> Option<StructuralSelector> {
    // Odoo selector grammar (relevant subset):
    //   [-][tag][/module][:class][.method]
    // We only parse the first structural selector occurrence and ignore tag-only parts.
    //
    // Examples:
    // - "intn,/partner_vat_unique:TestX.test_y" -> module=partner_vat_unique,class=TestX,method=test_y
    // - "/partner_vat_unique:TestX" -> module=partner_vat_unique,class=TestX
    // - ":TestX.test_y" -> class=TestX,method=test_y
    let re = Regex::new(
        r"(?x)
        (?:
            /(?P<module>[A-Za-z0-9_]+)
        )?
        (?:
            :(?P<class>[A-Za-z_][A-Za-z0-9_]*)
        )?
        (?:
            \.(?P<method>[A-Za-z0-9_]+)
        )?
        ",
    )
    .unwrap();

    for raw_part in spec.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        // Skip pure tag-only parts early.
        if !raw_part.contains('/') && !raw_part.contains(':') && !raw_part.contains('.') {
            continue;
        }

        // Strip a leading '-' (exclude spec) because we only care about the structural target.
        let part = raw_part.strip_prefix('-').unwrap_or(raw_part);
        if let Some(caps) = re.captures(part) {
            let module = caps.name("module").map(|m| m.as_str().to_string());
            let class_name = caps.name("class").map(|m| m.as_str().to_string());
            let method_name = caps.name("method").map(|m| m.as_str().to_string());

            if module.is_none() && class_name.is_none() && method_name.is_none() {
                continue;
            }
            return Some(StructuralSelector {
                module,
                class_name,
                method_name,
            });
        }
    }
    None
}

fn has_any_structural_selector(spec: &str) -> bool {
    parse_structural_selector(spec).is_some()
}

fn effective_spec_for_execution(spec: &str) -> String {
    // If the spec includes a structural selector (module/class/method),
    // strip tag-only parts like "intn" to avoid pulling unrelated suites.
    //
    // Keep any comma parts which themselves contain structural selectors.
    if !has_any_structural_selector(spec) {
        return spec.trim().to_string();
    }
    let kept: Vec<&str> = spec
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter(|p| p.contains('/') || p.contains(':') || p.contains('.'))
        .collect();

    if kept.is_empty() {
        // Defensive: if parsing said structural exists but we filtered all parts out,
        // fall back to original spec.
        return spec.trim().to_string();
    }
    kept.join(",")
}

fn group_methods_by_module_class(
    methods: &[MethodSpec],
) -> HashMap<String, HashMap<String, Vec<String>>> {
    let mut out: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for m in methods {
        out.entry(m.module.clone())
            .or_default()
            .entry(m.class_name.clone())
            .or_default()
            .push(m.method_name.clone());
    }
    for (_module, classes) in out.iter_mut() {
        for (_class, method_names) in classes.iter_mut() {
            method_names.sort();
        }
    }
    out
}

fn extract_module_filter(spec: &str) -> Option<String> {
    let re = Regex::new(r"/([A-Za-z0-9_]+)").unwrap();
    for part in spec.split(',').map(|s| s.trim()) {
        if let Some(caps) = re.captures(part) {
            return Some(caps.get(1).unwrap().as_str().to_string());
        }
    }
    None
}

fn inject_selector_class(spec: &str, module: &str, class_name: &str) -> String {
    inject_selector(spec, module, Some(class_name), None)
}

fn inject_selector_method(spec: &str, module: &str, class_name: &str, method: &str) -> String {
    inject_selector(spec, module, Some(class_name), Some(method))
}

fn inject_selector(spec: &str, module: &str, class_name: Option<&str>, method: Option<&str>) -> String {
    let mut parts: Vec<String> = spec
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let mut replaced = false;
    let module_prefix = format!("/{module}");
    for p in parts.iter_mut() {
        if p.starts_with(&module_prefix) && !p.contains(':') && !p.contains('.') {
            let base = match class_name {
                Some(cls) => format!("/{module}:{cls}"),
                None => format!("/{module}"),
            };
            let full = match method {
                Some(m) => format!("{}.{}", base, m),
                None => base,
            };
            *p = full;
            replaced = true;
        }
    }

    if !replaced {
        let base = match class_name {
            Some(cls) => format!("/{module}:{cls}"),
            None => format!("/{module}"),
        };
        let full = match method {
            Some(m) => format!("{}.{}", base, m),
            None => base,
        };
        parts.push(full);
    }

    parts.join(",")
}

fn discover_test_methods(project_root: &Path, modules: &[String]) -> Result<Vec<MethodSpec>, String> {
    let custom_addons = project_root.join("custom_addons");
    let mut out = Vec::new();
    for module in modules {
        if let Some(module_root) = find_addon_root_by_name(&custom_addons, module) {
            let tests_dir = module_root.join("tests");
            if !tests_dir.exists() {
                continue;
            }
            collect_methods_from_tests_dir(module, &tests_dir, &mut out)?;
        }
    }
    Ok(out)
}

fn find_addon_root_by_name(root: &Path, addon_name: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        // If this dir is an addon, compare name
        if p.join("__manifest__.py").exists() {
            if p.file_name().and_then(|n| n.to_str()) == Some(addon_name) {
                return Some(p);
            }
            continue;
        }
        if let Some(found) = find_addon_root_by_name(&p, addon_name) {
            return Some(found);
        }
    }
    None
}

fn collect_methods_from_tests_dir(
    module: &str,
    tests_dir: &Path,
    out: &mut Vec<MethodSpec>,
) -> Result<(), String> {
    let entries = fs::read_dir(tests_dir)
        .map_err(|e| format!("Failed to read tests directory {}: {}", tests_dir.display(), e))?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_methods_from_tests_dir(module, &p, out)?;
            continue;
        }
        if p.extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }
        let content = fs::read_to_string(&p)
            .map_err(|e| format!("Failed to read {}: {}", p.display(), e))?;
        out.extend(parse_test_methods_from_file(module, &content));
    }
    Ok(())
}

fn parse_test_methods_from_file(module: &str, content: &str) -> Vec<MethodSpec> {
    // Heuristic parser: find `class X(...):` and `def test_...`
    // This is intentionally simple and avoids importing Python.
    let class_re = Regex::new(r"^class\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap();
    let def_re = Regex::new(r"^\s*def\s+(test_[A-Za-z0-9_]+)\s*\(").unwrap();

    let mut current_class: Option<String> = None;
    let mut results = Vec::new();
    for line in content.lines() {
        if let Some(caps) = class_re.captures(line) {
            current_class = Some(caps.get(1).unwrap().as_str().to_string());
            continue;
        }
        if let Some(caps) = def_re.captures(line) {
            if let Some(cls) = &current_class {
                results.push(MethodSpec {
                    module: module.to_string(),
                    class_name: cls.to_string(),
                    method_name: caps.get(1).unwrap().as_str().to_string(),
                });
            }
        }
    }
    results
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

fn resolve_log_path(
    project_root: &Path,
    db_name: &str,
    log_file: Option<&str>,
    no_log_file: bool,
) -> Result<Option<PathBuf>, String> {
    if no_log_file {
        return Ok(None);
    }
    if let Some(p) = log_file {
        let pb = PathBuf::from(p);
        if pb.is_absolute() {
            return Ok(Some(pb));
        }
        return Ok(Some(project_root.join(pb)));
    }
    Ok(Some(
        project_root
            .join(".testing/logs")
            .join(format!("odx-test-{}.log", db_name)),
    ))
}

#[derive(Debug)]
struct OutputParser {
    warnings: Vec<String>,
    ran_tests: Option<u32>,
    failed: bool,
    skipped: Option<u32>,
    failures: Option<u32>,
    errors: Option<u32>,
    warning_re: Regex,
    ran_re: Regex,
    failed_re: Regex,
}

impl OutputParser {
    fn new() -> Self {
        Self {
            warnings: Vec::new(),
            ran_tests: None,
            failed: false,
            skipped: None,
            failures: None,
            errors: None,
            warning_re: Regex::new(
                r"\b(DeprecationWarning|PendingDeprecationWarning|FutureWarning|UserWarning|RuntimeWarning|ResourceWarning|SyntaxWarning|ImportWarning)\b",
            )
            .unwrap(),
            ran_re: Regex::new(r"^Ran\s+(\d+)\s+tests?\s+in\s+").unwrap(),
            failed_re: Regex::new(r"^FAILED\s*\((.+)\)\s*$").unwrap(),
        }
    }

    fn ingest(&mut self, line: &str) {
        self.collect_warning(line);
        self.parse_unittest_summary(line);
    }

    fn collect_warning(&mut self, line: &str) {
        if self.warning_re.is_match(line) {
            if self.warnings.len() < 200 {
                self.warnings.push(line.to_string());
            }
        }
        if line.contains(" WARNING ") || line.starts_with("WARNING") {
            if self.warnings.len() < 200 {
                self.warnings.push(line.to_string());
            }
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

#[derive(Debug, Clone)]
struct TagRunResult {
    tag: String,
    passed: bool,
    ran_tests: Option<u32>,
    failures: Option<u32>,
    errors: Option<u32>,
    skipped: Option<u32>,
}

impl TagRunResult {
    fn from_parser(tag: String, passed: bool, parser: &OutputParser) -> Self {
        Self {
            tag,
            passed,
            ran_tests: parser.ran_tests,
            failures: parser.failures,
            errors: parser.errors,
            skipped: parser.skipped,
        }
    }
}

fn print_summary(runs: &[TagRunResult], warnings: &BTreeSet<String>, log_path: Option<&Path>) {
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
                r.tag, r.ran_tests, r.failures, r.errors, r.skipped
            );
        }
    }

    if !warnings.is_empty() {
        println!();
        println!("Warnings (unique, capped): {}", warnings.len());
        for w in warnings.iter().take(50) {
            println!("- {}", w);
        }
        if warnings.len() > 50 {
            println!("... ({} more)", warnings.len() - 50);
        }
    }

    if let Some(p) = log_path {
        println!();
        println!("Log file: {}", p.display());
    }

    println!("================================================");
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
                    for subentry in subdirs {
                        if let Ok(subentry) = subentry {
                            let subpath = subentry.path();
                            if subpath.is_dir() {
                                let submanifest = subpath.join("__manifest__.py");
                                if submanifest.exists() {
                                    if let Some(module_name) = subpath.file_name().and_then(|n| n.to_str()) {
                                        modules.push(module_name.to_string());
                                    }
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
    fn effective_spec_keeps_tag_only_specs() {
        assert_eq!(effective_spec_for_execution("intn"), "intn");
        assert_eq!(effective_spec_for_execution("a,b,c"), "a,b,c");
        assert_eq!(effective_spec_for_execution("   intn  "), "intn");
    }

    #[test]
    fn effective_spec_strips_tag_only_parts_when_structural_present() {
        assert_eq!(
            effective_spec_for_execution("intn,/partner_vat_unique"),
            "/partner_vat_unique"
        );
        assert_eq!(
            effective_spec_for_execution("intn,/partner_vat_unique:TestX"),
            "/partner_vat_unique:TestX"
        );
        assert_eq!(
            effective_spec_for_execution("intn,/partner_vat_unique:TestX.test_y"),
            "/partner_vat_unique:TestX.test_y"
        );
        assert_eq!(
            effective_spec_for_execution("intn, /partner_vat_unique:TestX.test_y , external"),
            "/partner_vat_unique:TestX.test_y"
        );
    }

    #[test]
    fn parse_structural_selector_module_class_method() {
        let s = parse_structural_selector("intn,/partner_vat_unique:TestX.test_y").unwrap();
        assert_eq!(
            s,
            StructuralSelector {
                module: Some("partner_vat_unique".to_string()),
                class_name: Some("TestX".to_string()),
                method_name: Some("test_y".to_string())
            }
        );
    }

    #[test]
    fn parse_structural_selector_class_method_without_module() {
        let s = parse_structural_selector(":TestX.test_y").unwrap();
        assert_eq!(
            s,
            StructuralSelector {
                module: None,
                class_name: Some("TestX".to_string()),
                method_name: Some("test_y".to_string())
            }
        );
    }

    #[test]
    fn parse_structural_selector_module_only() {
        let s = parse_structural_selector("intn,/partner_vat_unique").unwrap();
        assert_eq!(
            s,
            StructuralSelector {
                module: Some("partner_vat_unique".to_string()),
                class_name: None,
                method_name: None
            }
        );
    }
}
