use anyhow::Result;

mod activity;
mod agent;
mod audit;
mod cli;
mod commands;
mod completions;
mod config;
mod duration;
mod icons;
mod marks;
mod paths;
mod report;
mod storage;
mod time;
mod tracker;
mod tui;
mod update;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};

fn main() -> Result<()> {
    // Exits the process on a completion request, so nothing below — including
    // the store-lock migration — runs on a Tab press.
    clap_complete::CompleteEnv::with_factory(Cli::command)
        .shells(completions::SHELLS)
        .complete();
    let cli = Cli::parse();

    // At most once per day, bounded to a couple of seconds — see
    // `update::maybe_check_and_notify`. Skipped entirely for commands that
    // must stay fast and script-friendly (`Active`, `Agent`) or that are
    // themselves an update check (`Update`).
    let update_notice = cli
        .command
        .wants_update_check()
        .then(|| update::maybe_check_and_notify(env!("CARGO_PKG_VERSION")))
        .flatten();

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
            return commands::report(commands::ReportRequest {
                all: *all,
                week: *week,
                since: *since,
                until: *until,
                project: project.clone(),
                json: *json,
            });
        }
        Commands::Update { check, yes } => {
            return commands::update(*check, *yes);
        }
        Commands::Completions { shell } => {
            return commands::completions(shell.as_deref());
        }
        _ => {}
    }

    if let Some(version) = &update_notice
        && !matches!(cli.command, Commands::Tui)
    {
        eprintln!(
            "Note: tt {version} is available (you have {}). Run `{}` to upgrade.",
            env!("CARGO_PKG_VERSION"),
            update::update_hint()
        );
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
        } => commands::start(description, project),
        Commands::Stop => commands::stop(),
        Commands::Log {
            description,
            time,
            tags,
            project,
            idle,
            trim,
        } => commands::log(commands::LogRequest {
            description,
            time,
            extra_tags: tags,
            project,
            idle,
            trim,
            ended_at: None,
        }),
        Commands::Today => commands::today(),
        Commands::Report {
            all,
            week,
            since,
            until,
            project,
            json,
        } => commands::report(commands::ReportRequest {
            all,
            week,
            since,
            until,
            project,
            json,
        }),
        Commands::List { limit } => commands::list(limit),
        Commands::Tui => tui::run_tui(update_notice),
        Commands::Status => commands::status(),
        Commands::Active => commands::active(),
        Commands::Agent { command } => agent::run(&command),
        // Dispatched ahead of the preamble above; unreachable in practice,
        // kept for exhaustiveness (same shape as `Report` just above it).
        Commands::Update { check, yes } => commands::update(check, yes),
        Commands::Completions { shell } => commands::completions(shell.as_deref()),
    }
}
