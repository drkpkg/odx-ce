use crate::ui::Ui;
use crate::utils::{
    ensure_venv, execute_command, find_project_root, find_python_command, require_odoo_bin,
};

pub fn execute(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;

    let gitmodules = project_root.join(".gitmodules");
    if gitmodules.exists() {
        let _sp = ui.spinner("Syncing git submodules...");
        execute_command(
            "git",
            &["submodule", "sync", "--recursive"],
            Some(&project_root),
        )?;

        let _sp = ui.spinner("Initializing git submodules at pinned commits...");
        execute_command(
            "git",
            &["submodule", "update", "--init", "--recursive"],
            Some(&project_root),
        )?;
    }

    ensure_venv()?;

    require_odoo_bin(&project_root)?;

    let python = find_python_command()?;

    let requirements = project_root.join("src/odoo/requirements.txt");
    if !requirements.exists() {
        return Err(format!(
            "Requirements file not found: {}",
            requirements.display()
        ));
    }

    let _sp = ui.spinner("Installing Python dependencies (pip)...");
    execute_command(
        &python,
        &[
            "-m",
            "pip",
            "install",
            "--upgrade",
            "-r",
            requirements.to_string_lossy().as_ref(),
        ],
        Some(&project_root),
    )?;

    ui.success("Python dependencies installed/updated");
    Ok(())
}
