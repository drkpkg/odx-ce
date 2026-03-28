use crate::utils::{create_venv, execute_command, find_project_root};
use std::fs;
use std::path::Path;

pub fn execute() -> Result<(), String> {
    let project_root = find_project_root()?;

    println!("Setting up development environment...");

    // Create virtual environment directly using Rust helper
    // This avoids executing any Python scripts from the project scripts directory.
    create_venv(&project_root, "python")?;

    println!("\nBuilding Odoo CLI...");
    build_cli(&project_root)?;

    println!("Installing Odoo CLI binary...");
    install_cli(&project_root)?;

    println!("Setup completed successfully!");
    println!("You can now use 'odx' command from anywhere in the project.");

    Ok(())
}

fn build_cli(project_root: &Path) -> Result<(), String> {
    let cli_dir = project_root.join("cli");
    if !cli_dir.exists() {
        return Err("CLI directory not found. Make sure you're in the project root.".to_string());
    }

    which::which("cargo")
        .map_err(|_| "Cargo not found. Please install Rust: https://rustup.rs/".to_string())?;

    execute_command("cargo", &["build", "--release"], Some(&cli_dir))?;

    Ok(())
}

fn install_cli(project_root: &Path) -> Result<(), String> {
    let release_dir = project_root.join("cli/target/release");

    let binary_name = if cfg!(windows) { "odx.exe" } else { "odx" };

    let source_binary = release_dir.join(binary_name);
    if !source_binary.exists() {
        return Err(format!(
            "Compiled binary not found: {}. Please build the CLI first.",
            source_binary.display()
        ));
    }

    let venv_bin = if cfg!(windows) {
        project_root.join(".venv/Scripts")
    } else {
        project_root.join(".venv/bin")
    };

    if !venv_bin.exists() {
        return Err("Virtual environment not found. Run Python setup first.".to_string());
    }

    let target_binary = venv_bin.join(binary_name);

    fs::copy(&source_binary, &target_binary)
        .map_err(|e| format!("Failed to copy binary: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target_binary)
            .map_err(|e| format!("Failed to get file metadata: {}", e))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target_binary, perms)
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    println!("Odoo CLI installed to: {}", target_binary.display());

    Ok(())
}
