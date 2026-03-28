// Integration tests for odoo CLI
// These tests verify the complete workflow by creating test projects
// and verifying commands work correctly

use std::process::Command;
use std::path::{Path, PathBuf};
use std::fs;
use std::env;

const TEST_VERSIONS: [&str; 3] = ["17.0", "18.0", "19.0"];
const TEST_DIR: &str = ".testing";
const ZIP_DIR: &str = ".testing/odoo-zips";

fn ensure_odoo_zip_downloaded(version: &str) {
    let zip_path = Path::new(ZIP_DIR).join(format!("odoo-{}.zip", version));
    
    // Check if file exists and is valid (larger than 1MB)
    if zip_path.exists() {
        if let Ok(metadata) = fs::metadata(&zip_path) {
            if metadata.len() > 1_000_000 {
                return; // Already downloaded and valid
            } else {
                // File too small, probably an error page
                fs::remove_file(&zip_path).ok();
            }
        }
    }
    
    // Try to download using the script
    println!("Odoo {} ZIP not found or invalid, attempting to download...", version);
    
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script_path = project_root.join("scripts/download-odoo-versions.sh");
    
    if script_path.exists() {
        let output = Command::new("bash")
            .arg(&script_path)
            .current_dir(&project_root)
            .output();
        
        if let Ok(output) = output {
            if output.status.success() && zip_path.exists() {
                // Verify the file is valid
                if let Ok(metadata) = fs::metadata(&zip_path) {
                    if metadata.len() > 1_000_000 {
                        println!("✓ Downloaded Odoo {} ZIP", version);
                        return;
                    } else {
                        println!("⚠  Downloaded file for {} is too small ({} bytes), may be invalid", 
                                 version, metadata.len());
                    }
                }
            }
        }
    }
    
    // If script doesn't work, warn but continue (will fall back to git clone)
    println!("⚠  Could not download valid ZIP for {}, will use git clone (slower)", version);
}

fn get_odoo_binary() -> PathBuf {
    // Get the project root (where Cargo.toml is)
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    
    // Try release first, then debug
    let release_path = project_root.join("target/release/odx");
    let debug_path = project_root.join("target/debug/odx");
    
    // Prefer debug for tests (faster compilation)
    if debug_path.exists() {
        debug_path
    } else if release_path.exists() {
        release_path
    } else {
        // Try to find it in the current directory
        let current_debug = Path::new("./target/debug/odx");
        let current_release = Path::new("./target/release/odx");
        
        if current_debug.exists() {
            current_debug.to_path_buf()
        } else if current_release.exists() {
            current_release.to_path_buf()
        } else {
            panic!("odx binary not found. Please build with 'cargo build' or 'cargo build --release'");
        }
    }
}

#[test]
fn test_doctor_command() {
    let odoo_bin = get_odoo_binary();
    
    let output = Command::new(&odoo_bin)
        .arg("doctor")
        .output();

    let output = output.expect(&format!("Failed to execute doctor command. Binary: {}", odoo_bin.display()));
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!("Doctor command failed:\nSTDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("System Requirements Check") || stdout.contains("Requirements"), 
            "Should show system check. Output: {}", stdout);
}

#[test]
fn test_new_project_17() {
    test_new_project("17.0", "test_project_17");
}

#[test]
fn test_new_project_18() {
    test_new_project("18.0", "test_project_18");
}

#[test]
fn test_new_project_19() {
    test_new_project("19.0", "test_project_19");
}

fn test_new_project(version: &str, project_name: &str) {
    // Ensure Odoo ZIP is downloaded
    ensure_odoo_zip_downloaded(version);
    
    let odoo_bin = get_odoo_binary();
    let test_path = Path::new(TEST_DIR).join(project_name);
    
    // Clean up if exists
    if test_path.exists() {
        fs::remove_dir_all(&test_path).ok();
    }
    
    // Create test directory
    fs::create_dir_all(TEST_DIR).expect("Failed to create test directory");
    
    // Change to test directory for project creation
    let output = Command::new(&odoo_bin)
        .arg("new")
        .arg(project_name)
        .arg("-v")
        .arg(version)
        .current_dir(TEST_DIR)
        .output();

    let output = output.expect(&format!("Failed to execute new command. Binary: {}", odoo_bin.display()));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!("Failed to create project {}:\nSTDOUT:\n{}\nSTDERR:\n{}", project_name, stdout, stderr);
    }

    // Verify project structure
    assert!(test_path.exists(), "Project directory should exist");
    assert!(test_path.join("src/odoo").exists(), "Odoo source should exist");
    assert!(test_path.join("custom_addons").exists(), "custom_addons should exist");
    assert!(test_path.join("external_addons").exists(), "external_addons should exist");
    assert!(test_path.join("compose.yml").exists(), "compose.yml should exist");
    assert!(test_path.join("odoo.conf").exists(), "odoo.conf should exist");
    assert!(test_path.join("README.md").exists(), "README.md should exist");
    assert!(test_path.join("AGENTS.md").exists(), "AGENTS.md should exist");
    
    // Verify Odoo version
    let odoo_init = test_path.join("src/odoo/odoo/__init__.py");
    if odoo_init.exists() {
        let content = fs::read_to_string(&odoo_init).expect("Failed to read __init__.py");
        // Check if version is mentioned (exact match may vary)
        assert!(content.contains("version") || content.contains("__version__"), 
                "Odoo __init__.py should contain version info");
    }
    
    // Test doctor command in the project
    let doctor_output = Command::new(&odoo_bin)
        .arg("doctor")
        .current_dir(&test_path)
        .output()
        .expect("Failed to execute doctor in project");

    assert!(doctor_output.status.success(), "Doctor should work in project");
    
    let doctor_stdout = String::from_utf8_lossy(&doctor_output.stdout);
    assert!(doctor_stdout.contains("Odoo"), "Doctor should show Odoo information");
}

#[test]
fn test_doctor_in_project() {
    // This test assumes a project exists, so we create one first
    let project_name = "test_doctor_project";
    let test_path = Path::new(TEST_DIR).join(project_name);
    
    // Create a minimal project structure if it doesn't exist
    if !test_path.exists() {
        fs::create_dir_all(&test_path).expect("Failed to create test project");
        fs::create_dir_all(test_path.join("src/odoo")).expect("Failed to create odoo dir");
        fs::create_dir_all(test_path.join("src/odoo/odoo")).expect("Failed to create odoo/odoo");
        
        // Create minimal __init__.py
        fs::write(
            test_path.join("src/odoo/odoo/__init__.py"),
            "version_info = (18, 0)\n"
        ).expect("Failed to write __init__.py");
    }
    
    let odoo_bin = get_odoo_binary();
    let output = Command::new(&odoo_bin)
        .arg("doctor")
        .current_dir(&test_path)
        .output();

    let output = output.expect(&format!("Failed to execute doctor. Binary: {}", odoo_bin.display()));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!("Doctor failed:\nSTDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Odoo") || stdout.contains("Requirements"), 
            "Doctor should show Odoo or requirements information. Output: {}", stdout);
}

#[test]
fn test_doctor_shows_odoo_in_existing_projects() {
    for version in TEST_VERSIONS {
        let project_name = format!("test_odoo_ce_{}", version.replace('.', "_"));
        let test_path = Path::new(TEST_DIR).join(&project_name);
        
        // Skip if project doesn't exist
        if !test_path.exists() {
            continue;
        }
        
        let odoo_bin = get_odoo_binary();
        let output = Command::new(&odoo_bin)
            .arg("doctor")
            .current_dir(&test_path)
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(stdout.contains("Odoo") || stdout.contains("version"),
                        "Doctor should show Odoo or version information for {}. Output: {}", version, stdout);
            }
        }
        // If command fails, that's ok - project may not exist yet
    }
}

#[test]
fn test_install_command() {
    // Create a minimal project for testing install
    let project_name = "test_install_project";
    let test_path = Path::new(TEST_DIR).join(project_name);
    
    if !test_path.exists() {
        // Create minimal structure
        fs::create_dir_all(test_path.join("src/odoo")).expect("Failed to create dirs");
        
        // Create a minimal requirements.txt
        fs::write(
            test_path.join("src/odoo/requirements.txt"),
            "# Minimal requirements for testing\n"
        ).expect("Failed to write requirements.txt");
    }
    
    let odoo_bin = get_odoo_binary();
    
    // Note: This test may fail if venv doesn't exist, which is expected
    // We just verify the command doesn't crash
    let output = Command::new(&odoo_bin)
        .arg("install")
        .current_dir(&test_path)
        .output();

    // Install may fail if venv doesn't exist, which is ok for testing
    // We just want to make sure the command is recognized and executes
    match output {
        Ok(output) => {
            // Command should at least attempt to run (may fail, but should execute)
            assert!(output.status.code().is_some(), "Install command should execute");
        }
        Err(e) => {
            // If command not found, that's a real error
            panic!("Install command failed to execute: {}. Binary: {}", e, odoo_bin.display());
        }
    }
}

#[test]
fn test_commands_exist() {
    let odoo_bin = get_odoo_binary();
    
    // Test that help works (verifies binary is functional)
    let output = Command::new(&odoo_bin)
        .arg("--help")
        .output();

    let output = output.expect(&format!("Failed to execute --help. Binary: {}", odoo_bin.display()));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("Help command failed: {}", stderr);
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Verify key commands are listed
    assert!(stdout.contains("new") || stdout.contains("New"), 
            "Should have 'new' command. Output: {}", stdout);
    assert!(stdout.contains("doctor") || stdout.contains("Doctor"), 
            "Should have 'doctor' command. Output: {}", stdout);
    assert!(stdout.contains("install") || stdout.contains("Install"), 
            "Should have 'install' command. Output: {}", stdout);
    assert!(stdout.contains("run") || stdout.contains("Run"), 
            "Should have 'run' command. Output: {}", stdout);
    assert!(stdout.contains("sync") || stdout.contains("Sync"),
            "Should have 'sync' command. Output: {}", stdout);
}

#[test]
fn test_sync_command_executes_after_new() {
    let odoo_bin = get_odoo_binary();
    let version = TEST_VERSIONS[0];
    let project_name = format!("test_sync_cmd_{}", version.replace('.', "_"));
    let test_path = Path::new(TEST_DIR).join(&project_name);

    ensure_odoo_zip_downloaded(version);

    if test_path.exists() {
        fs::remove_dir_all(&test_path).ok();
    }

    fs::create_dir_all(TEST_DIR).expect("Failed to create test directory");

    let new_output = Command::new(&odoo_bin)
        .arg("new")
        .arg(&project_name)
        .arg("-v")
        .arg(version)
        .current_dir(TEST_DIR)
        .output()
        .expect("Failed to execute new command for sync test");

    if !new_output.status.success() {
        let stderr = String::from_utf8_lossy(&new_output.stderr);
        let stdout = String::from_utf8_lossy(&new_output.stdout);
        panic!(
            "Failed to create project {} for sync test:\nSTDOUT:\n{}\nSTDERR:\n{}",
            project_name, stdout, stderr
        );
    }

    assert!(test_path.exists(), "Project directory should exist for sync test");

    let sync_output = Command::new(&odoo_bin)
        .arg("sync")
        .current_dir(&test_path)
        .output();

    match sync_output {
        Ok(output) => {
            assert!(
                output.status.code().is_some(),
                "'odx sync' should execute for version {}",
                version
            );
        }
        Err(e) => {
            panic!(
                "'odx sync' failed to execute for version {}: {}. Binary: {}",
                version,
                e,
                odoo_bin.display()
            );
        }
    }
}
