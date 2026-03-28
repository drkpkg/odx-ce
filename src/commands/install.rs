use crate::utils::{find_project_root, ensure_venv, find_python_command, execute_command};

pub fn execute() -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    let python = find_python_command()?;

    let requirements = project_root.join("src/odoo/requirements.txt");
    if !requirements.exists() {
        return Err(format!("Requirements file not found: {}", requirements.display()));
    }

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

    Ok(())
}
