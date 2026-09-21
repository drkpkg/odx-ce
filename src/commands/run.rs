use crate::debug;
use crate::tui::{self, LogLevel, OdooLogLine};
use crate::ui::Ui;
use crate::utils::{
    detect_odoo_version, ensure_odoo_conf_local, ensure_venv, find_project_root,
    find_python_command, require_odoo_bin, StreamSource,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long to wait for the debug port to come free before a (re)start. debugpy's
/// adapter releases it a moment after the previous Odoo process is gone.
const DEBUG_PORT_GRACE: Duration = Duration::from_secs(3);

/// Odoo's developer features. `all` means `reload,qweb,xml` on 17.0/18.0 and
/// `access,qweb,reload,xml` on 19.0 — none of which need an interactive stdin, so the
/// dashboard and the plain path can run exactly the same thing. Debugging is not part
/// of this flag on any supported version: it goes over DAP (see `crate::debug`).
const DEV_FLAG: &str = "--dev=all";

pub fn execute(
    ui: &Ui,
    plain: bool,
    debug_port: Option<u16>,
    debug_wait: bool,
) -> Result<(), String> {
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

    let (debug_setup, shim_dir) = debug::prepare(&project_root, debug_port, debug_wait)?;
    if !debug::is_available(&python) {
        ui.warn(format!(
            "debugpy is not installed in .venv; starting without a debugger (run 'odx install'). Expected listener: {}",
            debug_setup.address()
        ));
    }
    let debug_env = debug_setup.env(&shim_dir, &odoo_bin);

    let use_tui = !plain && ui.config().progress && !ui.config().json && ui.is_stdout_tty();

    if use_tui {
        return run_with_dashboard(
            ui,
            &python,
            &odoo_bin_str,
            &config_str,
            &project_root,
            session_log,
            &debug_setup,
            &debug_env,
        );
    }

    let args = [
        odoo_bin_str.as_str(),
        "-c",
        config_str.as_str(),
        "--addons-path",
        addons_path.as_str(),
        DEV_FLAG,
    ];
    debug::wait_for_port_free(debug_setup.port, DEBUG_PORT_GRACE);
    run_plain(ui, &python, &args, &project_root, session_log, &debug_env)
}

fn title(project_root: &Path, debug: &debug::DebugSetup) -> String {
    let name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("odx run");
    let version = detect_odoo_version(project_root).unwrap_or_else(|_| "unknown".to_string());
    format!(
        "odx run — {} (Odoo {}) · dap {}",
        name,
        version,
        debug.address()
    )
}

#[allow(clippy::too_many_arguments)]
fn run_with_dashboard(
    ui: &Ui,
    python: &str,
    odoo_bin: &str,
    config_file: &str,
    project_root: &Path,
    session_log: PathBuf,
    debug_setup: &debug::DebugSetup,
    debug_env: &[(String, String)],
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
            DEV_FLAG,
        ];
        // The previous process's debug adapter may still hold the port for a moment;
        // the shim only gets one window to bind it.
        debug::wait_for_port_free(debug_setup.port, DEBUG_PORT_GRACE);
        spawn_odoo(python, &args, project_root, debug_env)
    };

    tui::run(spawn, session_log, title(project_root, debug_setup), ui)
}

fn spawn_odoo(
    python: &str,
    args: &[&str],
    project_root: &Path,
    envs: &[(String, String)],
) -> Result<Child, String> {
    let mut cmd = Command::new(python);
    cmd.args(args)
        .current_dir(project_root)
        .envs(envs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
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
    project_root: &Path,
    session_log: PathBuf,
    debug_env: &[(String, String)],
) -> Result<(), String> {
    let json = ui.config().json;
    // `--quiet` still surfaces problems: only ERROR/CRITICAL lines make it to stderr.
    let quiet = ui.config().quiet;
    let use_color = ui.use_color();

    let envs: Vec<(&str, &str)> = debug_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let code = crate::utils::execute_command_streaming_status(
        python,
        args,
        Some(project_root),
        &envs,
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
