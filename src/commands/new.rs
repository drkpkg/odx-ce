use crate::debug;
use crate::odoo_source;
use crate::ui::Ui;
use crate::utils::{
    check_command_exists, create_project_structure, create_venv, generate_from_template,
    resolve_python,
};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn execute(
    ui: &Ui,
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

    // The Odoo source is not copied into the project: it is fetched once per version
    // into the shared store, so a second project on the same version costs nothing and
    // needs no network.
    let odoo_path = odoo_source::ensure_in_store(ui, version)?;
    out(&format!("Odoo {}: {}", version, odoo_path.display()));

    out("Generating configuration files...");
    odoo_source::write_project_config(&project_path, version)?;
    generate_config_files(&project_path, project_name, version)?;
    generate_agent_access_config(&project_path, &odoo_path)?;

    out("Setting up Python environment...");
    match resolve_python(python_version) {
        Ok(python_path) => match create_venv(&project_path, &python_path) {
            Ok(_) => out("✓ Virtual environment created"),
            Err(e) => {
                out(&format!("⚠  Failed to create virtual environment: {}", e));
                out("   You can create it manually later with: python -m venv .venv");
            }
        },
        Err(e) => {
            out(&format!("⚠  {}", e));
            out("   You can create the venv manually later, e.g.: pyenv install 3.11 && python -m venv .venv");
        }
    }

    out(&format!("\n{}", "=".repeat(50)));
    out(&format!(
        "✓ Project '{}' created successfully!",
        project_name
    ));
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
    let reserved = ["src", "custom_addons", "external_addons", "docs", "scripts"];
    if reserved.contains(&name.to_lowercase().as_str()) {
        return Err(format!(
            "'{}' is a reserved name. Please choose a different name.",
            name
        ));
    }

    Ok(())
}

fn check_prerequisites(python_version: &str) -> Result<(), String> {
    resolve_python(python_version).map_err(|e| format!("Python requirement: {}", e))?;
    check_command_exists("git").map_err(|e| format!("Git requirement: {}", e))?;
    Ok(())
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

    // Generate CLAUDE.md. It imports AGENTS.md instead of repeating it: Claude Code
    // reads CLAUDE.md, other agents read AGENTS.md, and there is only one file to keep
    // current.
    let claude_template = include_str!("../project_template/CLAUDE.md.template");
    let claude_content = generate_from_template(claude_template, &vars);
    fs::write(project_path.join("CLAUDE.md"), claude_content)
        .map_err(|e| format!("Failed to create CLAUDE.md: {}", e))?;

    generate_debug_config(project_path)?;

    // Worth having now that the project is small enough to commit: Odoo's source is no
    // longer sitting inside it as a 1.2 GB nested repository.
    let gitignore = project_path.join(".gitignore");
    if !gitignore.exists() {
        fs::write(
            &gitignore,
            include_str!("../project_template/gitignore.template"),
        )
        .map_err(|e| format!("Failed to create .gitignore: {}", e))?;
    }

    Ok(())
}

/// Grant agents read access to the Odoo source. It lives outside the project now, and
/// agent tools refuse paths outside the working directory, so a project that does not
/// declare it leaves the agent unable to read the framework it is writing against.
fn generate_agent_access_config(project_path: &Path, odoo_path: &Path) -> Result<(), String> {
    let claude_dir = project_path.join(".claude");
    let settings = claude_dir.join("settings.json");
    if settings.exists() {
        return Ok(());
    }

    fs::create_dir_all(&claude_dir)
        .map_err(|e| format!("Failed to create .claude directory: {}", e))?;

    let body = format!(
        r#"{{
  "permissions": {{
    "additionalDirectories": [
      "{}"
    ]
  }}
}}
"#,
        odoo_path.to_string_lossy().replace('\\', "\\\\")
    );
    fs::write(&settings, body).map_err(|e| format!("Failed to create .claude/settings.json: {}", e))
}

/// VS Code / Cursor attach configuration, so `odx run` + F5 debugs out of the box.
/// Never overwrites an existing launch.json.
fn generate_debug_config(project_path: &Path) -> Result<(), String> {
    let vscode_dir = project_path.join(".vscode");
    let launch_json = vscode_dir.join("launch.json");
    if launch_json.exists() {
        return Ok(());
    }

    fs::create_dir_all(&vscode_dir)
        .map_err(|e| format!("Failed to create .vscode directory: {}", e))?;

    let setup = debug::DebugSetup {
        host: debug::HOST.to_string(),
        port: debug::DEFAULT_PORT,
        wait: false,
    };
    fs::write(&launch_json, debug::launch_json(&setup))
        .map_err(|e| format!("Failed to create .vscode/launch.json: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "odx-new-{}-{}-{:?}",
            label,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generated_project_briefs_agents_from_a_single_file() {
        let dir = temp_dir("templates");

        generate_config_files(&dir, "my_project", "18.0").unwrap();

        for name in [
            "compose.yml",
            "odoo.conf",
            "README.md",
            "AGENTS.md",
            "CLAUDE.md",
            ".gitignore",
        ] {
            assert!(dir.join(name).exists(), "{name} must be generated");
        }

        let claude = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(
            claude.contains("@AGENTS.md"),
            "CLAUDE.md must import the briefing instead of duplicating it"
        );

        let agents = fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        assert!(agents.contains("my_project") && agents.contains("18.0"));
        assert!(
            agents.contains("rg \"_compute_display_name\" custom_addons/"),
            "agents need the search guidance for this project's own code"
        );
        assert!(
            agents.contains("odx store path") && agents.contains("additionalDirectories"),
            "agents need to know Odoo core is outside the project, and how to reach it"
        );

        // `generate_from_template` replaces `{{var}}`; anything left over means a
        // template used the wrong number of braces and ships a literal placeholder.
        for name in ["README.md", "AGENTS.md", "CLAUDE.md"] {
            let rendered = fs::read_to_string(dir.join(name)).unwrap();
            assert!(
                !rendered.contains("{{") && !rendered.contains("{my_project}"),
                "{name} still contains an unrendered placeholder"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_project_gets_an_attach_config() {
        let dir = temp_dir("launch");

        generate_debug_config(&dir).unwrap();

        let launch = fs::read_to_string(dir.join(".vscode/launch.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&launch).unwrap();
        assert_eq!(parsed["configurations"][0]["request"], "attach");
        assert_eq!(
            parsed["configurations"][0]["connect"]["port"],
            debug::DEFAULT_PORT
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_launch_config_is_never_overwritten() {
        let dir = temp_dir("keep");
        fs::create_dir_all(dir.join(".vscode")).unwrap();
        fs::write(dir.join(".vscode/launch.json"), "{ \"mine\": true }").unwrap();

        generate_debug_config(&dir).unwrap();

        assert_eq!(
            fs::read_to_string(dir.join(".vscode/launch.json")).unwrap(),
            "{ \"mine\": true }"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
