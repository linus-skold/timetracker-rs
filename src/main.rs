use anyhow::Result;

mod cli;
mod storage;
mod time;
mod tui;

use cli::{Cli, Commands};
use clap::Parser;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { description } => cli::start(description),
        Commands::Stop => cli::stop(),
        Commands::Log { description, time } => cli::log(description, time),
        Commands::Today => cli::today(),
        Commands::List => cli::list(),
        Commands::Tui => tui::run_tui(),
        Commands::Status => cli::status(),
    }
}
