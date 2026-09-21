use crate::odoo_source;
use crate::ui::Ui;
use crate::utils::{
    ensure_venv, execute_command, find_project_root, find_python_command, require_odoo_bin,
};

pub fn execute(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;

    // This is what makes a freshly cloned project repo usable: it carries no Odoo
    // source, so fetch the version it pins into the shared store (a no-op when another
    // project already did).
    let version = odoo_source::project_version(&project_root)?;
    let odoo_path = odoo_source::ensure_in_store(ui, &version)?;

    ensure_venv()?;

    require_odoo_bin(&project_root)?;

    let python = find_python_command()?;

    let source = odoo_source::resolve(&project_root)?;
    ui.info(format!(
        "Odoo {} from {} ({})",
        source.version,
        source.path.display(),
        source.origin.label()
    ));
    let _ = odoo_path;

    let requirements = source.requirements();
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

    // odx always starts Odoo with a DAP listener, so debugpy belongs in the venv next
    // to Odoo's own requirements. A failure here is not fatal: the project is usable,
    // just not debuggable until it is installed.
    let _sp = ui.spinner("Installing debugger (debugpy)...");
    match execute_command(
        &python,
        &["-m", "pip", "install", "--upgrade", "debugpy"],
        Some(&project_root),
    ) {
        Ok(()) => {
            drop(_sp);
            ui.success("Python dependencies installed/updated (including debugpy)");
        }
        Err(e) => {
            drop(_sp);
            ui.warn(format!(
                "Could not install debugpy ({e}). Odoo will start without a debugger; retry with: {} -m pip install debugpy",
                python
            ));
            ui.success("Python dependencies installed/updated");
        }
    }

    Ok(())
}
