use crate::utils::{
    ensure_odoo_conf_local, execute_command, find_project_root, find_python_command, ensure_venv,
};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn execute(tags: &[String]) -> Result<(), String> {
    ensure_venv()?;

    let project_root = find_project_root()?;
    ensure_odoo_conf_local(&project_root)?;

    let python = find_python_command()?;

    // Find all modules in custom_addons
    let custom_addons_path = project_root.join("custom_addons");
    let modules = find_custom_modules(&custom_addons_path)?;

    if modules.is_empty() {
        return Err("No modules found in custom_addons".to_string());
    }

    // Generate unique database name using timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let db_name = format!("test_odoo_{}", timestamp);

    println!("Creating temporary database: {}", db_name);
    println!("Found {} modules to install", modules.len());

    let odoo_bin = project_root.join("src/odoo/odoo-bin");
    if !odoo_bin.exists() {
        return Err(format!("odoo-bin not found: {}", odoo_bin.display()));
    }

    let config_file = project_root.join("odoo.conf.local");
    let odoo_bin_str = odoo_bin.to_string_lossy().to_string();
    let config_file_str = config_file.to_string_lossy().to_string();

    // Step 1: Create database and install base module
    println!("Step 1: Creating database and installing base module...");
    let args = vec![
        odoo_bin_str.as_str(),
        "-c",
        config_file_str.as_str(),
        "-d",
        db_name.as_str(),
        "--init", "base",
        "--stop-after-init",
        "--without-demo", "all",
    ];
    execute_command(&python, &args, Some(&project_root))?;

    // Step 2: Install all custom_addons modules
    println!("Step 2: Installing custom_addons modules...");
    let modules_str = modules.join(",");
    let args = vec![
        odoo_bin_str.as_str(),
        "-c",
        config_file_str.as_str(),
        "-d",
        db_name.as_str(),
        "--init", modules_str.as_str(),
        "--stop-after-init",
        "--without-demo", "all",
    ];
    execute_command(&python, &args, Some(&project_root))?;

    // Step 3: Run tests
    println!("Step 3: Running tests...");
    let tags_str = if tags.is_empty() {
        "+standard".to_string()
    } else {
        tags.join(",")
    };
    let args = vec![
        odoo_bin_str.as_str(),
        "-c",
        config_file_str.as_str(),
        "-d",
        db_name.as_str(),
        "--test-tags", tags_str.as_str(),
        "--stop-after-init",
        "--log-level=warn",
    ];
    let test_result = execute_command(&python, &args, Some(&project_root));

    // Step 4: Drop database (always, even if tests failed)
    println!("Step 4: Cleaning up temporary database...");
    let drop_args = vec![
        odoo_bin_str.as_str(),
        "-c",
        config_file_str.as_str(),
        "db", "drop",
        db_name.as_str(),
    ];

    if let Err(drop_err) = execute_command(&python, &drop_args, Some(&project_root)) {
        eprintln!("Warning: Failed to drop temporary database {}: {}", db_name, drop_err);
        eprintln!("You may need to manually drop it: odoo db drop {}", db_name);
    } else {
        println!("Temporary database {} dropped successfully", db_name);
    }

    // Return the test result
    test_result?;

    Ok(())
}

fn find_custom_modules(custom_addons_path: &std::path::Path) -> Result<Vec<String>, String> {
    if !custom_addons_path.exists() {
        return Ok(vec![]);
    }

    let mut modules = Vec::new();

    for entry in std::fs::read_dir(custom_addons_path)
        .map_err(|e| format!("Failed to read custom_addons directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();

        if path.is_dir() {
            // Look for __manifest__.py in this directory
            let manifest = path.join("__manifest__.py");
            if manifest.exists() {
                if let Some(module_name) = path.file_name().and_then(|n| n.to_str()) {
                    modules.push(module_name.to_string());
                }
            } else {
                // Check subdirectories (for namespace packages)
                if let Ok(subdirs) = std::fs::read_dir(&path) {
                    for subentry in subdirs {
                        if let Ok(subentry) = subentry {
                            let subpath = subentry.path();
                            if subpath.is_dir() {
                                let submanifest = subpath.join("__manifest__.py");
                                if submanifest.exists() {
                                    if let Some(module_name) = subpath.file_name().and_then(|n| n.to_str()) {
                                        modules.push(module_name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(modules)
}
