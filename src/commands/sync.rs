use crate::odoo_source;
use crate::ui::Ui;
use crate::utils::{execute_command, find_project_root};

/// Sync Odoo source: pull the latest upstream commit for this project's version.
///
/// With the shared store there is one checkout per version, so this updates every
/// project on that version at once. That is usually what you want (they all track the
/// same branch) but it is worth saying out loud.
pub fn execute(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;
    let source = odoo_source::resolve(&project_root)?;

    let is_git_repo = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .current_dir(&source.path)
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git_repo {
        return Err(format!(
            "{} is not a git repository, so there is nothing to pull.",
            source.path.display()
        ));
    }

    if source.origin == odoo_source::Origin::Store {
        ui.warn(format!(
            "Updating the shared Odoo {} in {} — every project on this version sees it.",
            source.version,
            source.path.display()
        ));
    }

    let sp = ui.spinner(format!("Pulling latest Odoo {} source...", source.version));
    let result = execute_command("git", &["pull"], Some(&source.path));
    sp.finish_and_clear();
    result?;

    match odoo_source::version_from_checkout(&source.path) {
        Some(v) => ui.info(format!("Odoo version in tree: {}", v)),
        None => ui.warn("Could not read Odoo version from release files"),
    }

    ui.success("Sync complete");
    Ok(())
}
