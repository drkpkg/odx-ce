use clap::Subcommand;
use crate::utils::{find_project_root, execute_command, find_docker_compose_command};

#[derive(Subcommand)]
pub enum DbCommands {
    /// Start PostgreSQL with Docker Compose
    Start,
    /// Stop PostgreSQL
    Stop,
    /// View PostgreSQL logs
    Logs,
    /// List Odoo databases
    Ls,
    /// Connect to PostgreSQL database
    Psql,
}

pub fn execute(cmd: DbCommands) -> Result<(), String> {
    match cmd {
        DbCommands::Start => start(),
        DbCommands::Stop => stop(),
        DbCommands::Logs => logs(),
        DbCommands::Ls => ls(),
        DbCommands::Psql => psql(),
    }
}

fn start() -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    println!("Starting PostgreSQL...");
    if docker_compose == "docker compose" {
        execute_command("docker", &["compose", "up", "-d", "postgres"], Some(&project_root))?;
    } else {
        execute_command(&docker_compose, &["up", "-d", "postgres"], Some(&project_root))?;
    }
    println!("PostgreSQL is starting. Use 'odoo db logs' to view logs.");
    Ok(())
}

fn stop() -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    println!("Stopping PostgreSQL...");
    if docker_compose == "docker compose" {
        execute_command("docker", &["compose", "down"], Some(&project_root))?;
    } else {
        execute_command(&docker_compose, &["down"], Some(&project_root))?;
    }
    Ok(())
}

fn logs() -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    if docker_compose == "docker compose" {
        execute_command("docker", &["compose", "logs", "-f", "postgres"], Some(&project_root))?;
    } else {
        execute_command(&docker_compose, &["logs", "-f", "postgres"], Some(&project_root))?;
    }
    Ok(())
}

fn ls() -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    println!("Listing Odoo databases...");

    // Query PostgreSQL inside the postgres container for databases typically used by Odoo.
    // This avoids executing any Python scripts from the project scripts directory.
    let query = "SELECT datname FROM pg_database \
WHERE datname NOT IN ('template0','template1','postgres') \
ORDER BY datname;";

    if docker_compose == "docker compose" {
        execute_command(
            "docker",
            &[
                "compose",
                "exec",
                "-T",
                "postgres",
                "psql",
                "-U",
                "odoo",
                "-d",
                "postgres",
                "-c",
                query,
            ],
            Some(&project_root),
        )?;
    } else {
        execute_command(
            &docker_compose,
            &[
                "exec",
                "-T",
                "postgres",
                "psql",
                "-U",
                "odoo",
                "-d",
                "postgres",
                "-c",
                query,
            ],
            Some(&project_root),
        )?;
    }

    Ok(())
}

fn psql() -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    println!("Connecting to PostgreSQL...");
    if docker_compose == "docker compose" {
        execute_command(
            "docker",
            &["compose", "exec", "postgres", "psql", "-U", "odoo", "-d", "postgres"],
            Some(&project_root),
        )?;
    } else {
        execute_command(
            &docker_compose,
            &["exec", "postgres", "psql", "-U", "odoo", "-d", "postgres"],
            Some(&project_root),
        )?;
    }
    Ok(())
}
