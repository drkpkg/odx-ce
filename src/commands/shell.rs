use crate::debug;
use crate::ui::Ui;
use crate::utils::{
    ensure_odoo_conf_local, ensure_venv, execute_command_with_env, find_project_root,
    find_python_command, require_odoo_bin, validate_db_name,
};

pub fn execute(ui: &Ui, database: &str, debug_port: Option<u16>) -> Result<(), String> {
    validate_db_name(database)?;
    ensure_venv()?;

    let project_root = find_project_root()?;
    // Also writes this addons_path into odoo.conf.local, so it's not recomputed here.
    let addons_path = ensure_odoo_conf_local(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = require_odoo_bin(&project_root)?;

    // The shell is interactive on this terminal, but code called from it is just as
    // debuggable as the server: same listener, same attach config.
    let (debug_setup, shim_dir) = debug::prepare(&project_root, debug_port, false)?;
    if !debug::is_available(&python) {
        ui.warn(
            "debugpy is not installed in .venv; starting without a debugger (run 'odx install')",
        );
    }
    let debug_env = debug_setup.env(&shim_dir, &odoo_bin);
    let envs: Vec<(&str, &str)> = debug_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let config_file = project_root.join("odoo.conf.local");
    let odoo_bin_str = odoo_bin.to_string_lossy();
    let config_str = config_file.to_string_lossy();
    execute_command_with_env(
        &python,
        &[
            odoo_bin_str.as_ref(),
            "shell",
            "-c",
            config_str.as_ref(),
            "--addons-path",
            addons_path.as_str(),
            "-d",
            database,
        ],
        Some(&project_root),
        &envs,
    )?;

    Ok(())
}
