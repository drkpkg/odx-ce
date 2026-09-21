//! Where Odoo's source lives, and the shared store that holds it.
//!
//! Projects do not carry their own copy of Odoo any more: a checkout is ~1.2 GB and
//! 40k files, so N projects on the same version used to cost N x 1.2 GB and N clones
//! of github.com/odoo/odoo. Instead odx keeps one checkout per version under
//! `~/.cache/odx/odoo/<version>` and points `odoo-bin`, `--addons-path` and friends at
//! it.
//!
//! Resolution is deliberately layered so nothing breaks under people's feet:
//! 1. `ODX_ODOO_PATH` — explicit override (CI, a custom checkout).
//! 2. `path` in the project's `.odx.toml`.
//! 3. `src/odoo` inside the project — projects created before the store still work.
//! 4. the store entry for the project's version.

use crate::ui::Ui;
use crate::utils::execute_command;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Project metadata file, committed with the project.
pub const PROJECT_CONFIG: &str = ".odx.toml";
const STORE_ENV: &str = "ODX_ODOO_STORE";
const PATH_ENV: &str = "ODX_ODOO_PATH";
const ODOO_REPO: &str = "https://github.com/odoo/odoo.git";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `ODX_ODOO_PATH`, or `path` pinned in `.odx.toml`.
    Override,
    /// A pre-store project that still has its own `src/odoo` checkout.
    InProject,
    /// The shared store.
    Store,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Override => "override",
            Origin::InProject => "in-project (legacy)",
            Origin::Store => "shared store",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OdooSource {
    pub path: PathBuf,
    pub version: String,
    pub origin: Origin,
}

impl OdooSource {
    pub fn odoo_bin(&self) -> PathBuf {
        self.path.join("odoo-bin")
    }

    pub fn addons_dir(&self) -> PathBuf {
        self.path.join("addons")
    }

    pub fn requirements(&self) -> PathBuf {
        self.path.join("requirements.txt")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub odoo: OdooConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OdooConfig {
    /// Odoo series this project targets, e.g. "18.0".
    #[serde(default)]
    pub version: String,
    /// Absolute path to a specific checkout, instead of the shared store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Root of the shared store. `ODX_ODOO_STORE` overrides it (used by the integration
/// tests so they never touch a developer's real store).
pub fn store_root() -> PathBuf {
    if let Ok(p) = std::env::var(STORE_ENV) {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("odx")
        .join("odoo")
}

pub fn store_path_for(version: &str) -> PathBuf {
    store_root().join(version)
}

fn is_checkout(path: &Path) -> bool {
    path.join("odoo-bin").exists()
}

pub fn read_project_config(project_root: &Path) -> Result<Option<ProjectConfig>, String> {
    let path = project_root.join(PROJECT_CONFIG);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    toml::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
}

pub fn write_project_config(project_root: &Path, version: &str) -> Result<(), String> {
    let cfg = ProjectConfig {
        odoo: OdooConfig {
            version: version.to_string(),
            path: None,
        },
    };
    let body = toml::to_string_pretty(&cfg)
        .map_err(|e| format!("Failed to serialize {}: {}", PROJECT_CONFIG, e))?;
    let contents = format!(
        "# odx project metadata. The Odoo source itself is not stored in this project:\n\
         # odx keeps one checkout per version in a shared store (see 'odx store path').\n\
         # Set odoo.path to use a specific checkout instead.\n\n{}",
        body
    );
    let path = project_root.join(PROJECT_CONFIG);
    fs::write(&path, contents).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

/// Which Odoo series this project targets, without needing the source to be present.
pub fn project_version(project_root: &Path) -> Result<String, String> {
    if let Some(cfg) = read_project_config(project_root)? {
        if !cfg.odoo.version.trim().is_empty() {
            return Ok(cfg.odoo.version);
        }
    }
    // Pre-store project: the version can still be read from its own checkout.
    let legacy = project_root.join("src/odoo");
    if is_checkout(&legacy) {
        if let Some(v) = version_from_checkout(&legacy) {
            return Ok(v);
        }
    }
    Err(format!(
        "Odoo version unknown for this project: set it in {} (e.g. [odoo] version = \"18.0\"), \
         then run 'odx install' to fetch it into the store.",
        PROJECT_CONFIG
    ))
}

/// Resolve the Odoo source for a project. See the module docs for the order.
pub fn resolve(project_root: &Path) -> Result<OdooSource, String> {
    if let Ok(p) = std::env::var(PATH_ENV) {
        if !p.trim().is_empty() {
            let path = PathBuf::from(p);
            if !is_checkout(&path) {
                return Err(format!(
                    "{} points at {}, which is not an Odoo checkout (no odoo-bin)",
                    PATH_ENV,
                    path.display()
                ));
            }
            let version = version_from_checkout(&path).unwrap_or_else(|| "unknown".to_string());
            return Ok(OdooSource {
                path,
                version,
                origin: Origin::Override,
            });
        }
    }

    let cfg = read_project_config(project_root)?;

    if let Some(pinned) = cfg.as_ref().and_then(|c| c.odoo.path.clone()) {
        let path = PathBuf::from(pinned);
        if !is_checkout(&path) {
            return Err(format!(
                "{} pins odoo.path = {}, which is not an Odoo checkout (no odoo-bin)",
                PROJECT_CONFIG,
                path.display()
            ));
        }
        let version = version_from_checkout(&path)
            .or_else(|| cfg.as_ref().map(|c| c.odoo.version.clone()))
            .unwrap_or_else(|| "unknown".to_string());
        return Ok(OdooSource {
            path,
            version,
            origin: Origin::Override,
        });
    }

    let legacy = project_root.join("src/odoo");
    if is_checkout(&legacy) {
        let version = version_from_checkout(&legacy)
            .or_else(|| cfg.as_ref().map(|c| c.odoo.version.clone()))
            .unwrap_or_else(|| "unknown".to_string());
        return Ok(OdooSource {
            path: legacy,
            version,
            origin: Origin::InProject,
        });
    }

    let version = project_version(project_root)?;
    let path = store_path_for(&version);
    if !is_checkout(&path) {
        return Err(format!(
            "Odoo {} is not in the store yet ({}). Run 'odx install' to fetch it.",
            version,
            path.display()
        ));
    }
    Ok(OdooSource {
        path,
        version,
        origin: Origin::Store,
    })
}

/// Make sure the store holds this version, cloning it once if not. Returns its path.
pub fn ensure_in_store(ui: &Ui, version: &str) -> Result<PathBuf, String> {
    let target = store_path_for(version);
    if is_checkout(&target) {
        return Ok(target);
    }

    let root = store_root();
    fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create store {}: {}", root.display(), e))?;

    // Clone into a scratch directory and rename it into place, so an interrupted or
    // failed clone (GitHub hangs up more often than you would like) can never leave a
    // half-populated version that later runs would treat as usable.
    let staging = root.join(format!(".incoming-{}-{}", version, std::process::id()));
    let _ = fs::remove_dir_all(&staging);

    let sp = ui.spinner(format!(
        "Fetching Odoo {} into the shared store (once per version)...",
        version
    ));
    let result = execute_command(
        "git",
        &[
            "clone",
            "--branch",
            version,
            "--depth",
            "1",
            ODOO_REPO,
            staging.to_string_lossy().as_ref(),
        ],
        Some(&root),
    );
    sp.finish_and_clear();

    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("Failed to fetch Odoo {}: {}", version, e));
    }

    match fs::rename(&staging, &target) {
        Ok(()) => {}
        Err(_) if is_checkout(&target) => {
            // Another odx populated it while we were cloning; theirs is as good as ours.
            let _ = fs::remove_dir_all(&staging);
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!(
                "Failed to move the fetched Odoo into {}: {}",
                target.display(),
                e
            ));
        }
    }

    ui.info(format!(
        "Odoo {} is in the store: {}",
        version,
        target.display()
    ));
    Ok(target)
}

/// Versions currently in the store, sorted.
pub fn list_store() -> Vec<(String, PathBuf)> {
    let root = store_root();
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, path)| !name.starts_with('.') && is_checkout(path))
        .collect();
    out.sort();
    out
}

pub fn remove_from_store(version: &str) -> Result<PathBuf, String> {
    let path = store_path_for(version);
    if !path.exists() {
        return Err(format!("Odoo {} is not in the store", version));
    }
    fs::remove_dir_all(&path).map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
    Ok(path)
}

/// Read the series from a checkout (release.py, then the package __init__).
pub fn version_from_checkout(path: &Path) -> Option<String> {
    let release_py = path.join("odoo/release.py");
    if let Ok(content) = fs::read_to_string(&release_py) {
        if let Some(v) = parse_version(&content) {
            return Some(v);
        }
    }
    let init_py = path.join("odoo/__init__.py");
    if let Ok(content) = fs::read_to_string(&init_py) {
        if let Some(v) = parse_version(&content) {
            return Some(v);
        }
    }
    None
}

fn parse_version(content: &str) -> Option<String> {
    let re = regex::Regex::new(r"version_info\s*=\s*\((\d+),\s*(\d+)").ok()?;
    if let Some(caps) = re.captures(content) {
        return Some(format!("{}.{}", &caps[1], &caps[2]));
    }
    let re2 = regex::Regex::new(r#"version\s*=\s*['"](\d+)\.(\d+)"#).ok()?;
    let caps = re2.captures(content)?;
    Some(format!("{}.{}", &caps[1], &caps[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "odx-source-{}-{}-{:?}",
            label,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Minimal thing that passes for an Odoo checkout.
    fn fake_checkout(path: &Path, version: &str) {
        fs::create_dir_all(path.join("odoo")).unwrap();
        fs::create_dir_all(path.join("addons")).unwrap();
        fs::write(path.join("odoo-bin"), "#!/usr/bin/env python3\n").unwrap();
        let (major, minor) = version.split_once('.').unwrap();
        fs::write(
            path.join("odoo/release.py"),
            format!("version_info = ({}, {}, 0, 'final', 0, '')\n", major, minor),
        )
        .unwrap();
    }

    #[test]
    fn project_config_round_trips() {
        let root = temp_root("config");

        write_project_config(&root, "18.0").unwrap();
        let cfg = read_project_config(&root).unwrap().unwrap();

        assert_eq!(cfg.odoo.version, "18.0");
        assert!(cfg.odoo.path.is_none());
        assert_eq!(project_version(&root).unwrap(), "18.0");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pinned_path_wins_over_the_store() {
        let root = temp_root("pinned");
        let elsewhere = root.join("custom-odoo");
        fake_checkout(&elsewhere, "17.0");

        fs::write(
            root.join(PROJECT_CONFIG),
            format!(
                "[odoo]\nversion = \"18.0\"\npath = \"{}\"\n",
                elsewhere.display()
            ),
        )
        .unwrap();

        let source = resolve(&root).unwrap();

        assert_eq!(source.origin, Origin::Override);
        assert_eq!(source.path, elsewhere);
        assert_eq!(source.version, "17.0", "version comes from the checkout");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_with_its_own_checkout_still_works() {
        // Projects created before the store keep a src/odoo; they must not break.
        let root = temp_root("legacy");
        let legacy = root.join("src/odoo");
        fake_checkout(&legacy, "17.0");

        let source = resolve(&root).unwrap();

        assert_eq!(source.origin, Origin::InProject);
        assert_eq!(source.path, legacy);
        assert_eq!(source.version, "17.0");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_store_entry_says_how_to_fix_it() {
        let root = temp_root("missing");
        write_project_config(&root, "19.0").unwrap();

        let err = resolve(&root).unwrap_err();

        assert!(err.contains("19.0"), "err was: {err}");
        assert!(err.contains("odx install"), "err was: {err}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn version_is_read_from_a_checkout() {
        let root = temp_root("version");
        let checkout = root.join("odoo");
        fake_checkout(&checkout, "18.0");

        assert_eq!(version_from_checkout(&checkout).as_deref(), Some("18.0"));
        assert_eq!(version_from_checkout(&root), None);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn store_layout_is_one_directory_per_version() {
        let root = store_path_for("18.0");
        assert!(root.ends_with("18.0"));
        assert_eq!(root.parent().unwrap(), store_root());
    }
}
