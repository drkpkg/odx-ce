use crate::debug;
use crate::install_guide::{build_install_guide, Requirement};
use crate::odoo_source;
use crate::os_context::{LinuxFamily, OsContext, PackageManager, Platform};
use crate::ui::Ui;
use crate::utils::{
    check_command_exists, check_python_version, check_system_package, detect_os,
    find_docker_compose_command, find_project_root, find_python_command, get_command_version,
    require_odoo_bin,
};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn execute(ui: &Ui) -> Result<(), String> {
    let os = detect_os();
    let ctx = OsContext::detect();

    ui.heading("Odoo Framework - System Requirements Check");
    ui.info("===========================================");
    ui.info("");
    ui.info(format!(
        "Operating System: {}",
        ctx.pretty_name.as_deref().unwrap_or(format_os_name(os))
    ));
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
        "linux" => all_ok &= check_linux_dependencies(ui, &ctx)?,
        "windows" => all_ok &= check_windows_dependencies(ui, &ctx)?,
        "macos" => all_ok &= check_macos_dependencies(ui)?,
        _ => {
            ui.warn("OS-specific checks not available for this platform");
        }
    }

    ui.info("");

    if let Ok(project_root) = find_project_root() {
        ui.heading("Project layout:");
        ui.info("---------------");
        check_project_compose(ui, &project_root)?;
        ui.info("");

        ui.heading("Project Python Dependencies:");
        ui.info("---------------------------");
        check_python_dependencies(ui, &project_root)?;
        ui.info("");

        ui.heading("Debugger (DAP):");
        ui.info("---------------");
        all_ok &= check_debugger(ui);
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
        ui.info("WARNING: Some requirements are missing. Please install them before proceeding.");
    }

    ui.info("");
    print_install_guide(ui, &ctx);

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

    match find_docker_compose_command() {
        Ok(compose_cmd) => {
            let version = if compose_cmd == "docker compose" {
                Command::new("docker")
                    .args(["compose", "version"])
                    .output()
                    .ok()
                    .and_then(|o| {
                        if o.status.success() {
                            String::from_utf8(o.stdout).ok()
                        } else {
                            None
                        }
                    })
            } else {
                get_command_version("docker-compose").ok()
            };

            let detail = version
                .as_ref()
                .and_then(|v| v.lines().next())
                .map(|line| format!("{} ({})", line, compose_cmd))
                .unwrap_or_else(|| format!("installed ({})", compose_cmd));

            ui.check(true, "Docker Compose", Some(&detail));
            compose_ok = true;
        }
        Err(e) => {
            ui.warn(format!("{} (optional, for database operations)", e));
        }
    }

    Ok(docker_ok && compose_ok)
}

fn check_linux_dependencies(ui: &Ui, ctx: &OsContext) -> Result<bool, String> {
    let mut all_ok = true;

    let family = ctx.linux_family.unwrap_or(LinuxFamily::Unknown);
    let pm = ctx.package_manager;

    ui.info("Checking common packages...");
    for (pkg, ok) in check_odoo_full_packages(pm, family) {
        if ok {
            ui.check(true, &pkg, None);
        } else {
            ui.check(false, &pkg, Some("(recommended)"));
            all_ok = false;
        }
    }

    Ok(all_ok)
}

fn check_windows_dependencies(ui: &Ui, ctx: &OsContext) -> Result<bool, String> {
    ui.heading("Windows-specific checks:");
    ui.info("Visual C++ Build Tools may be required for some Python packages");
    ui.info("WSL2 is recommended for better compatibility");
    ui.info("PostgreSQL client libraries are optional");
    if ctx.package_manager == PackageManager::Winget {
        ui.check(true, "winget", Some("available"));
    } else {
        ui.check(
            false,
            "winget",
            Some("not found (optional, for easy installs)"),
        );
    }
    Ok(true)
}

fn check_macos_dependencies(ui: &Ui) -> Result<bool, String> {
    ui.heading("macOS-specific checks:");

    if which::which("brew").is_ok() {
        ui.check(true, "Homebrew", Some("installed"));
    } else {
        ui.check(
            false,
            "Homebrew",
            Some("not found (recommended for package management)"),
        );
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

/// odx always starts Odoo with a debugpy listener, so a missing debugpy is a real
/// finding rather than an optional extra.
fn check_debugger(ui: &Ui) -> bool {
    let python = match find_python_command() {
        Ok(p) => p,
        Err(e) => {
            ui.check(false, "debugpy", Some(&e));
            return false;
        }
    };

    if debug::is_available(&python) {
        let version = Command::new(&python)
            .args(["-c", "import debugpy; print(debugpy.__version__)"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let detail = if version.is_empty() {
            "installed".to_string()
        } else {
            format!(
                "{} (attach on {}:{})",
                version,
                debug::HOST,
                debug::DEFAULT_PORT
            )
        };
        ui.check(true, "debugpy", Some(&detail));
        true
    } else {
        ui.check(false, "debugpy", Some("not installed - run 'odx install'"));
        false
    }
}

fn check_python_dependencies(ui: &Ui, project_root: &Path) -> Result<(), String> {
    let Ok(source) = odoo_source::resolve(project_root) else {
        ui.warn("Odoo source not available yet, skipping requirements check (run 'odx install')");
        return Ok(());
    };
    let requirements_file = source.requirements();

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
            .split(['=', '>', '<'])
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

fn check_project_compose(ui: &Ui, project_root: &Path) -> Result<(), String> {
    let compose_yml = project_root.join("compose.yml");
    let compose_yaml = project_root.join("compose.yaml");
    let compose_path = if compose_yml.exists() {
        Some(compose_yml)
    } else if compose_yaml.exists() {
        Some(compose_yaml)
    } else {
        None
    };

    match compose_path {
        Some(path) => ui.check(true, "compose file", Some(&path.display().to_string())),
        None => {
            ui.check(
                false,
                "compose file",
                Some("compose.yml or compose.yaml missing"),
            );
            return Ok(());
        }
    }

    if let Ok(compose_cmd) = find_docker_compose_command() {
        let output = if compose_cmd == "docker compose" {
            Command::new("docker")
                .args(["compose", "config", "--services"])
                .current_dir(project_root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        } else {
            Command::new(&compose_cmd)
                .args(["config", "--services"])
                .current_dir(project_root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        };
        match output {
            Ok(output) if output.status.success() => {
                let services = String::from_utf8_lossy(&output.stdout);
                let has_postgres = services.lines().any(|line| line.trim() == "postgres");
                if has_postgres {
                    ui.check(true, "postgres service", Some("defined in compose file"));
                } else {
                    ui.check(
                        false,
                        "postgres service",
                        Some("not found in compose file (odx db expects service name 'postgres')"),
                    );
                }
            }
            Ok(_) => {
                ui.warn(
                    "Could not validate compose services (run 'docker compose config' manually)",
                );
            }
            Err(e) => {
                ui.warn(format!("Could not run docker compose config: {}", e));
            }
        }
    }

    Ok(())
}

fn check_odoo_in_project(ui: &Ui, project_root: &Path) -> Result<(), String> {
    ui.info(format!("Store: {}", odoo_source::store_root().display()));

    let source = match odoo_source::resolve(project_root) {
        Ok(source) => source,
        Err(e) => {
            ui.check(false, "Odoo source", Some(&e));
            return Ok(());
        }
    };

    ui.check(
        true,
        "Odoo source",
        Some(&format!(
            "{} ({}) — {}",
            source.path.display(),
            source.origin.label(),
            source.version
        )),
    );

    if source.origin == odoo_source::Origin::InProject {
        ui.warn(
            "This project keeps its own Odoo checkout in src/odoo. Newer projects share one \
             per version (see 'odx store ls'); delete src/odoo and add the version to .odx.toml \
             to reclaim the space.",
        );
    }

    if let Err(e) = require_odoo_bin(project_root) {
        ui.warn(e);
        return Ok(());
    }

    // Agents cannot read outside the project unless the directory is declared, and the
    // source now lives outside it.
    if source.origin == odoo_source::Origin::Store {
        let declared = project_root.join(".claude/settings.json");
        let granted = fs::read_to_string(&declared)
            .map(|c| c.contains(source.path.to_string_lossy().as_ref()))
            .unwrap_or(false);
        ui.check(
            granted,
            "agent access to Odoo source",
            Some(if granted {
                "declared in .claude/settings.json"
            } else {
                "not declared - agents cannot read Odoo core; add it to .claude/settings.json permissions.additionalDirectories"
            }),
        );
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

fn check_odoo_full_packages(pm: PackageManager, family: LinuxFamily) -> Vec<(String, bool)> {
    use Requirement::*;

    // Keep a stable, minimal set for checks (avoid over-reporting).
    // The install guide will include the full list.
    let check_set = [
        BuildTools, PythonDev, PythonPip, LibPQDev, LibXML2Dev, LibXSLTDev, LibJPEGDev, ZlibDev,
        OpenSSLDev, LibFFIDev,
    ];

    check_set
        .iter()
        .flat_map(|r| crate::install_guide::linux_pkg_names(pm, family, *r))
        .map(|pkg| {
            let ok = check_system_package(&pkg);
            (pkg, ok)
        })
        .collect()
}

fn print_install_guide(ui: &Ui, ctx: &OsContext) {
    // No macOS support for now per project preference; keep output focused.
    if ctx.platform == Platform::Unknown {
        return;
    }

    ui.heading("How to install Odoo dependencies (Odoo full)");
    let guide = build_install_guide(ctx);
    ui.info(format!("Detected: {}", guide.detected));
    if let Some(cmd) = guide.command {
        ui.info("Recommended command:");
        ui.info(cmd);
    } else {
        ui.warn("Could not generate an exact install command for this system.");
    }
    for n in guide.notes {
        ui.info(format!("- {}", n));
    }
}
