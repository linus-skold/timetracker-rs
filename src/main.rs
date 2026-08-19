use anyhow::Result;

mod agent;
mod cli;
mod config;
mod duration;
mod icons;
mod marks;
mod report;
mod storage;
mod tracker;
mod tui;

use cli::{Cli, Commands};
use clap::Parser;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Dispatched ahead of the preamble below because they must not take the store
    // lock; the preamble runs before the dispatch table, not inside it. `tt report`
    // migrates its own in-memory copy instead — see `AgentCommands::touches_store`.
    match &cli.command {
        Commands::Agent { command } if !command.touches_store() => {
            return agent::run(command);
        }
        Commands::Report {
            all,
            week,
            since,
            until,
            project,
            json,
        } => {
            return cli::report(*all, *week, *since, *until, project.clone(), *json);
        }
        _ => {}
    }

    // Migrate once under the store lock, never in `load_data`: a write from a
    // loader would surprise every read-only path.
    storage::with_data(|data| {
        tracker::migrate(data);
        Ok(())
    })?;

    match cli.command {
        Commands::Start {
            description,
            project,
        } => cli::start(description, project),
        Commands::Stop => cli::stop(),
        Commands::Log {
            description,
            time,
            tags,
            project,
            idle,
            trim,
        } => cli::log(description, time, tags, project, idle, trim, None),
        Commands::Today => cli::today(),
        Commands::Report {
            all,
            week,
            since,
            until,
            project,
            json,
        } => cli::report(all, week, since, until, project, json),
        Commands::List { limit } => cli::list(limit),
        Commands::Tui => tui::run_tui(),
        Commands::Status => cli::status(),
        Commands::Active => cli::active(),
        Commands::Agent { command } => agent::run(&command),
    }
}
