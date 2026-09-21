//! `odx store` — manage the shared Odoo checkouts that projects point at.

use crate::odoo_source;
use crate::ui::Ui;
use clap::Subcommand;
use std::fs;
use std::path::Path;

#[derive(Subcommand, Debug)]
pub enum StoreCommands {
    /// List the Odoo versions in the shared store
    Ls,
    /// Print the store path (or the path of one version)
    Path {
        /// Odoo version, e.g. 18.0. Defaults to this project's version when inside one
        version: Option<String>,
    },
    /// Fetch a version into the store (no-op when it is already there)
    Add {
        /// Odoo version, e.g. 18.0
        version: String,
    },
    /// Delete a version from the store
    Rm {
        /// Odoo version, e.g. 18.0
        version: String,
    },
}

pub fn execute(ui: &Ui, cmd: StoreCommands) -> Result<(), String> {
    match cmd {
        StoreCommands::Ls => list(ui),
        StoreCommands::Path { version } => path(ui, version),
        StoreCommands::Add { version } => {
            odoo_source::ensure_in_store(ui, &version)?;
            Ok(())
        }
        StoreCommands::Rm { version } => {
            let removed = odoo_source::remove_from_store(&version)?;
            ui.success(format!("Removed {}", removed.display()));
            Ok(())
        }
    }
}

fn list(ui: &Ui) -> Result<(), String> {
    let versions = odoo_source::list_store();
    if versions.is_empty() {
        ui.info(format!(
            "No Odoo versions in the store yet ({})",
            odoo_source::store_root().display()
        ));
        return Ok(());
    }

    ui.heading(format!(
        "Odoo store: {}",
        odoo_source::store_root().display()
    ));
    for (version, path) in versions {
        let size = dir_size(&path);
        ui.info(format!("  {:<8} {:>8}  {}", version, size, path.display()));
    }
    Ok(())
}

fn path(ui: &Ui, version: Option<String>) -> Result<(), String> {
    // Printed with `println!` rather than `ui.info`: this is a value meant to be
    // substituted into another command, so it must survive `--quiet`.
    let resolved = match version {
        Some(v) => odoo_source::store_path_for(&v),
        None => match crate::utils::find_project_root() {
            Ok(root) => odoo_source::resolve(&root)
                .map(|s| s.path)
                .unwrap_or_else(|_| odoo_source::store_root()),
            Err(_) => odoo_source::store_root(),
        },
    };
    let _ = ui;
    println!("{}", resolved.display());
    Ok(())
}

/// Rough size, good enough for `ls`. Walks the tree rather than shelling out to `du`.
fn dir_size(path: &Path) -> String {
    fn walk(path: &Path) -> u64 {
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path()),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }

    let bytes = walk(path);
    let gb = bytes as f64 / 1_073_741_824.0;
    if gb >= 1.0 {
        format!("{:.1} GB", gb)
    } else {
        format!("{:.0} MB", bytes as f64 / 1_048_576.0)
    }
}
