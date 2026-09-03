//! The clap surface: what `tt`'s arguments are. The command implementations
//! they dispatch to live in `src/commands.rs`.

use chrono::{Duration, Local, NaiveDate, NaiveTime, TimeZone};
use clap::builder::PossibleValuesParser;
use clap::{Parser, Subcommand};
use clap_complete::ArgValueCandidates;

use crate::completions;
use crate::tracker::IdleInterval;

#[derive(Parser)]
#[command(name = "tt", about = "Simple time tracking CLI", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start tracking a new task
    Start {
        /// Description of the task
        #[arg(required = true)]
        description: Vec<String>,
        /// Project this entry belongs to
        #[arg(short = 'p', long, add = ArgValueCandidates::new(completions::projects))]
        project: Option<String>,
        /// Back-date the start to a time of day today, like "9.30", "9:30",
        /// "0930" or "9". A time still to come today is a usage error.
        #[arg(short = 's', long = "start", value_name = "TIME", value_parser = parse_clock)]
        started_at: Option<NaiveTime>,
    },
    /// Stop the current active task
    Stop,
    /// Log a completed task with a specific duration
    Log {
        /// Description of the task
        #[arg(short = 'd', long)]
        description: String,
        /// Duration in format like "1h30m", "45m", "2h", or bare minutes "90"
        #[arg(short = 't', long, value_parser = parse_duration)]
        time: Duration,
        /// Comma-separated tags (e.g. tagA,tagB,tagC)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Project this entry belongs to
        #[arg(long, add = ArgValueCandidates::new(completions::projects))]
        project: Option<String>,
        /// A silent stretch inside the logged span, as epoch seconds
        /// `<start>-<end>`. Repeatable; records the interval without changing the
        /// logged duration.
        #[arg(long, value_parser = parse_idle)]
        idle: Vec<IdleInterval>,
        /// Trim the recorded idle stretches out of the entry, splitting it into the
        /// pieces between them. Destructive and unconfirmed; requires `--idle`, so
        /// asking for a trim with nothing to trim is a usage error rather than a
        /// silent no-op.
        #[arg(long, requires = "idle")]
        trim: bool,
    },
    /// Show all entries for today
    Today,
    /// Show all entries
    List {
        /// Maximum number of entries to show (defaults to the `list.default_limit`
        /// config value, or 20 if unset)
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },
    /// Open interactive TUI
    Tui,
    /// Show current status
    Status,
    /// true/false if something is active
    Active,
    /// Roll up logged time by project and item
    Report {
        /// Every entry, with no date bound
        #[arg(long, group = "scope")]
        all: bool,
        /// This week, from Monday
        #[arg(long, group = "scope")]
        week: bool,
        /// From this date onwards (YYYY-MM-DD)
        #[arg(long, group = "scope")]
        since: Option<NaiveDate>,
        /// Up to and including this date (YYYY-MM-DD). Narrows a scope, so one of
        /// --all/--week/--since is required alongside it: on its own it would have
        /// nothing to narrow but the single default day.
        #[arg(long, requires = "scope")]
        until: Option<NaiveDate>,
        /// Only entries whose project field is this
        #[arg(long, add = ArgValueCandidates::new(completions::projects))]
        project: Option<String>,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
    /// Phase marks for the agent layer
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Check for and install a newer release
    Update {
        /// Only check for a newer version; don't install it
        #[arg(long)]
        check: bool,
        /// Skip the confirmation prompt before replacing the binary
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Print the shell completion hook, for `eval "$(tt completions zsh)"`
    Completions {
        /// Detected from $SHELL when omitted
        #[arg(value_parser = PossibleValuesParser::new(completions::SHELLS.names()))]
        shell: Option<String>,
    },
}

impl Commands {
    /// Whether this command should trigger the passive startup update check.
    /// `Active` (shell-prompt integration) and `Agent` (the tt-time-logging
    /// hook contract, invoked on every phase begin/touch/end) must stay fast
    /// and silent; `Update` would be a redundant check right before a real
    /// one.
    pub fn wants_update_check(&self) -> bool {
        !matches!(
            self,
            Commands::Update { .. }
                | Commands::Active
                | Commands::Agent { .. }
                | Commands::Completions { .. }
        )
    }
}

/// The agent layer's commands. Most touch mark files only; the ones that log an
/// entry say so through [`AgentCommands::touches_store`].
#[derive(Subcommand)]
pub enum AgentCommands {
    /// Open a mark for a phase, keeping the start of one already open
    Begin {
        #[arg(add = ArgValueCandidates::new(completions::projects))]
        project: String,
        /// Issue number, or `-` for a phase with no issue
        #[arg(add = ArgValueCandidates::new(completions::issues))]
        issue: String,
        #[arg(add = ArgValueCandidates::new(completions::phases))]
        phase: String,
    },
    /// Record one heartbeat for an open phase
    Touch {
        #[arg(add = ArgValueCandidates::new(completions::projects))]
        project: String,
        /// Issue number, or `-` for a phase with no issue
        #[arg(add = ArgValueCandidates::new(completions::issues))]
        issue: String,
        #[arg(add = ArgValueCandidates::new(completions::phases))]
        phase: String,
    },
    /// Drop a phase's mark and heartbeats without logging anything
    Cancel {
        #[arg(add = ArgValueCandidates::new(completions::projects))]
        project: String,
        /// Issue number, or `-` for a phase with no issue
        #[arg(add = ArgValueCandidates::new(completions::issues))]
        issue: String,
        #[arg(add = ArgValueCandidates::new(completions::phases))]
        phase: String,
    },
    /// List every open mark
    List,
    /// Log one finished piece of work for a **known** duration, with no mark
    /// involved. A fallback for when there was nothing to `begin`/`end`
    /// around (the duration is already known some other way) — prefer
    /// `begin`/`touch`/`end` whenever the work can be marked as it happens,
    /// so the logged span is measured, not guessed.
    Item {
        #[arg(add = ArgValueCandidates::new(completions::projects))]
        project: String,
        /// Issue number, or `-` for a phase with no issue
        #[arg(add = ArgValueCandidates::new(completions::issues))]
        issue: String,
        #[arg(add = ArgValueCandidates::new(completions::phases))]
        phase: String,
        /// 3-6 words of plain prose, with no issue number in them
        summary: Option<String>,
        /// Whole minutes, rounded up to the nearest 5 minutes
        minutes: Option<String>,
    },
    /// Close a marked phase, measuring it to its last heartbeat
    End {
        #[arg(add = ArgValueCandidates::new(completions::projects))]
        project: String,
        /// Issue number, or `-` for a phase with no issue
        #[arg(add = ArgValueCandidates::new(completions::issues))]
        issue: String,
        #[arg(add = ArgValueCandidates::new(completions::phases))]
        phase: String,
        /// 3-6 words of plain prose, with no issue number in them
        summary: Option<String>,
        /// Whole minutes, overriding the mark's own timestamps entirely — and
        /// winning over both flags below
        minutes: Option<String>,
        /// Log the whole measured span, recording the flagged silence without
        /// removing it
        #[arg(long, conflicts_with = "trim")]
        full: bool,
        /// Log the measured span minus every flagged gap, splitting the entry at
        /// each one
        #[arg(long)]
        trim: bool,
    },
    /// Hook-only activity ledger, hidden from `--help` — never called by the
    /// model. See docs/decisions/0001-agent-activity-tracking.md.
    #[command(hide = true, subcommand)]
    Activity(ActivityCommands),
    /// Reconcile the activity ledger against marks and logged entries,
    /// reporting activity with no evidence it was ever tracked.
    Audit {
        /// Write a fixed-phase `#auto` entry for every window that has also
        /// passed `agent.auto_log_after_minutes`. A no-op unless that setting
        /// is configured — see
        /// docs/decisions/0002-auto-logging-unaccounted-activity.md.
        #[arg(long)]
        auto_log: bool,
    },
}

/// See [`AgentCommands::Activity`]. Keyed by Claude Code's own session id.
#[derive(Subcommand)]
pub enum ActivityCommands {
    /// SessionStart: open this session's activity window.
    Begin {
        session_id: String,
        project: Option<String>,
    },
    /// Stop: close this session's activity window.
    End { session_id: String },
    /// SubagentStop: record that one subagent dispatch finished.
    Subagent { session_id: String },
    /// Stop: report this one session's window if it is unaccounted for,
    /// silent otherwise. Same reconciliation as `tt agent audit`, narrowed
    /// to a single session so the Stop hook can warn immediately.
    Check {
        session_id: String,
        /// Also auto-log this session's own unaccounted window, per
        /// `agent.auto_log_on_stop` — a no-op unless that setting is
        /// configured. See docs/decisions/0003-auto-log-on-stop.md.
        #[arg(long)]
        auto_log: bool,
    },
}

impl ActivityCommands {
    /// Whether this subcommand may write a `tt` entry. Only `Check
    /// --auto-log` can, and only when `agent.auto_log_on_stop` is also
    /// configured — but that config read happens too late for dispatch
    /// ordering, so this stays conservative: `--auto-log` alone routes
    /// through the store-locking preamble, whether or not it ends up writing.
    pub fn touches_store(&self) -> bool {
        matches!(self, ActivityCommands::Check { auto_log: true, .. })
    }
}

impl AgentCommands {
    /// Whether this subcommand creates or reads a `tt` entry, which decides
    /// which side of the migrate preamble in `main.rs` it dispatches on.
    ///
    /// Exhaustive on purpose: a new subcommand must decide.
    pub fn touches_store(&self) -> bool {
        match self {
            AgentCommands::Begin { .. }
            | AgentCommands::Touch { .. }
            | AgentCommands::Cancel { .. }
            | AgentCommands::List => false,
            AgentCommands::Activity(command) => command.touches_store(),
            // Only `--auto-log` actually writes; a plain audit stays on the
            // fast, no-preamble path like `list` and `report`.
            AgentCommands::Audit { auto_log } => *auto_log,
            AgentCommands::Item { .. } | AgentCommands::End { .. } => true,
        }
    }
}

/// Parse one `-t/--time` value. A malformed duration is a usage error, never a
/// zero-length or partially honoured entry.
fn parse_duration(value: &str) -> Result<Duration, String> {
    crate::duration::parse(value).ok_or_else(|| {
        format!(
            "expected a duration like `1h30m`, `45m`, `2h` or bare minutes `90`, got `{}`",
            value
        )
    })
}

/// Parse one `-s/--start` value: a time of day today, written the way a person
/// says it — `9.30`, `9:30`, `0930` or a bare `9`. The date is not part of it;
/// `commands::start` resolves it against today and rejects a future time.
fn parse_clock(value: &str) -> Result<NaiveTime, String> {
    let malformed = || {
        format!(
            "expected a time of day like `9.30`, `9:30`, `0930` or `9`, got `{}`",
            value
        )
    };
    let raw = value.trim();
    let (hour, minute) = match raw.split_once([':', '.']) {
        Some((hour, minute)) => (hour, minute),
        // No separator: `HMM`/`HHMM` is a clock, anything shorter is a bare hour.
        None if raw.len() > 2 => raw.split_at(raw.len() - 2),
        None => (raw, "0"),
    };
    let field = |raw: &str| raw.parse::<u32>().map_err(|_| malformed());
    NaiveTime::from_hms_opt(field(hour)?, field(minute)?, 0).ok_or_else(|| {
        format!(
            "`{}` is not a time on the clock — hours are 0-23, minutes 0-59",
            value
        )
    })
}

/// Parse one `--idle` value: `<start>-<end>` in epoch seconds. A malformed value
/// is an error, never a silently dropped interval.
fn parse_idle(value: &str) -> Result<IdleInterval, String> {
    let (start, end) = value
        .split_once('-')
        .ok_or_else(|| format!("expected <start>-<end> epoch seconds, got `{}`", value))?;
    let stamp = |field: &str, raw: &str| {
        raw.trim()
            .parse::<i64>()
            .map_err(|_| format!("{} is not epoch seconds: `{}`", field, raw))
            .and_then(|secs| {
                Local
                    .timestamp_opt(secs, 0)
                    .single()
                    .ok_or_else(|| format!("{} is not a valid timestamp: `{}`", field, raw))
            })
    };
    let start = stamp("start", start)?;
    let end = stamp("end", end)?;
    if end < start {
        return Err(format!("idle interval ends before it starts: `{}`", value));
    }
    Ok(IdleInterval::new(start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// `--until` alone is a usage error, not a silently discarded bound.
    #[test]
    fn until_requires_a_scope_to_narrow() {
        assert!(
            Cli::try_parse_from(["tt", "report", "--until", "2026-08-05"]).is_err(),
            "--until alone is a usage error"
        );
        assert!(
            Cli::try_parse_from([
                "tt",
                "report",
                "--since",
                "2026-08-01",
                "--until",
                "2026-08-05"
            ])
            .is_ok(),
            "--until narrows a scope that exists"
        );
    }

    #[test]
    fn the_scope_selectors_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["tt", "report", "--week", "--all"]).is_err(),
            "two scopes contradict each other"
        );
        assert!(Cli::try_parse_from(["tt", "report", "--week"]).is_ok());
        assert!(Cli::try_parse_from(["tt", "report", "--all"]).is_ok());
    }

    #[test]
    fn a_malformed_date_is_rejected_rather_than_ignored() {
        assert!(Cli::try_parse_from(["tt", "report", "--since", "last tuesday"]).is_err());
        assert!(Cli::try_parse_from(["tt", "report", "--since", "2026-13-01"]).is_err());
    }

    #[test]
    fn report_takes_a_project_filter_and_a_json_flag_in_any_scope() {
        assert!(
            Cli::try_parse_from(["tt", "report", "--project", "vinge", "--json"]).is_ok(),
            "neither belongs to the scope group"
        );
    }

    #[test]
    fn a_malformed_idle_value_is_a_parse_error_not_a_dropped_interval() {
        for bad in ["notanumber", "100", "100-", "abc-200", "100-abc", "300-200"] {
            let parsed = Cli::try_parse_from([
                "tt",
                "log",
                "-d",
                "x",
                "-t",
                "5m",
                &format!("--idle={}", bad),
            ]);
            assert!(parsed.is_err(), "`{bad}` parsed instead of failing");
        }
    }

    /// #41: `-t` refuses what it cannot parse instead of logging zero.
    #[test]
    fn a_malformed_time_value_is_a_parse_error_not_a_zero_entry() {
        for bad in ["garbage", "1x30", "-45m", "1h30", "1.5h", "90 minutes", ""] {
            let parsed = Cli::try_parse_from(["tt", "log", "-d", "x", "-t", bad]);
            assert!(parsed.is_err(), "`{bad}` parsed instead of failing");
        }
    }

    #[test]
    fn the_documented_time_forms_all_parse() {
        for (raw, expected) in [
            ("1h30m", Duration::minutes(90)),
            ("45m", Duration::minutes(45)),
            ("2h", Duration::hours(2)),
            ("90", Duration::minutes(90)),
        ] {
            let cli = Cli::try_parse_from(["tt", "log", "-d", "x", "-t", raw])
                .unwrap_or_else(|e| panic!("`{raw}` failed to parse: {e}"));
            match cli.command {
                Commands::Log { time, .. } => assert_eq!(time, expected, "`{raw}`"),
                _ => panic!("not a log command"),
            }
        }
    }

    #[test]
    fn trim_without_idle_is_a_usage_error() {
        let parsed = Cli::try_parse_from(["tt", "log", "-d", "x", "-t", "5m", "--trim"]);
        assert!(
            parsed.is_err(),
            "--trim alone parsed, so it would be a silent no-op"
        );
    }

    #[test]
    fn a_well_formed_idle_value_parses_to_the_epoch_seconds_given() {
        let interval = parse_idle("1700000000-1700000600").unwrap();
        assert_eq!(interval.start.timestamp(), 1_700_000_000);
        assert_eq!(interval.end.timestamp(), 1_700_000_600);
        assert_eq!(interval.duration(), chrono::Duration::minutes(10));
    }
    /// The short forms are the point of the flags: `-p` and `-s` mean the same
    /// as `--project` and `--start`.
    #[test]
    fn start_takes_short_forms_for_project_and_start_time() {
        let parsed = Cli::try_parse_from(["tt", "start", "-p", "tt", "-s", "9.30", "writing"])
            .expect("short flags")
            .command;
        match parsed {
            Commands::Start {
                description,
                project,
                started_at,
            } => {
                assert_eq!(description, vec!["writing".to_string()]);
                assert_eq!(project.as_deref(), Some("tt"));
                assert_eq!(started_at, NaiveTime::from_hms_opt(9, 30, 0));
            }
            _ => panic!("not a start command"),
        }
    }

    #[test]
    fn a_clock_time_is_read_the_way_a_person_writes_one() {
        let at = |h, m| Ok(NaiveTime::from_hms_opt(h, m, 0).unwrap());
        assert_eq!(parse_clock("9.30"), at(9, 30));
        assert_eq!(parse_clock("9:30"), at(9, 30));
        assert_eq!(parse_clock("930"), at(9, 30));
        assert_eq!(parse_clock("0930"), at(9, 30));
        assert_eq!(parse_clock("9"), at(9, 0));
        assert_eq!(parse_clock("13:05"), at(13, 5));
    }

    #[test]
    fn a_time_that_is_not_on_the_clock_is_a_usage_error() {
        assert!(parse_clock("25:00").is_err(), "no 25th hour");
        assert!(parse_clock("9:70").is_err(), "no 70th minute");
        assert!(parse_clock("half nine").is_err());
        assert!(parse_clock("").is_err());
    }
}
