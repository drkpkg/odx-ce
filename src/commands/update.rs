use crate::ui::Ui;
use crate::utils::{
    ensure_odoo_conf_local, ensure_venv, execute_command, find_project_root, find_python_command,
    require_odoo_bin, validate_db_name,
};

pub fn execute(_ui: &Ui, database: &str) -> Result<(), String> {
    validate_db_name(database)?;
    ensure_venv()?;

    let project_root = find_project_root()?;
    // Also writes this addons_path into odoo.conf.local, so it's not recomputed here.
    let addons_path = ensure_odoo_conf_local(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = require_odoo_bin(&project_root)?;

    let config_file = project_root.join("odoo.conf.local");
    execute_command(
        &python,
        &[
            odoo_bin.to_string_lossy().as_ref(),
            "-c",
            config_file.to_string_lossy().as_ref(),
            "--addons-path",
            addons_path.as_str(),
            "-d",
            database,
            "-u",
            "all",
            "--no-http",
            "--stop-after-init",
            "--log-level=warn",
        ],
        Some(&project_root),
    )?;

    Ok(())
}
