use std::fs;
use std::path::PathBuf;

use crate::utils::{
    build_addons_path, ensure_odoo_conf_local, ensure_venv, execute_command, find_project_root,
    find_python_command,
};

pub fn execute(database: Option<&str>, lang: Option<&str>) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let addons_path = build_addons_path(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = project_root.join("src/odoo/odoo-bin");
    if !odoo_bin.exists() {
        return Err(format!("odoo-bin not found: {}", odoo_bin.display()));
    }

    let db = match database {
        Some(db) => db,
        None => {
            return Err(
                "Database is required. Use: odx i18n -d <database> [--lang <code>]".to_string(),
            )
        }
    };

    let export_dir = project_root.join("i18n_export");
    fs::create_dir_all(&export_dir)
        .map_err(|e| format!("Failed to create i18n_export directory: {}", e))?;

    let (export_path, args): (PathBuf, Vec<String>) = if let Some(lang_code) = lang {
        let file = export_dir.join(format!("{}.po", lang_code));
        let args = vec![
            odoo_bin.to_string_lossy().to_string(),
            "-c".to_string(),
            project_root.join("odoo.conf.local").to_string_lossy().to_string(),
            "--addons-path".to_string(),
            addons_path,
            "-d".to_string(),
            db.to_string(),
            "--stop-after-init".to_string(),
            "--i18n-export".to_string(),
            file.to_string_lossy().to_string(),
            "--language".to_string(),
            lang_code.to_string(),
        ];
        (file, args)
    } else {
        let file = export_dir.join("template.pot");
        let args = vec![
            odoo_bin.to_string_lossy().to_string(),
            "-c".to_string(),
            project_root.join("odoo.conf.local").to_string_lossy().to_string(),
            "--addons-path".to_string(),
            addons_path,
            "-d".to_string(),
            db.to_string(),
            "--stop-after-init".to_string(),
            "--i18n-export".to_string(),
            file.to_string_lossy().to_string(),
        ];
        (file, args)
    };

    execute_command(
        &python,
        &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        Some(&project_root),
    )?;

    println!("Exported translations to {}", export_path.display());

    Ok(())
}
