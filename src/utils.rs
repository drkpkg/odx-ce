use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn find_project_root() -> Result<PathBuf, String> {
    let mut current =
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;

    loop {
        let compose_file = current.join("compose.yml");
        let odoo_bin = current.join("src/odoo/odoo-bin");

        if compose_file.exists() && odoo_bin.exists() {
            return Ok(current);
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return Err("Could not find project root (compose.yml not found)".to_string()),
        }
    }
}

pub fn find_python_command() -> Result<String, String> {
    let project_root = find_project_root()?;

    if cfg!(windows) {
        let venv_python = project_root.join(".venv/Scripts/python.exe");
        if venv_python.exists() {
            return Ok(venv_python.to_string_lossy().to_string());
        }
    } else {
        let venv_python = project_root.join(".venv/bin/python3");
        if venv_python.exists() {
            return Ok(venv_python.to_string_lossy().to_string());
        }
    }

    which::which("python3")
        .or_else(|_| which::which("python"))
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|_| "Python not found. Please install Python or run 'odoo setup'".to_string())
}

pub fn ensure_venv() -> Result<(), String> {
    let project_root = find_project_root()?;
    let venv_path = if cfg!(windows) {
        project_root.join(".venv/Scripts")
    } else {
        project_root.join(".venv/bin")
    };

    if !venv_path.exists() {
        return Err("Virtual environment not found. Run 'odoo setup' first.".to_string());
    }

    Ok(())
}

pub fn execute_command(
    program: &str,
    args: &[&str],
    working_dir: Option<&Path>,
) -> Result<(), String> {
    let mut cmd = Command::new(program);

    cmd.args(args);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|e| format!("Failed to execute {}: {}", program, e))?;

    if !status.success() {
        return Err(format!(
            "Command failed with exit code: {:?}",
            status.code()
        ));
    }

    Ok(())
}

pub fn find_docker_compose_command() -> Result<String, String> {
    which::which("docker").map_err(|_| "Docker not found. Please install Docker".to_string())?;

    if which::which("docker").is_ok() && which::which("compose").is_ok() {
        Ok("docker compose".to_string())
    } else if which::which("docker-compose").is_ok() {
        Ok("docker-compose".to_string())
    } else {
        Err("Docker Compose not found. Please install Docker Compose".to_string())
    }
}

/// Detect the operating system
pub fn detect_os() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    }
}

/// Check if a command exists and return its path
pub fn check_command_exists(cmd: &str) -> Result<String, String> {
    which::which(cmd)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|_| format!("Command '{}' not found", cmd))
}

/// Resolve Python interpreter for a given version (e.g. "3.11", "3.12").
/// Tries: pyenv which <version>, then python<version> (e.g. python3.11), then python3/python with version check.
pub fn resolve_python(version: &str) -> Result<String, String> {
    let version = version.trim();
    let (want_major, want_minor) = parse_major_minor(version)?;

    // 1. Try pyenv (e.g. pyenv which 3.11)
    if which::which("pyenv").is_ok() {
        let out = Command::new("pyenv")
            .arg("which")
            .arg(version)
            .output()
            .ok();
        if let Some(o) = out {
            if o.status.success() {
                let path = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !path.is_empty() && path.contains("python") {
                    if check_python_version_matches(&path, want_major, want_minor) {
                        return Ok(path);
                    }
                }
            }
        }
    }

    // 2. Try python<version> (e.g. python3.11)
    let name = if version.starts_with("python") {
        version.to_string()
    } else {
        format!("python{}", version)
    };
    if let Ok(cmd) = which::which(&name) {
        let path = cmd.to_string_lossy().to_string();
        if check_python_version_matches(&path, want_major, want_minor) {
            return Ok(path);
        }
    }

    // 3. Try python3 / python and check version
    let candidates = ["python3", "python"];
    for name in candidates {
        if let Ok(cmd) = which::which(name) {
            let path = cmd.to_string_lossy().to_string();
            if check_python_version_matches(&path, want_major, want_minor) {
                return Ok(path);
            }
        }
    }

    Err(format!(
        "Python {} not found. Install it or use pyenv (e.g. pyenv install 3.11 && pyenv local 3.11).",
        version
    ))
}

fn parse_major_minor(version: &str) -> Result<(u32, u32), String> {
    let v = version.trim_start_matches("python");
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() < 2 {
        return Err(format!("Invalid Python version '{}'. Use e.g. 3.11 or 3.12", version));
    }
    let major = parts[0]
        .parse::<u32>()
        .map_err(|_| format!("Invalid major version: {}", parts[0]))?;
    let minor = parts[1]
        .parse::<u32>()
        .map_err(|_| format!("Invalid minor version: {}", parts[1]))?;
    Ok((major, minor))
}

fn check_python_version_matches(python_path: &str, want_major: u32, want_minor: u32) -> bool {
    let output = Command::new(python_path).arg("--version").output().ok();
    let output = match output {
        Some(o) => o,
        None => return false,
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let text_stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{} {}", text, text_stderr);
    let re = Regex::new(r"(\d+)\.(\d+)").unwrap();
    if let Some(caps) = re.captures(&combined) {
        if let (Ok(maj), Ok(min)) = (
            caps.get(1).unwrap().as_str().parse::<u32>(),
            caps.get(2).unwrap().as_str().parse::<u32>(),
        ) {
            return maj == want_major && min == want_minor;
        }
    }
    false
}

/// Check Python version and return (version_string, path)
pub fn check_python_version(min_version: &str) -> Result<(String, String), String> {
    // Try python3 first, then python
    let python_cmd = which::which("python3")
        .or_else(|_| which::which("python"))
        .map_err(|_| "Python not found. Please install Python 3.10 or higher".to_string())?;

    let python_path = python_cmd.to_string_lossy().to_string();

    // Get Python version
    let output = Command::new(&python_path)
        .arg("--version")
        .output()
        .map_err(|e| format!("Failed to execute Python: {}", e))?;

    if !output.status.success() {
        return Err("Failed to get Python version".to_string());
    }

    let version_output = String::from_utf8_lossy(&output.stdout);

    // Extract version number (e.g., "Python 3.10.12" -> "3.10.12")
    let re = Regex::new(r"(\d+)\.(\d+)\.(\d+)").unwrap();
    let version_str = if let Some(caps) = re.captures(&version_output) {
        format!("{}.{}.{}", &caps[1], &caps[2], &caps[3])
    } else {
        return Err(format!(
            "Could not parse Python version: {}",
            version_output
        ));
    };

    // Parse minimum version requirement
    let min_parts: Vec<&str> = min_version.split('.').collect();
    let version_parts: Vec<&str> = version_str.split('.').collect();

    if min_parts.len() < 2 || version_parts.len() < 2 {
        return Err("Invalid version format".to_string());
    }

    let min_major: u32 = min_parts[0].parse().unwrap_or(0);
    let min_minor: u32 = min_parts[1].parse().unwrap_or(0);
    let ver_major: u32 = version_parts[0].parse().unwrap_or(0);
    let ver_minor: u32 = version_parts[1].parse().unwrap_or(0);

    if ver_major > min_major || (ver_major == min_major && ver_minor >= min_minor) {
        Ok((version_str, python_path))
    } else {
        Err(format!(
            "Python version {} is required, but found {}. Please install Python {} or higher",
            min_version, version_str, min_version
        ))
    }
}

/// Create project directory structure
pub fn create_project_structure(root: &Path) -> Result<(), String> {
    let dirs = vec![
        "custom_addons",
        "docs",
        "external_addons",
        "scripts",
        ".testing",
        "src",
    ];

    for dir in dirs {
        let dir_path = root.join(dir);
        fs::create_dir_all(&dir_path)
            .map_err(|e| format!("Failed to create directory {}: {}", dir, e))?;
    }

    Ok(())
}

/// Generate file content from template with variable substitution
pub fn generate_from_template(template: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template.to_string();

    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }

    result
}

/// Get command version (if available)
pub fn get_command_version(cmd: &str) -> Result<String, String> {
    let output = Command::new(cmd)
        .arg("--version")
        .output()
        .map_err(|e| format!("Failed to execute {}: {}", cmd, e))?;

    if !output.status.success() {
        return Err(format!("Failed to get {} version", cmd));
    }

    let version = String::from_utf8_lossy(&output.stdout);
    Ok(version.trim().to_string())
}

/// Check if a system package is installed (Linux only)
pub fn check_system_package(package: &str) -> bool {
    if detect_os() != "linux" {
        return false;
    }

    // Try different package managers
    let package_managers = vec![
        ("dpkg", vec!["-l", package]),
        ("rpm", vec!["-q", package]),
        ("pacman", vec!["-Q", package]),
    ];

    for (pm, args) in package_managers {
        if which::which(pm).is_ok() {
            let output = Command::new(pm).args(&args).output();

            if let Ok(output) = output {
                if output.status.success() {
                    return true;
                }
            }
        }
    }

    false
}

/// Initialize Git repository
pub fn init_git_repo(path: &Path) -> Result<(), String> {
    let status = Command::new("git")
        .arg("init")
        .current_dir(path)
        .status()
        .map_err(|e| format!("Failed to initialize Git repository: {}", e))?;

    if !status.success() {
        return Err("Git init failed".to_string());
    }

    Ok(())
}

/// Create Python virtual environment
pub fn create_venv(path: &Path, python: &str) -> Result<(), String> {
    let venv_path = path.join(".venv");

    if venv_path.exists() {
        return Ok(()); // Already exists
    }

    let status = Command::new(python)
        .arg("-m")
        .arg("venv")
        .arg(&venv_path)
        .current_dir(path)
        .status()
        .map_err(|e| format!("Failed to create virtual environment: {}", e))?;

    if !status.success() {
        return Err("Failed to create virtual environment".to_string());
    }

    Ok(())
}

/// Extract Odoo from ZIP file
pub fn extract_odoo_from_zip(zip_path: &Path, target_path: &Path) -> Result<(), String> {
    if !zip_path.exists() {
        return Err(format!("ZIP file not found: {}", zip_path.display()));
    }

    // Validate ZIP before attempting extraction
    if !validate_zip_file(zip_path)? {
        return Err(format!(
            "ZIP file is invalid or corrupted: {}",
            zip_path.display()
        ));
    }

    // Create target directory
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create target directory: {}", e))?;
    }

    // Extract using unzip command (more reliable than Rust zip crate)
    let extract_dir = target_path.parent().ok_or("Invalid target path")?;

    let output = Command::new("unzip")
        .arg("-q") // Quiet mode
        .arg("-o") // Overwrite files
        .arg(zip_path)
        .arg("-d")
        .arg(extract_dir)
        .output()
        .map_err(|e| format!("Failed to extract ZIP (unzip not found?): {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to extract ZIP file: {}", stderr.trim()));
    }

    // The extracted directory will be odoo-{version}, we need to rename it
    let extracted_dir = extract_dir.join(format!(
        "odoo-{}",
        zip_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .replace("odoo-", "")
    ));

    if extracted_dir.exists() && extracted_dir != target_path {
        // Move/rename to target path
        if target_path.exists() {
            fs::remove_dir_all(target_path)
                .map_err(|e| format!("Failed to remove existing target: {}", e))?;
        }
        fs::rename(&extracted_dir, target_path)
            .map_err(|e| format!("Failed to rename extracted directory: {}", e))?;
    }

    Ok(())
}

/// Validate ZIP file by checking signature and size
pub fn validate_zip_file(zip_path: &Path) -> Result<bool, String> {
    use std::io::Read;

    // Check file size (should be at least 1MB)
    let metadata =
        fs::metadata(zip_path).map_err(|e| format!("Failed to read ZIP metadata: {}", e))?;
    if metadata.len() < 1_000_000 {
        return Ok(false);
    }

    // Check ZIP signature
    let mut file =
        fs::File::open(zip_path).map_err(|e| format!("Failed to open ZIP file: {}", e))?;
    let mut buffer = [0u8; 4];
    if file.read_exact(&mut buffer).is_err() {
        return Ok(false);
    }

    // Verify ZIP signature (PK\x03\x04 or PK\x05\x06)
    let is_valid = buffer.starts_with(b"PK\x03\x04") || buffer.starts_with(b"PK\x05\x06");
    Ok(is_valid)
}

/// Get Odoo ZIP file path for a version
pub fn get_odoo_zip_path(version: &str) -> PathBuf {
    let mut zip_path = PathBuf::from(".testing/odoo-zips");
    zip_path.push(format!("odoo-{}.zip", version));
    zip_path
}

/// Collect all directories under `root` that have at least one direct child with __manifest__.py.
/// This allows nested layouts (e.g. custom_addons/company/team/module_x) to be found by Odoo.
fn addon_root_dirs_under(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if !root.exists() || !root.is_dir() {
        return result;
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return result,
    };
    let has_direct_addon = entries
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_dir() && e.path().join("__manifest__.py").exists());
    if has_direct_addon {
        result.push(root.to_path_buf());
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return result,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_dir() {
            result.extend(addon_root_dirs_under(&p));
        }
    }
    result
}

/// Direct children of external_addons that contain at least one addon (parent folders only).
/// Odoo will scan each of these and find addons inside (e.g. external_addons/dms, external_addons/knowledge).
pub fn external_addons_dirs_with_manifest(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let external = project_root.join("external_addons");
    if !external.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = addon_root_dirs_under(&external);
    dirs.sort_by(|a, b| a.cmp(b));
    Ok(dirs)
}

/// Build addons_path: only directories that exist (Odoo rejects non-existent paths).
/// Includes custom_addons (and subdirs with modules), external_addons (and subdirs), and src/odoo/addons.
pub fn build_addons_path(project_root: &Path) -> Result<String, String> {
    let external_dirs = external_addons_dirs_with_manifest(project_root)?;

    let root = project_root
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize project root: {}", e))?;

    let mut parts = Vec::new();

    let custom = root.join("custom_addons");
    let mut custom_dirs = addon_root_dirs_under(&custom);
    custom_dirs.sort_by(|a, b| a.cmp(b));
    for d in &custom_dirs {
        let abs = d
            .canonicalize()
            .map_err(|e| format!("Failed to canonicalize {:?}: {}", d, e))?;
        parts.push(abs.to_string_lossy().into_owned());
    }

    for d in &external_dirs {
        let abs = d
            .canonicalize()
            .map_err(|e| format!("Failed to canonicalize {:?}: {}", d, e))?;
        parts.push(abs.to_string_lossy().into_owned());
    }

    let odoo_addons = root.join("src/odoo/addons");
    if odoo_addons.exists() {
        parts.push(odoo_addons.to_string_lossy().into_owned());
    }

    if parts.is_empty() {
        return Err("No valid addons directories found (custom_addons, external_addons subdirs, or src/odoo/addons)".to_string());
    }

    Ok(parts.join(","))
}

/// Ensure odoo.conf.local exists (create from odoo.conf if not) and refresh addons_path
/// with external_addons subdirs that have __manifest__.py.
pub fn ensure_odoo_conf_local(project_root: &Path) -> Result<(), String> {
    let local_path = project_root.join("odoo.conf.local");
    let base_path = project_root.join("odoo.conf");

    if !base_path.exists() {
        return Err("odoo.conf not found. Run from project root.".to_string());
    }

    if !local_path.exists() {
        fs::copy(&base_path, &local_path)
            .map_err(|e| format!("Failed to create odoo.conf.local: {}", e))?;
    }

    let addons_path = build_addons_path(project_root)?;
    let content = fs::read_to_string(&local_path)
        .map_err(|e| format!("Failed to read odoo.conf.local: {}", e))?;

    let addons_re = Regex::new(r"^(\s*addons_path\s*=\s*).*").unwrap();
    let mut updated = false;
    let new_content: String = content
        .lines()
        .map(|line| {
            if let Some(cap) = addons_re.captures(line) {
                updated = true;
                format!("{}{}", &cap[1], addons_path)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    if !updated {
        return Err("addons_path line not found in odoo.conf.local".to_string());
    }

    let out = if new_content.ends_with('\n') {
        new_content
    } else {
        format!("{}\n", new_content)
    };
    fs::write(&local_path, out).map_err(|e| format!("Failed to write odoo.conf.local: {}", e))?;

    Ok(())
}

/// Detect Odoo version from project `src/odoo` (release.py or __init__.py).
pub fn detect_odoo_version(project_root: &Path) -> Result<String, String> {
    let release_py = project_root.join("src/odoo/odoo/release.py");
    if release_py.exists() {
        let content = fs::read_to_string(&release_py)
            .map_err(|e| format!("Failed to read Odoo release.py: {}", e))?;

        let re = Regex::new(r"version_info\s*=\s*\((\d+),\s*(\d+)").unwrap();
        if let Some(caps) = re.captures(&content) {
            return Ok(format!("{}.{}", &caps[1], &caps[2]));
        }

        let re2 = Regex::new(r#"version\s*=\s*['"](\d+)\.(\d+)"#).unwrap();
        if let Some(caps) = re2.captures(&content) {
            return Ok(format!("{}.{}", &caps[1], &caps[2]));
        }
    }

    let odoo_init = project_root.join("src/odoo/odoo/__init__.py");
    if odoo_init.exists() {
        let content = fs::read_to_string(&odoo_init)
            .map_err(|e| format!("Failed to read Odoo __init__.py: {}", e))?;

        let re = Regex::new(r"version_info\s*=\s*\((\d+),\s*(\d+)\)").unwrap();
        if let Some(caps) = re.captures(&content) {
            return Ok(format!("{}.{}", &caps[1], &caps[2]));
        }

        let re2 = Regex::new(r#"__version__\s*=\s*['"](\d+)\.(\d+)"#).unwrap();
        if let Some(caps) = re2.captures(&content) {
            return Ok(format!("{}.{}", &caps[1], &caps[2]));
        }
    }

    Err("Could not detect Odoo version from release.py or __init__.py".to_string())
}
