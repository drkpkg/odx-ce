use crate::utils::{
    check_command_exists, check_python_version, check_system_package, detect_odoo_version,
    detect_os, find_project_root, get_command_version,
};
use std::fs;
use std::path::Path;

pub fn execute() -> Result<(), String> {
    let os = detect_os();

    println!("Odoo Framework - System Requirements Check");
    println!("===========================================\n");
    println!("Operating System: {}", format_os_name(os));
    println!();

    let mut all_ok = true;

    // Check common dependencies
    println!("Common Dependencies:");
    println!("--------------------");

    all_ok &= check_python()?;
    all_ok &= check_git()?;
    all_ok &= check_docker()?;

    println!();

    // Check OS-specific dependencies
    println!("System Dependencies ({})", format_os_name(os));
    println!("{}", "-".repeat(30));
    match os {
        "linux" => all_ok &= check_linux_dependencies()?,
        "windows" => all_ok &= check_windows_dependencies()?,
        "macos" => all_ok &= check_macos_dependencies()?,
        _ => {
            println!("⚠  OS-specific checks not available for this platform");
        }
    }

    println!();

    // Check Python dependencies from project
    if let Ok(project_root) = find_project_root() {
        println!("Project Python Dependencies:");
        println!("---------------------------");
        check_python_dependencies(&project_root)?;
        println!();

        // Odoo version in project (Community Edition: vanilla Odoo, no core patch)
        println!("Odoo in project:");
        println!("----------------");
        check_odoo_in_project(&project_root)?;
        println!();
    }

    // Final status
    println!("{}", "=".repeat(50));
    if all_ok {
        println!("✓ All requirements met");
    } else {
        println!("⚠  Some requirements are missing. Please install them before proceeding.");
    }

    Ok(())
}

fn check_python() -> Result<bool, String> {
    match check_python_version("3.10") {
        Ok((version, path)) => {
            println!("✓ Python {} ({})", version, path);
            Ok(true)
        }
        Err(e) => {
            println!("✗ {}", e);
            Ok(false)
        }
    }
}

fn check_git() -> Result<bool, String> {
    match check_command_exists("git") {
        Ok(path) => {
            match get_command_version("git") {
                Ok(version) => {
                    println!(
                        "✓ Git {} ({})",
                        version.lines().next().unwrap_or("unknown"),
                        path
                    );
                }
                Err(_) => {
                    println!("✓ Git installed ({})", path);
                }
            }
            Ok(true)
        }
        Err(e) => {
            println!("✗ {}", e);
            Ok(false)
        }
    }
}

fn check_docker() -> Result<bool, String> {
    let mut docker_ok = false;
    let mut compose_ok = false;

    match check_command_exists("docker") {
        Ok(path) => {
            match get_command_version("docker") {
                Ok(version) => {
                    let ver_line = version.lines().next().unwrap_or("unknown");
                    println!("✓ Docker {} ({})", ver_line, path);
                }
                Err(_) => {
                    println!("✓ Docker installed ({})", path);
                }
            }
            docker_ok = true;
        }
        Err(_) => {
            println!("⚠  Docker not found (optional, for database operations)");
        }
    }

    // Check Docker Compose
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
                println!(
                    "✓ Docker Compose {} ({})",
                    version.lines().next().unwrap_or("unknown"),
                    compose_cmd
                );
            }
            Err(_) => {
                println!("✓ Docker Compose installed ({})", compose_cmd);
            }
        }
        compose_ok = true;
    } else {
        println!("⚠  Docker Compose not found (optional, for database operations)");
    }

    Ok(docker_ok && compose_ok)
}

fn check_linux_dependencies() -> Result<bool, String> {
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

    println!("Checking common packages...");
    for package in common_packages {
        if check_system_package(package) {
            println!("  ✓ {}", package);
        } else {
            println!("  ✗ {} (recommended)", package);
            all_ok = false;
        }
    }

    println!("\nChecking optional packages...");
    for (package, description) in optional_packages {
        if check_system_package(package) {
            println!("  ✓ {} - {}", package, description);
        } else {
            println!(
                "  ⚠  {} - {} (may be needed for some Python packages)",
                package, description
            );
        }
    }

    Ok(all_ok)
}

fn check_windows_dependencies() -> Result<bool, String> {
    println!("Windows-specific checks:");
    println!("  ℹ  Visual C++ Build Tools may be required for some Python packages");
    println!("  ℹ  WSL2 is recommended for better compatibility");
    println!("  ℹ  PostgreSQL client libraries are optional");
    Ok(true)
}

fn check_macos_dependencies() -> Result<bool, String> {
    println!("macOS-specific checks:");

    // Check for Homebrew
    if which::which("brew").is_ok() {
        println!("  ✓ Homebrew installed");
    } else {
        println!("  ⚠  Homebrew not found (recommended for package management)");
    }

    // Check for Xcode Command Line Tools
    if Path::new("/Library/Developer/CommandLineTools").exists() {
        println!("  ✓ Xcode Command Line Tools installed");
    } else {
        println!("  ⚠  Xcode Command Line Tools not found (run: xcode-select --install)");
    }

    println!("  ℹ  Common packages: postgresql, python3-dev");

    Ok(true)
}

fn check_python_dependencies(project_root: &Path) -> Result<(), String> {
    let requirements_file = project_root.join("src/odoo/requirements.txt");

    if !requirements_file.exists() {
        println!("⚠  requirements.txt not found (project may not be initialized)");
        return Ok(());
    }

    let requirements_content = fs::read_to_string(&requirements_file)
        .map_err(|e| format!("Failed to read requirements.txt: {}", e))?;

    // Parse requirements.txt (simple parsing)
    let mut packages = Vec::new();
    for line in requirements_content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Extract package name (before ==, >=, etc.)
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
        println!("⚠  No Python packages found in requirements.txt");
        return Ok(());
    }

    println!(
        "Found {} Python packages in requirements.txt",
        packages.len()
    );
    println!("(Install with: odx install)");

    Ok(())
}

fn check_odoo_in_project(project_root: &Path) -> Result<(), String> {
    let odoo_path = project_root.join("src/odoo");
    if !odoo_path.exists() {
        println!("  ⚠  src/odoo not found (create a project with 'odx new')");
        return Ok(());
    }
    match detect_odoo_version(project_root) {
        Ok(version) => println!("  Odoo version: {}", version),
        Err(e) => println!("  ⚠  {}", e),
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
