use crate::utils::{detect_odoo_version, execute_command, find_project_root};

/// Sync Odoo source: pull latest from upstream.
pub fn execute() -> Result<(), String> {
    let project_root = find_project_root()?;
    let odoo_path = project_root.join("src/odoo");

    if !odoo_path.exists() {
        return Err("Odoo directory not found. Run 'odx new' first.".to_string());
    }

    let is_git_repo = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .current_dir(&odoo_path)
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git_repo {
        return Err("src/odoo is not a git repository. Sync is only supported when Odoo was cloned with 'odx new'.".to_string());
    }

    println!("Pulling latest Odoo source...");
    execute_command("git", &["pull"], Some(&odoo_path))?;

    match detect_odoo_version(&project_root) {
        Ok(v) => println!("Odoo version in tree: {}", v),
        Err(_) => println!("(Could not read Odoo version from release files)"),
    }

    println!("Sync complete.");
    Ok(())
}
