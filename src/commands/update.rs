use crate::utils::{
    ensure_odoo_conf_local, execute_command, find_project_root, find_python_command, ensure_venv,
};

pub fn execute(database: &str) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = project_root.join("src/odoo/odoo-bin");
    if !odoo_bin.exists() {
        return Err(format!("odoo-bin not found: {}", odoo_bin.display()));
    }

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
            "all",
            "--no-http",
            "--stop-after-init",
            "--log-level=warn",
        ],
        Some(&project_root),
    )?;

    Ok(())
}
