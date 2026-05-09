use crate::utils::{
    check_command_exists, check_python_version, check_system_package, detect_odoo_version,
    detect_os, find_project_root, get_command_version,
};
use crate::ui::Ui;
use std::fs;
use std::path::Path;

pub fn execute(ui: &Ui) -> Result<(), String> {
    let os = detect_os();

    ui.heading("Odoo Framework - System Requirements Check");
    ui.info("===========================================");
    ui.info("");
    ui.info(format!("Operating System: {}", format_os_name(os)));
    ui.info("");

    let mut all_ok = true;

    ui.heading("Common Dependencies:");
    ui.info("--------------------");

    all_ok &= check_python(ui)?;
    all_ok &= check_git(ui)?;
    all_ok &= check_docker(ui)?;

    ui.info("");

    ui.heading(format!("System Dependencies ({})", format_os_name(os)));
    ui.info("-".repeat(30));
    match os {
        "linux" => all_ok &= check_linux_dependencies(ui)?,
        "windows" => all_ok &= check_windows_dependencies(ui)?,
        "macos" => all_ok &= check_macos_dependencies(ui)?,
        _ => {
            ui.warn("OS-specific checks not available for this platform");
        }
    }

    ui.info("");

    if let Ok(project_root) = find_project_root() {
        ui.heading("Project Python Dependencies:");
        ui.info("---------------------------");
        check_python_dependencies(ui, &project_root)?;
        ui.info("");

        ui.heading("Odoo in project:");
        ui.info("----------------");
        check_odoo_in_project(ui, &project_root)?;
        ui.info("");
    }

    ui.info("=".repeat(50));
    if all_ok {
        ui.success("All requirements met");
    } else {
        ui.warn("Some requirements are missing. Please install them before proceeding.");
    }

    Ok(())
}

fn check_python(ui: &Ui) -> Result<bool, String> {
    match check_python_version("3.10") {
        Ok((version, path)) => {
            ui.check(true, "Python", Some(&format!("{} ({})", version, path)));
            Ok(true)
        }
        Err(e) => {
            ui.check(false, "Python", Some(&e));
            Ok(false)
        }
    }
}

fn check_git(ui: &Ui) -> Result<bool, String> {
    match check_command_exists("git") {
        Ok(path) => {
            match get_command_version("git") {
                Ok(version) => {
                    ui.check(
                        true,
                        "Git",
                        Some(&format!(
                            "{} ({})",
                            version.lines().next().unwrap_or("unknown"),
                            path
                        )),
                    );
                }
                Err(_) => {
                    ui.check(true, "Git", Some(&format!("installed ({})", path)));
                }
            }
            Ok(true)
        }
        Err(e) => {
            ui.check(false, "Git", Some(&e));
            Ok(false)
        }
    }
}

fn check_docker(ui: &Ui) -> Result<bool, String> {
    let mut docker_ok = false;
    let mut compose_ok = false;

    match check_command_exists("docker") {
        Ok(path) => {
            match get_command_version("docker") {
                Ok(version) => {
                    let ver_line = version.lines().next().unwrap_or("unknown");
                    ui.check(true, "Docker", Some(&format!("{} ({})", ver_line, path)));
                }
                Err(_) => {
                    ui.check(true, "Docker", Some(&format!("installed ({})", path)));
                }
            }
            docker_ok = true;
        }
        Err(_) => {
            ui.warn("Docker not found (optional, for database operations)");
        }
    }

    if which::which("compose").is_ok() || which::which("docker-compose").is_ok() {
        let compose_cmd = if which::which("compose").is_ok() {
            "docker compose"
        } else {
            "docker-compose"
        };

        match get_command_version(if compose_cmd == "docker compose" {
            "compose"
        } else {
            "docker-compose"
        }) {
            Ok(version) => {
                ui.check(
                    true,
                    "Docker Compose",
                    Some(&format!(
                        "{} ({})",
                        version.lines().next().unwrap_or("unknown"),
                        compose_cmd
                    )),
                );
            }
            Err(_) => {
                ui.check(true, "Docker Compose", Some(&format!("installed ({})", compose_cmd)));
            }
        }
        compose_ok = true;
    } else {
        ui.warn("Docker Compose not found (optional, for database operations)");
    }

    Ok(docker_ok && compose_ok)
}

fn check_linux_dependencies(ui: &Ui) -> Result<bool, String> {
    let mut all_ok = true;

    let common_packages = vec!["build-essential", "python3-dev", "python3-pip"];

    let optional_packages = vec![
        ("libpq-dev", "PostgreSQL development libraries"),
        ("libxml2-dev", "XML libraries (for lxml)"),
        ("libxslt1-dev", "XSLT libraries (for lxml)"),
        ("libjpeg-dev", "JPEG libraries (for Pillow)"),
        ("zlib1g-dev", "Zlib libraries (for Pillow)"),
        ("libssl-dev", "SSL libraries (for cryptography)"),
        ("libffi-dev", "FFI libraries (for cryptography)"),
    ];

    ui.info("Checking common packages...");
    for package in common_packages {
        if check_system_package(package) {
            ui.check(true, package, None);
        } else {
            ui.check(false, package, Some("(recommended)"));
            all_ok = false;
        }
    }

    ui.info("");
    ui.info("Checking optional packages...");
    for (package, description) in optional_packages {
        if check_system_package(package) {
            ui.check(true, package, Some(&format!("- {}", description)));
        } else {
            ui.warn(format!(
                "{} - {} (may be needed for some Python packages)",
                package, description
            ));
        }
    }

    Ok(all_ok)
}

fn check_windows_dependencies(ui: &Ui) -> Result<bool, String> {
    ui.heading("Windows-specific checks:");
    ui.info("Visual C++ Build Tools may be required for some Python packages");
    ui.info("WSL2 is recommended for better compatibility");
    ui.info("PostgreSQL client libraries are optional");
    Ok(true)
}

fn check_macos_dependencies(ui: &Ui) -> Result<bool, String> {
    ui.heading("macOS-specific checks:");

    if which::which("brew").is_ok() {
        ui.check(true, "Homebrew", Some("installed"));
    } else {
        ui.check(false, "Homebrew", Some("not found (recommended for package management)"));
    }

    if Path::new("/Library/Developer/CommandLineTools").exists() {
        ui.check(true, "Xcode Command Line Tools", Some("installed"));
    } else {
        ui.check(
            false,
            "Xcode Command Line Tools",
            Some("not found (run: xcode-select --install)"),
        );
    }

    ui.info("Common packages: postgresql, python3-dev");

    Ok(true)
}

fn check_python_dependencies(ui: &Ui, project_root: &Path) -> Result<(), String> {
    let requirements_file = project_root.join("src/odoo/requirements.txt");

    if !requirements_file.exists() {
        ui.warn("requirements.txt not found (project may not be initialized)");
        return Ok(());
    }

    let requirements_content = fs::read_to_string(&requirements_file)
        .map_err(|e| format!("Failed to read requirements.txt: {}", e))?;

    let mut packages = Vec::new();
    for line in requirements_content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let package_name = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .split(|c| c == '=' || c == '>' || c == '<')
            .next()
            .unwrap_or("")
            .to_string();

        if !package_name.is_empty() {
            packages.push(package_name);
        }
    }

    if packages.is_empty() {
        ui.warn("No Python packages found in requirements.txt");
        return Ok(());
    }

    ui.info(format!(
        "Found {} Python packages in requirements.txt",
        packages.len()
    ));
    ui.info("(Install with: odx install)");

    Ok(())
}

fn check_odoo_in_project(ui: &Ui, project_root: &Path) -> Result<(), String> {
    let odoo_path = project_root.join("src/odoo");
    if !odoo_path.exists() {
        ui.warn("src/odoo not found (create a project with 'odx new')");
        return Ok(());
    }
    match detect_odoo_version(project_root) {
        Ok(version) => ui.info(format!("Odoo version: {}", version)),
        Err(e) => ui.warn(e),
    }
    Ok(())
}

fn format_os_name(os: &str) -> &str {
    match os {
        "linux" => "Linux",
        "windows" => "Windows",
        "macos" => "macOS",
        _ => "Unknown",
    }
}
