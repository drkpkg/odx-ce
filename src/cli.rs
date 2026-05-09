use crate::commands;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "odx")]
#[command(about = "odx: Odoo development CLI", long_about = None)]
pub struct Cli {
    /// Python version to use (e.g. 3.11, 3.12). Default: 3.11. Used by 'new' for the project venv.
    #[arg(global = true, long, default_value = "3.11")]
    pub python: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run Odoo server
    Run,

    /// Update all Odoo modules
    Update {
        /// Database name
        #[arg(short, long)]
        database: String,
    },

    /// Update specific module
    UpdateModule {
        /// Module name
        module: String,
        /// Database name
        #[arg(short, long)]
        database: String,
    },

    /// Open Odoo shell
    Shell {
        /// Database name
        #[arg(short, long)]
        database: String,
    },

    /// Database operations
    #[command(subcommand)]
    Db(commands::db::DbCommands),

    /// Export translations per addon (.pot template or .po for a locale)
    I18n {
        /// PostgreSQL database name (Odoo must be able to connect)
        #[arg(short = 'd', long)]
        database: String,
        /// Single addon technical name (omit to export all project addons under custom_addons and external_addons)
        #[arg(short = 'm', long)]
        module: Option<String>,
        /// Locale for a .po file (e.g. es_BO). Must exist and be active in Settings > Translations > Languages. Omit to write a .pot template using en_US as source language
        #[arg(long)]
        lang: Option<String>,
    },

    /// Run tests (creates temporary database, installs custom_addons modules, runs tests, then deletes database)
    Test {
        /// Test tags (comma-separated or space-separated)
        tags: Vec<String>,
        /// Emit a heartbeat line if no output is produced for N seconds (0 disables)
        #[arg(long, default_value_t = 60)]
        heartbeat_seconds: u64,
        /// Write full output to a log file (defaults to .testing/logs/odx-test-<db>.log)
        #[arg(long)]
        log_file: Option<String>,
        /// Disable log file writing
        #[arg(long, default_value_t = false)]
        no_log_file: bool,
        /// Odoo log level for the test run (e.g. info, warn, error, debug)
        #[arg(long, default_value = "warn")]
        odoo_log_level: String,
    },

    /// Install/update Python dependencies
    Install,

    /// Sync Odoo source (git pull in src/odoo)
    Sync,

    /// Setup development environment
    Setup,

    /// Clean temporary files
    Clean,

    /// Create a new Odoo project
    New {
        /// Project name
        project_name: String,
        /// Odoo version (e.g., 18.0)
        #[arg(short, long)]
        version: String,
        /// Print 'cd <project>' so you can run: eval $(odx new <name> -v <ver> --cd)
        #[arg(long)]
        cd: bool,
    },

    /// Check system requirements and dependencies
    Doctor,
}

impl Cli {
    pub fn run(self) -> Result<(), String> {
        match self.command {
            Commands::Run => commands::run::execute(),
            Commands::Update { database } => commands::update::execute(&database),
            Commands::UpdateModule { module, database } => {
                commands::update_module::execute(&module, &database)
            }
            Commands::Shell { database } => commands::shell::execute(&database),
            Commands::Db(cmd) => commands::db::execute(cmd),
            Commands::I18n {
                database,
                module,
                lang,
            } => commands::i18n::execute(&database, module.as_deref(), lang.as_deref()),
            Commands::Test {
                tags,
                heartbeat_seconds,
                log_file,
                no_log_file,
                odoo_log_level,
            } => commands::test::execute(
                &tags,
                heartbeat_seconds,
                log_file.as_deref(),
                no_log_file,
                &odoo_log_level,
            ),
            Commands::Install => commands::install::execute(),
            Commands::Sync => commands::sync::execute(),
            Commands::Setup => commands::setup::execute(),
            Commands::Clean => commands::clean::execute(),
            Commands::New {
                project_name,
                version,
                cd,
            } => commands::new::execute(&project_name, &version, cd, &self.python),
            Commands::Doctor => commands::doctor::execute(),
        }
    }
}
