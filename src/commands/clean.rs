use crate::ui::Ui;
use crate::utils::find_project_root;
use std::fs;
use std::path::Path;

pub fn execute(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;

    ui.heading("Cleaning temporary files...");

    remove_dir_all_matches(&project_root, "__pycache__")?;
    remove_file_matches(&project_root, "*.pyc")?;
    remove_file_matches(&project_root, "*.pyo")?;
    remove_file_matches(&project_root, "*.log")?;

    ui.success("Clean completed.");
    Ok(())
}

fn remove_dir_all_matches(root: &Path, pattern: &str) -> Result<(), String> {
    let mut callback = |path: &Path| {
        if path.file_name().and_then(|n| n.to_str()) == Some(pattern) && path.is_dir() {
            fs::remove_dir_all(path).ok();
        }
    };
    walk_dir(root, &mut callback);
    Ok(())
}

fn remove_file_matches(root: &Path, pattern: &str) -> Result<(), String> {
    let Some(ext) = pattern.strip_prefix("*.") else {
        return Ok(());
    };

    let mut callback = |path: &Path| {
        if path.is_file() {
            if let Some(path_ext) = path.extension().and_then(|e| e.to_str()) {
                if path_ext == ext {
                    fs::remove_file(path).ok();
                }
            }
        }
    };
    walk_dir(root, &mut callback);
    Ok(())
}

fn walk_dir(dir: &Path, f: &mut impl FnMut(&Path)) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            f(&path);
            if path.is_dir() {
                walk_dir(&path, f);
            }
        }
    }
}
