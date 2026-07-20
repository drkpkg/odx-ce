use crate::tui::{self, OdooLogLine};
use crate::ui::Ui;
use crate::utils::{
    build_addons_path, detect_odoo_version, ensure_odoo_conf_local, ensure_venv, find_project_root,
    find_python_command, require_odoo_bin, StreamSource,
};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn execute(ui: &Ui, plain: bool) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let addons_path = build_addons_path(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = require_odoo_bin(&project_root)?;

    let config_file = project_root.join("odoo.conf.local");
    let odoo_bin_str = odoo_bin.to_string_lossy().to_string();
    let config_str = config_file.to_string_lossy().to_string();
    let args = [
        odoo_bin_str.as_str(),
        "-c",
        config_str.as_str(),
        "--addons-path",
        addons_path.as_str(),
        "--dev=all",
    ];

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
        run_with_dashboard(ui, &python, &args, &project_root, session_log)
    } else {
        run_plain(ui, &python, &args, &project_root, session_log)
    }
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
    args: &[&str],
    project_root: &std::path::Path,
    session_log: PathBuf,
) -> Result<(), String> {
    let child = Command::new(python)
        .args(args)
        .current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start {}: {}", python, e))?;

    tui::run(child, session_log, title(project_root), ui)
}

fn run_plain(
    ui: &Ui,
    python: &str,
    args: &[&str],
    project_root: &std::path::Path,
    session_log: PathBuf,
) -> Result<(), String> {
    crate::utils::execute_command_streaming_with_env(
        python,
        args,
        Some(project_root),
        &[],
        |src, line| {
            let colored = tui::colorize(ui, &OdooLogLine::parse(line));
            match src {
                StreamSource::Stdout | StreamSource::LogFile => println!("{}", colored),
                StreamSource::Stderr => eprintln!("{}", colored),
            }
        },
        Some(&session_log),
        None,
        None,
        "",
    )?;

    Ok(())
}
