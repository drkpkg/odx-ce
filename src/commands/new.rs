use crate::utils::{
    check_command_exists, create_project_structure, create_venv, execute_command,
    generate_from_template, resolve_python,
};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn execute(
    project_name: &str,
    version: &str,
    cd_into: bool,
    python_version: &str,
) -> Result<(), String> {
    let out = if cd_into {
        |s: &str| eprintln!("{}", s)
    } else {
        |s: &str| println!("{}", s)
    };

    validate_project_name(project_name)?;
    check_prerequisites(python_version)?;
    let project_path = std::env::current_dir()
        .map_err(|e| format!("Failed to get current directory: {}", e))?
        .join(project_name);
    fs::create_dir_all(&project_path)
        .map_err(|e| format!("Failed to create project directory: {}", e))?;
    // Resolve to absolute path so later steps run in the created project, not cwd
    let project_path = project_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve project path: {}", e))?;
    create_project_structure(&project_path)?;

    out(&format!("Cloning Odoo {}...", version));
    clone_odoo_repo(version, &project_path)?;

    out("Generating configuration files...");
    generate_config_files(&project_path, project_name, version)?;

    out("Setting up Python environment...");
    match resolve_python(python_version) {
        Ok(python_path) => {
            match create_venv(&project_path, &python_path) {
                Ok(_) => out("✓ Virtual environment created"),
                Err(e) => {
                    out(&format!("⚠  Failed to create virtual environment: {}", e));
                    out("   You can create it manually later with: python -m venv .venv");
                }
            }
        }
        Err(e) => {
            out(&format!("⚠  {}", e));
            out("   You can create the venv manually later, e.g.: pyenv install 3.11 && python -m venv .venv");
        }
    }

    out(&format!("\n{}", "=".repeat(50)));
    out(&format!("✓ Project '{}' created successfully!", project_name));
    if cd_into {
        out("\nNext steps (you are in the project dir):");
        out("  1. git init       # Initialize Git repository (optional)");
        out("  2. odx install    # Install Python dependencies");
        out("  3. odx db start   # Start PostgreSQL");
        out("  4. odx run        # Run Odoo server");
        println!("cd {}", project_path.display());
    } else {
        out("\nNext steps:");
        out(&format!("  1. cd {}", project_name));
        out("  2. git init       # Initialize Git repository (optional)");
        out("  3. odx install    # Install Python dependencies");
        out("  4. odx db start   # Start PostgreSQL");
        out("  5. odx run        # Run Odoo server");
        out("\nFor more information, see README.md");
    }

    Ok(())
}

fn validate_project_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Project name cannot be empty".to_string());
    }

    // Check for valid characters (alphanumeric, underscore, hyphen)
    let re = Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
    if !re.is_match(name) {
        return Err(
            "Project name can only contain letters, numbers, underscores, and hyphens".to_string(),
        );
    }

    // Check for reserved names
    let reserved = vec!["src", "custom_addons", "external_addons", "docs", "scripts"];
    if reserved.contains(&name.to_lowercase().as_str()) {
        return Err(format!(
            "'{}' is a reserved name. Please choose a different name.",
            name
        ));
    }

    Ok(())
}

fn check_prerequisites(python_version: &str) -> Result<(), String> {
    resolve_python(python_version)
        .map_err(|e| format!("Python requirement: {}", e))?;
    check_command_exists("git").map_err(|e| format!("Git requirement: {}", e))?;
    Ok(())
}

fn clone_odoo_repo(version: &str, project_path: &Path) -> Result<(), String> {
    execute_command(
        "git",
        &[
            "clone",
            "--branch",
            version,
            "--depth",
            "1",
            "https://github.com/odoo/odoo.git",
            "src/odoo",
        ],
        Some(project_path),
    )
}

fn generate_config_files(
    project_path: &Path,
    project_name: &str,
    version: &str,
) -> Result<(), String> {
    let mut vars = HashMap::new();
    vars.insert("project_name".to_string(), project_name.to_string());
    vars.insert("version".to_string(), version.to_string());

    // Generate compose.yml
    let compose_template = include_str!("../project_template/compose.yml.template");
    let compose_content = generate_from_template(compose_template, &vars);
    fs::write(project_path.join("compose.yml"), compose_content)
        .map_err(|e| format!("Failed to create compose.yml: {}", e))?;

    // Generate odoo.conf
    let odoo_conf_template = include_str!("../project_template/odoo.conf.template");
    let odoo_conf_content = generate_from_template(odoo_conf_template, &vars);
    fs::write(project_path.join("odoo.conf"), odoo_conf_content)
        .map_err(|e| format!("Failed to create odoo.conf: {}", e))?;

    // Generate README.md
    let readme_template = include_str!("../project_template/README.md.template");
    let readme_content = generate_from_template(readme_template, &vars);
    fs::write(project_path.join("README.md"), readme_content)
        .map_err(|e| format!("Failed to create README.md: {}", e))?;

    // Generate AGENTS.md
    let agents_template = include_str!("../project_template/AGENTS.md.template");
    let agents_content = generate_from_template(agents_template, &vars);
    fs::write(project_path.join("AGENTS.md"), agents_content)
        .map_err(|e| format!("Failed to create AGENTS.md: {}", e))?;

    Ok(())
}
