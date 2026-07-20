use crate::commands;
use crate::ui::{ColorMode, Ui, UiConfig};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "odx")]
#[command(about = "odx: Odoo development CLI", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Python version to use (e.g. 3.11, 3.12). Default: 3.11. Used by 'new' for the project venv.
    #[arg(global = true, long, default_value = "3.11")]
    pub python: String,

    /// Output format suitable for scripts/CI (disables colors, spinners and prompts)
    #[arg(global = true, long, default_value_t = false)]
    pub json: bool,

    /// Reduce output (keeps errors)
    #[arg(global = true, long, default_value_t = false)]
    pub quiet: bool,

    /// Control colored output
    #[arg(global = true, long, value_enum, default_value_t = CliColor::Auto)]
    pub color: CliColor,

    /// Disable progress indicators (spinners/progress bars)
    #[arg(global = true, long, default_value_t = false)]
    pub no_progress: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliColor {
    Auto,
    Always,
    Never,
}

impl From<CliColor> for ColorMode {
    fn from(v: CliColor) -> Self {
        match v {
            CliColor::Auto => ColorMode::Auto,
            CliColor::Always => ColorMode::Always,
            CliColor::Never => ColorMode::Never,
        }
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run Odoo server
    Run {
        /// Skip the live log dashboard and stream plain (level-colored) log lines instead
        #[arg(long, default_value_t = false)]
        plain: bool,
    },

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

    /// Run tests (creates temporary database, installs modules with --test-enable, runs filtered tests via --test-tags in one odoo-bin run, then deletes database)
    Test {
        /// Odoo --test-tags specs (comma-separated or space-separated; passed once to odoo-bin). If a spec includes /module_name, only that module is installed; otherwise all custom_addons modules are installed.
        tags: Vec<String>,
        /// Emit a heartbeat line if no output is produced for N seconds (0 disables)
        #[arg(long, default_value_t = 60)]
        heartbeat_seconds: u64,
        /// Override combined log path (default: .testing/sessions/<run_id>/combined.log)
        #[arg(long)]
        log_file: Option<String>,
        /// Disable all .testing session artifacts (logs, report.json)
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
        let ui = Ui::new(UiConfig {
            color: self.color.into(),
            quiet: self.quiet,
            json: self.json,
            progress: !self.no_progress,
        });

        let result = match self.command {
            Commands::Run { plain } => commands::run::execute(&ui, plain),
            Commands::Update { database } => commands::update::execute(&ui, &database),
            Commands::UpdateModule { module, database } => {
                commands::update_module::execute(&ui, &module, &database)
            }
            Commands::Shell { database } => commands::shell::execute(&ui, &database),
            Commands::Db(cmd) => commands::db::execute(&ui, cmd),
            Commands::I18n {
                database,
                module,
                lang,
            } => commands::i18n::execute(&ui, &database, module.as_deref(), lang.as_deref()),
            Commands::Test {
                tags,
                heartbeat_seconds,
                log_file,
                no_log_file,
                odoo_log_level,
            } => commands::test::execute(
                &ui,
                &tags,
                heartbeat_seconds,
                log_file.as_deref(),
                no_log_file,
                &odoo_log_level,
            ),
            Commands::Install => commands::install::execute(&ui),
            Commands::Sync => commands::sync::execute(&ui),
            Commands::Clean => commands::clean::execute(&ui),
            Commands::New {
                project_name,
                version,
                cd,
            } => commands::new::execute(&ui, &project_name, &version, cd, &self.python),
            Commands::Doctor => commands::doctor::execute(&ui),
        };

        if let Err(ref e) = result {
            ui.error(format!("Error: {}", e));
        }

        result
    }
}
