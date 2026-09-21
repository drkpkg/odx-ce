use crate::tui::{self, LogLevel, OdooLogLine};
use crate::ui::Ui;
use crate::utils::{
    detect_odoo_version, ensure_odoo_conf_local, ensure_venv, find_project_root,
    find_python_command, require_odoo_bin, StreamSource,
};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// `--dev=all` includes `pdb`, which drops into an interactive post-mortem debugger on
/// an unhandled exception. That needs a real stdin, which the dashboard path can't give
/// it (the terminal belongs to the TUI), so the dashboard runs the same dev features
/// minus the debugger. Use `odx run --plain` when you want the pdb prompt.
const DEV_FLAG_PLAIN: &str = "--dev=all";
const DEV_FLAG_DASHBOARD: &str = "--dev=reload,qweb,werkzeug,xml";

pub fn execute(ui: &Ui, plain: bool) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    // Also writes this addons_path into odoo.conf.local, so it's not recomputed here.
    let addons_path = ensure_odoo_conf_local(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = require_odoo_bin(&project_root)?;

    let config_file = project_root.join("odoo.conf.local");
    let odoo_bin_str = odoo_bin.to_string_lossy().to_string();
    let config_str = config_file.to_string_lossy().to_string();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let session_log = project_root
        .join(".testing")
        .join("sessions")
        .join(format!("run-{}", timestamp))
        .join("run.log");

    let use_tui = !plain && ui.config().progress && !ui.config().json && ui.is_stdout_tty();

    if use_tui {
        return run_with_dashboard(
            ui,
            &python,
            &odoo_bin_str,
            &config_str,
            &project_root,
            session_log,
        );
    }

    let args = [
        odoo_bin_str.as_str(),
        "-c",
        config_str.as_str(),
        "--addons-path",
        addons_path.as_str(),
        DEV_FLAG_PLAIN,
    ];
    run_plain(ui, &python, &args, &project_root, session_log)
}

fn title(project_root: &std::path::Path) -> String {
    let name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("odx run");
    let version = detect_odoo_version(project_root).unwrap_or_else(|_| "unknown".to_string());
    format!("odx run — {} (Odoo {})", name, version)
}

fn run_with_dashboard(
    ui: &Ui,
    python: &str,
    odoo_bin: &str,
    config_file: &str,
    project_root: &std::path::Path,
    session_log: PathBuf,
) -> Result<(), String> {
    // Called for the first start and again for every restart the user asks for with
    // 'r', so the addons path is rebuilt each time: a module added to custom_addons
    // while the server was running is picked up by the restart.
    let spawn = || {
        let addons_path = ensure_odoo_conf_local(project_root)?;
        let args = [
            odoo_bin,
            "-c",
            config_file,
            "--addons-path",
            addons_path.as_str(),
            DEV_FLAG_DASHBOARD,
        ];
        spawn_odoo(python, &args, project_root)
    };

    tui::run(spawn, session_log, title(project_root), ui)
}

fn spawn_odoo(
    python: &str,
    args: &[&str],
    project_root: &std::path::Path,
) -> Result<Child, String> {
    let mut cmd = Command::new(python);
    cmd.args(args)
        .current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Own process group so shutting down reaches Odoo's prefork workers too, not just
    // the master process (see `tui::stop_child`). Safe here because the dashboard
    // signals the child explicitly instead of relying on terminal-delivered signals.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to start {}: {}", python, e))
}

fn run_plain(
    ui: &Ui,
    python: &str,
    args: &[&str],
    project_root: &std::path::Path,
    session_log: PathBuf,
) -> Result<(), String> {
    let json = ui.config().json;
    // `--quiet` still surfaces problems: only ERROR/CRITICAL lines make it to stderr.
    let quiet = ui.config().quiet;
    let use_color = ui.use_color();

    let code = crate::utils::execute_command_streaming_status(
        python,
        args,
        Some(project_root),
        &[],
        |src, line| {
            let parsed = OdooLogLine::parse(line);
            let to_stderr = matches!(src, StreamSource::Stderr);

            if json {
                ui.json_line(&serde_json::json!({
                    "type": "log",
                    "stream": if to_stderr { "stderr" } else { "stdout" },
                    "level": level_name(parsed.level),
                    "message": parsed.raw,
                }));
                return;
            }

            if quiet && !matches!(parsed.level, LogLevel::Error | LogLevel::Critical) {
                return;
            }

            ui.passthrough(to_stderr, tui::colorize_with(use_color, &parsed));
        },
        Some(&session_log),
        None,
        None,
        "",
    )?;

    match code {
        // Terminated by a signal: Ctrl+C is the normal way to stop the server, and the
        // dashboard path reports that as success too.
        None => Ok(()),
        Some(0) => Ok(()),
        Some(code) => Err(format!("odoo-bin exited with code {}", code)),
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warning => "warning",
        LogLevel::Error => "error",
        LogLevel::Critical => "critical",
        LogLevel::Unknown => "unknown",
        LogLevel::Notice => "notice",
    }
}
