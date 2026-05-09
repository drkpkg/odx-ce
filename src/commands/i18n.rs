use std::fs;
use std::path::{Path, PathBuf};

use crate::utils::{
    build_addons_path, ensure_odoo_conf_local, ensure_venv, execute_command, find_project_root,
    find_python_command, project_addon_modules,
};

const TEMPLATE_LANG: &str = "en_US";

pub fn execute(database: &str, module: Option<&str>, lang: Option<&str>) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let addons_path = build_addons_path(&project_root)?;

    let python = find_python_command()?;
    let odoo_bin = project_root.join("src/odoo/odoo-bin");
    if !odoo_bin.exists() {
        return Err(format!("odoo-bin not found: {}", odoo_bin.display()));
    }

    let targets = resolve_targets(&project_root, module)?;

    let config_str = project_root.join("odoo.conf.local").to_string_lossy().to_string();
    let odoo_bin_str = odoo_bin.to_string_lossy().to_string();

    for (mod_name, mod_path) in targets {
        let i18n_dir = mod_path.join("i18n");
        fs::create_dir_all(&i18n_dir).map_err(|e| {
            format!(
                "Failed to create {}: {}",
                i18n_dir.display(),
                e
            )
        })?;

        let (file_name, lang_for_odoo): (String, &str) = match lang {
            Some(code) => (format!("{}.po", code), code),
            None => (format!("{}.pot", mod_name), TEMPLATE_LANG),
        };

        let export_path = i18n_dir.join(&file_name);

        let i18n_abs = i18n_dir.canonicalize().map_err(|e| {
            format!(
                "Failed to canonicalize {}: {}",
                i18n_dir.display(),
                e
            )
        })?;
        let export_abs = i18n_abs.join(&file_name);
        let export_abs_str = export_abs.to_string_lossy().into_owned();

        let args = vec![
            odoo_bin_str.as_str(),
            "-c",
            config_str.as_str(),
            "--addons-path",
            addons_path.as_str(),
            "-d",
            database,
            "--stop-after-init",
            "--i18n-export",
            export_abs_str.as_str(),
            "--modules",
            mod_name.as_str(),
            "--language",
            lang_for_odoo,
        ];

        execute_command(&python, &args, Some(&project_root))?;

        println!("Exported translations to {}", export_path.display());
    }

    Ok(())
}

fn resolve_targets(project_root: &Path, module: Option<&str>) -> Result<Vec<(String, PathBuf)>, String> {
    let pairs = project_addon_modules(project_root)?;
    match module {
        None => {
            if pairs.is_empty() {
                return Err(
                    "No addons found under custom_addons or external_addons.".to_string(),
                );
            }
            Ok(pairs)
        }
        Some(name) => {
            for (n, p) in &pairs {
                if n == name {
                    return Ok(vec![(n.clone(), p.clone())]);
                }
            }
            Err(format!(
                "Unknown module {:?}. It must exist under custom_addons or external_addons with __manifest__.py",
                name
            ))
        }
    }
}
