use crate::ui::Ui;
use crate::utils::{
    build_addons_path, ensure_odoo_conf_local, ensure_venv, execute_command, find_project_root,
    find_python_command, require_odoo_bin, validate_db_name,
};

pub fn execute(_ui: &Ui, module: &str, database: &str) -> Result<(), String> {
    validate_db_name(database)?;
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let addons_path = build_addons_path(&project_root)?;

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
            module,
            "--no-http",
            "--stop-after-init",
        ],
        Some(&project_root),
    )?;

    Ok(())
}
