use crate::utils::{
    build_addons_path, ensure_odoo_conf_local, ensure_venv, execute_command, find_project_root,
    find_python_command,
};

pub fn execute() -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let addons_path = build_addons_path(&project_root)?;

    let python = find_python_command()?;

    let odoo_bin = project_root.join("src/odoo/odoo-bin");
    if !odoo_bin.exists() {
        return Err(format!("odoo-bin not found: {}", odoo_bin.display()));
    }

    let config_file = project_root.join("odoo.conf.local");
    let odoo_bin_str = odoo_bin.to_string_lossy();
    let config_str = config_file.to_string_lossy();
    execute_command(
        &python,
        &[
            odoo_bin_str.as_ref(),
            "-c",
            config_str.as_ref(),
            "--addons-path",
            addons_path.as_str(),
            "--dev=all",
        ],
        Some(&project_root),
    )?;

    Ok(())
}
