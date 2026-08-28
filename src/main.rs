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

    // The preamble runs before the dispatch table, not inside it, so the
    // commands that must not take the store lock skip it — and with it the
    // notice, which they have never printed.
    if needs_store_preamble(&cli.command) {
        if let Some(version) = &update_notice
            && !matches!(cli.command, Commands::Tui)
        {
            eprintln!(
                "Note: tt {version} is available (you have {}). Run `{}` to upgrade.",
                env!("CARGO_PKG_VERSION"),
                update::update_hint()
            );
        }

        // Migrate once under the store lock, never in `load_data`: a write from
        // a loader would surprise every read-only path.
        storage::with_data(|data| {
            tracker::migrate(data);
            Ok(())
        })?;
    }

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
        Commands::Update { check, yes } => commands::update(check, yes),
        Commands::Completions { shell } => commands::completions(shell.as_deref()),
    }
}

/// Whether this command dispatches *after* the store-lock migrate preamble in
/// [`main`]. The rest must not take the store lock: `tt report` migrates its own
/// in-memory copy, and the agent hooks that only touch marks stay off it — see
/// [`cli::AgentCommands::touches_store`].
///
/// Exhaustive on purpose: a new variant must decide.
fn needs_store_preamble(command: &Commands) -> bool {
    match command {
        Commands::Report { .. } | Commands::Update { .. } | Commands::Completions { .. } => false,
        Commands::Agent { command } => command.touches_store(),
        Commands::Start { .. }
        | Commands::Stop
        | Commands::Log { .. }
        | Commands::Today
        | Commands::List { .. }
        | Commands::Tui
        | Commands::Status
        | Commands::Active => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn command_of(args: &[&str]) -> Commands {
        Cli::try_parse_from(args).expect("args parse").command
    }

    #[test]
    fn the_store_locking_commands_take_the_preamble() {
        for args in [
            vec!["tt", "start", "x"],
            vec!["tt", "stop"],
            vec!["tt", "log", "-d", "x", "-t", "5m"],
            vec!["tt", "today"],
            vec!["tt", "list"],
            vec!["tt", "tui"],
            vec!["tt", "status"],
            vec!["tt", "active"],
        ] {
            assert!(
                needs_store_preamble(&command_of(&args)),
                "{args:?} should migrate under the lock"
            );
        }
    }

    #[test]
    fn report_update_and_completions_skip_the_preamble() {
        for args in [
            vec!["tt", "report", "--week"],
            vec!["tt", "update", "--check"],
            vec!["tt", "completions", "bash"],
        ] {
            assert!(
                !needs_store_preamble(&command_of(&args)),
                "{args:?} must not take the store lock"
            );
        }
    }

    /// The agent layer decides per subcommand, through `touches_store`.
    #[test]
    fn agent_follows_touches_store() {
        for args in [
            vec!["tt", "agent", "begin", "p", "-", "impl"],
            vec!["tt", "agent", "touch", "p", "-", "impl"],
            vec!["tt", "agent", "cancel", "p", "-", "impl"],
            vec!["tt", "agent", "list"],
            vec!["tt", "agent", "audit"],
        ] {
            assert!(!needs_store_preamble(&command_of(&args)), "{args:?}");
        }
        for args in [
            vec!["tt", "agent", "end", "p", "-", "impl"],
            vec!["tt", "agent", "item", "p", "-", "impl"],
            vec!["tt", "agent", "audit", "--auto-log"],
        ] {
            assert!(needs_store_preamble(&command_of(&args)), "{args:?}");
        }
    }
}
