use crate::ui::Ui;
use crate::utils::{
    ensure_odoo_conf_local, ensure_venv, execute_command, find_project_root, find_python_command,
    require_odoo_bin,
};

pub fn execute(_ui: &Ui, module: &str, database: &str) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = require_odoo_bin(&project_root)?;

    let config_file = project_root.join("odoo.conf.local");
    execute_command(
        &python,
        &[
            odoo_bin.to_string_lossy().as_ref(),
            "-c",
            config_file.to_string_lossy().as_ref(),
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
