use crate::ui::Ui;
use crate::utils::find_project_root;
use std::fs;
use std::path::{Path, PathBuf};

/// Directories that must never be descended into by `odx clean`:
/// `.git` (internal objects, not project junk), `.venv` (regenerable but
/// pointless/slow to sweep), `.testing` (holds `odx test`/`odx run` session artifacts
/// like combined.log/run.log that the `*.log` pattern would otherwise delete —
/// `prune_run_sessions` handles that directory instead).
const SKIP_DIRS: &[&str] = &[".git", ".venv", ".testing"];

/// Directory names removed whole, with their contents.
const JUNK_DIRS: &[&str] = &["__pycache__"];

/// File extensions removed individually.
const JUNK_EXTENSIONS: &[&str] = &["pyc", "pyo", "log"];

/// How many `odx run` log sessions (`.testing/sessions/run-<timestamp>/`) to keep.
/// `odx run` writes one per invocation and nothing else prunes them.
const KEEP_RUN_SESSIONS: usize = 5;

#[derive(Debug, Default, PartialEq, Eq)]
struct CleanStats {
    dirs: usize,
    files: usize,
}

pub fn execute(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;

    ui.heading("Cleaning temporary files...");

    let stats = sweep(&project_root);
    ui.info(format!(
        "Removed {} directories and {} files",
        stats.dirs, stats.files
    ));

    let pruned = prune_run_sessions(&project_root, KEEP_RUN_SESSIONS);
    if pruned > 0 {
        ui.info(format!(
            "Pruned {} old run log session(s) (kept the {} most recent)",
            pruned, KEEP_RUN_SESSIONS
        ));
    }

    ui.success("Clean completed.");
    Ok(())
}

/// One traversal for every pattern. Doing a pass per pattern meant walking the whole
/// Odoo checkout (~40k files) four times over.
fn sweep(root: &Path) -> CleanStats {
    let mut stats = CleanStats::default();
    sweep_dir(root, &mut stats);
    stats
}

fn sweep_dir(dir: &Path, stats: &mut CleanStats) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        // `DirEntry::file_type` does not follow symlinks, unlike `Path::is_dir`: a
        // symlink under external_addons/ pointing at one of its own ancestors would
        // otherwise make this recurse until the stack overflows.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            if JUNK_DIRS.contains(&name.as_ref()) {
                if fs::remove_dir_all(&path).is_ok() {
                    stats.dirs += 1;
                }
                continue;
            }
            sweep_dir(&path, stats);
            continue;
        }

        if file_type.is_file() && has_junk_extension(&path) && fs::remove_file(&path).is_ok() {
            stats.files += 1;
        }
    }
}

fn has_junk_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| JUNK_EXTENSIONS.contains(&ext))
}

/// Delete all but the `keep` newest `.testing/sessions/run-*` directories. Only the
/// `run-` prefix is touched: `odx test` sessions are named with a bare timestamp and
/// are the artifacts users come back to after a failing run.
fn prune_run_sessions(project_root: &Path, keep: usize) -> usize {
    let sessions = project_root.join(".testing").join("sessions");
    let Ok(entries) = fs::read_dir(&sessions) else {
        return 0;
    };

    let mut run_dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("run-"))
        })
        .collect();

    if run_dirs.len() <= keep {
        return 0;
    }

    // Names are run-<unix timestamp>, so sorting by name sorts by age.
    run_dirs.sort();
    let stale = run_dirs.len() - keep;
    run_dirs
        .into_iter()
        .take(stale)
        .filter(|p| fs::remove_dir_all(p).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "odx-clean-{}-{}-{:?}",
            label,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn clean_removes_pycache_and_logs_outside_skip_dirs() {
        let tmp = temp_dir("basic");

        fs::create_dir_all(tmp.join("custom_addons/my_module/__pycache__")).unwrap();
        fs::write(tmp.join("custom_addons/my_module/__pycache__/mod.pyc"), "").unwrap();
        fs::write(tmp.join("custom_addons/my_module/stray.log"), "").unwrap();
        fs::write(tmp.join("custom_addons/my_module/models.py"), "").unwrap();

        let stats = sweep(&tmp);

        assert!(!tmp.join("custom_addons/my_module/__pycache__").exists());
        assert!(!tmp.join("custom_addons/my_module/stray.log").exists());
        assert!(tmp.join("custom_addons/my_module/models.py").exists());
        assert_eq!(stats, CleanStats { dirs: 1, files: 1 });

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clean_preserves_test_session_logs_and_skip_dirs() {
        let tmp = temp_dir("skip");

        fs::create_dir_all(tmp.join(".testing/sessions/123")).unwrap();
        fs::write(tmp.join(".testing/sessions/123/combined.log"), "").unwrap();
        fs::create_dir_all(tmp.join(".git")).unwrap();
        fs::write(tmp.join(".git/some.log"), "").unwrap();
        fs::create_dir_all(tmp.join(".venv/lib/__pycache__")).unwrap();
        fs::write(tmp.join(".venv/lib/__pycache__/x.pyc"), "").unwrap();

        sweep(&tmp);

        assert!(tmp.join(".testing/sessions/123/combined.log").exists());
        assert!(tmp.join(".git/some.log").exists());
        assert!(tmp.join(".venv/lib/__pycache__/x.pyc").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn clean_does_not_follow_symlinks_into_a_cycle() {
        let tmp = temp_dir("symlink");

        fs::create_dir_all(tmp.join("external_addons/shared")).unwrap();
        fs::write(tmp.join("external_addons/shared/stray.log"), "").unwrap();
        // A link back to an ancestor: following it would recurse until the stack blows.
        std::os::unix::fs::symlink(&tmp, tmp.join("external_addons/shared/loop")).unwrap();

        let stats = sweep(&tmp);

        assert_eq!(stats.files, 1);
        assert!(tmp.join("external_addons/shared/loop").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prune_keeps_newest_run_sessions_and_leaves_test_sessions_alone() {
        let tmp = temp_dir("prune");

        let sessions = tmp.join(".testing/sessions");
        for ts in ["run-100", "run-200", "run-300", "1700000000"] {
            fs::create_dir_all(sessions.join(ts)).unwrap();
        }

        let pruned = prune_run_sessions(&tmp, 2);

        assert_eq!(pruned, 1);
        assert!(!sessions.join("run-100").exists());
        assert!(sessions.join("run-200").exists());
        assert!(sessions.join("run-300").exists());
        assert!(
            sessions.join("1700000000").exists(),
            "odx test sessions must not be pruned"
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
