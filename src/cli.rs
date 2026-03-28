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

    /// Generate/update i18n translation files
    I18n {
        /// Database name
        #[arg(short, long)]
        database: Option<String>,
        /// Language code (e.g., es_PY) to generate .po file instead of .pot
        #[arg(long)]
        lang: Option<String>,
    },

    /// Run tests (creates temporary database, installs custom_addons modules, runs tests, then deletes database)
    Test {
        /// Test tags (comma-separated or space-separated)
        tags: Vec<String>,
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
            Commands::I18n { database, lang } => {
                commands::i18n::execute(database.as_deref(), lang.as_deref())
            }
            Commands::Test { tags } => commands::test::execute(&tags),
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
