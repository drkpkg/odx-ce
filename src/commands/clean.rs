use crate::ui::Ui;
use crate::utils::find_project_root;
use std::fs;
use std::path::Path;

/// Directories that must never be descended into by `odx clean`:
/// `.git` (internal objects, not project junk), `.venv` (regenerable but
/// pointless/slow to sweep), `.testing` (holds `odx test` session artifacts
/// like combined.log/warnings.log that the `*.log` pattern would otherwise
/// delete).
const SKIP_DIRS: &[&str] = &[".git", ".venv", ".testing"];

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
            let is_skipped_dir = path.is_dir()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| SKIP_DIRS.contains(&n));
            if is_skipped_dir {
                continue;
            }
            f(&path);
            if path.is_dir() {
                walk_dir(&path, f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_removes_pycache_and_logs_outside_skip_dirs() {
        let tmp = std::env::temp_dir().join(format!("odx-clean-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);

        fs::create_dir_all(tmp.join("custom_addons/my_module/__pycache__")).unwrap();
        fs::write(
            tmp.join("custom_addons/my_module/__pycache__/mod.pyc"),
            "",
        )
        .unwrap();
        fs::write(tmp.join("custom_addons/my_module/stray.log"), "").unwrap();

        remove_dir_all_matches(&tmp, "__pycache__").unwrap();
        remove_file_matches(&tmp, "*.log").unwrap();

        assert!(!tmp.join("custom_addons/my_module/__pycache__").exists());
        assert!(!tmp.join("custom_addons/my_module/stray.log").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clean_preserves_test_session_logs_and_skip_dirs() {
        let tmp = std::env::temp_dir().join(format!("odx-clean-skip-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);

        fs::create_dir_all(tmp.join(".testing/sessions/123")).unwrap();
        fs::write(tmp.join(".testing/sessions/123/combined.log"), "").unwrap();
        fs::create_dir_all(tmp.join(".git")).unwrap();
        fs::write(tmp.join(".git/some.log"), "").unwrap();
        fs::create_dir_all(tmp.join(".venv/lib/__pycache__")).unwrap();
        fs::write(tmp.join(".venv/lib/__pycache__/x.pyc"), "").unwrap();

        remove_dir_all_matches(&tmp, "__pycache__").unwrap();
        remove_file_matches(&tmp, "*.pyc").unwrap();
        remove_file_matches(&tmp, "*.log").unwrap();

        assert!(tmp.join(".testing/sessions/123/combined.log").exists());
        assert!(tmp.join(".git/some.log").exists());
        assert!(tmp.join(".venv/lib/__pycache__/x.pyc").exists());

        let _ = fs::remove_dir_all(&tmp);
    }
}
