use clap::Parser;
use odoo_cli::cli::Cli;

fn main() {
    let cli = Cli::parse();

    if cli.run().is_err() {
        std::process::exit(1);
    }
}
