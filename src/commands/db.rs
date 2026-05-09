use clap::Subcommand;
use crate::utils::{find_project_root, execute_command, find_docker_compose_command};
use crate::ui::Ui;

fn validate_db_name(db: &str) -> Result<(), String> {
    if db.is_empty() {
        return Err("Database name cannot be empty".to_string());
    }
    // Conservative validation to avoid passing arbitrary values to subprocesses.
    // Supports typical Odoo DB names like `my_db`, `test_odoo_123`.
    let ok = db
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(format!(
            "Invalid database name '{}'. Allowed characters: letters, numbers, underscore (_)",
            db
        ));
    }
    Ok(())
}

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
    /// Drop a PostgreSQL database
    Drop {
        /// Database name
        database: String,
        /// Terminate active connections and drop anyway
        #[arg(long)]
        force: bool,
        /// Do not error if the database does not exist
        #[arg(long)]
        if_exists: bool,
    },
}

pub fn execute(ui: &Ui, cmd: DbCommands) -> Result<(), String> {
    match cmd {
        DbCommands::Start => start(ui),
        DbCommands::Stop => stop(ui),
        DbCommands::Logs => logs(ui),
        DbCommands::Ls => ls(ui),
        DbCommands::Psql => psql(ui),
        DbCommands::Drop {
            database,
            force,
            if_exists,
        } => drop_db(ui, &database, force, if_exists),
    }
}

pub(crate) fn drop_db(ui: &Ui, database: &str, force: bool, if_exists: bool) -> Result<(), String> {
    validate_db_name(database)?;

    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    if !force {
        ui.warn(format!("About to drop database: {}", database));
        let ok = ui.prompt_confirm("Continue?", false)?;
        if !ok {
            return Err("Canceled".to_string());
        }
    } else {
        ui.info(format!("Dropping database: {}", database));
    }

    let mut args: Vec<String> = Vec::new();
    if docker_compose == "docker compose" {
        args.extend(["compose", "exec", "-T", "postgres"].map(|s| s.to_string()));
    } else {
        args.extend(["exec", "-T", "postgres"].map(|s| s.to_string()));
    }

    args.push("dropdb".to_string());

    if force {
        args.push("--force".to_string());
    }
    if if_exists {
        args.push("--if-exists".to_string());
    }

    args.push("-U".to_string());
    args.push("odoo".to_string());
    args.push(database.to_string());

    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    if docker_compose == "docker compose" {
        execute_command("docker", &args_refs, Some(&project_root))?;
    } else {
        execute_command(&docker_compose, &args_refs, Some(&project_root))?;
    }

    Ok(())
}

fn start(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    let _sp = ui.spinner("Starting PostgreSQL...");
    if docker_compose == "docker compose" {
        execute_command("docker", &["compose", "up", "-d", "postgres"], Some(&project_root))?;
    } else {
        execute_command(&docker_compose, &["up", "-d", "postgres"], Some(&project_root))?;
    }
    ui.success("PostgreSQL is starting. Use 'odx db logs' to view logs.");
    Ok(())
}

fn stop(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    let _sp = ui.spinner("Stopping PostgreSQL...");
    if docker_compose == "docker compose" {
        execute_command("docker", &["compose", "down"], Some(&project_root))?;
    } else {
        execute_command(&docker_compose, &["down"], Some(&project_root))?;
    }
    Ok(())
}

fn logs(_ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    if docker_compose == "docker compose" {
        execute_command("docker", &["compose", "logs", "-f", "postgres"], Some(&project_root))?;
    } else {
        execute_command(&docker_compose, &["logs", "-f", "postgres"], Some(&project_root))?;
    }
    Ok(())
}

fn ls(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    ui.heading("Listing Odoo databases...");

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

fn psql(ui: &Ui) -> Result<(), String> {
    let project_root = find_project_root()?;
    let docker_compose = find_docker_compose_command()?;

    ui.info("Connecting to PostgreSQL...");
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
