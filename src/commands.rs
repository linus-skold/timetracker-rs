//! The command implementations behind the clap surface in `src/cli.rs`.
//! `cli.rs` defines what the arguments are; this module is what they do.

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime};

use crate::completions;
use crate::config;
use crate::duration;
use crate::icons;
use crate::report;
use crate::storage::{load_data, with_data};
use crate::tracker::{IdleInterval, TimeEntry, format_tags, parse_tags};

/// Print the completion hook for `eval` at shell startup. The hook embeds this
/// binary's absolute path, so it is regenerated on every startup, never saved;
/// nu is the exception (see `completions::Nu`).
pub fn completions(shell: Option<&str>) -> Result<()> {
    let from_env = std::env::var_os("SHELL").and_then(|s| {
        std::path::Path::new(&s)
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
    });
    let completer = shell
        .map(str::to_string)
        .or(from_env)
        .or_else(|| cfg!(windows).then(|| "powershell".to_string()))
        .and_then(|n| completions::SHELLS.completer(&n))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not detect the shell from $SHELL; pass one of: {}",
                completions::SHELLS.names().collect::<Vec<_>>().join(", ")
            )
        })?;
    let exe = std::env::current_exe()?;
    completer.write_registration(
        "COMPLETE",
        "tt",
        "tt",
        &exe.to_string_lossy(),
        &mut std::io::stdout(),
    )?;
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        let name = completer.name();
        // install.sh and install.ps1 print copies of this table.
        let line = match name {
            "fish" => "tt completions fish | source   # ~/.config/fish/config.fish".to_string(),
            "powershell" => {
                "tt completions powershell | Out-String | Invoke-Expression   # $PROFILE".to_string()
            }
            "elvish" => "eval (tt completions elvish | slurp)   # ~/.config/elvish/rc.elv".to_string(),
            "nu" => "tt completions nu | save -f ($nu.user-autoload-dirs.0 | path join tt-completer.nu)   # run once".to_string(),
            _ => format!("eval \"$(tt completions {name})\"   # ~/.{name}rc"),
        };
        eprintln!(
            "\nTo enable completion, run this once or add it to your shell startup file:\n  {line}"
        );
    }
    Ok(())
}

/// Bracketed, space-prefixed tag display for println! output, e.g. " [#a #b]",
/// or "" when there are no tags.
fn tags_display(tags: &[String]) -> String {
    if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", format_tags(tags))
    }
}

/// Render a project for a single-line listing: ` (demo)`, or nothing when unset.
/// The project is a field, never a tag, so it prints separately from the tags.
fn project_display(project: Option<&String>) -> String {
    match project {
        Some(p) => format!(" ({})", p),
        None => String::new(),
    }
}

/// One entry as `today` and `list` print it. They differ only in the date
/// column, so `with_date` is the whole difference between the two listings.
fn entry_line(entry: &TimeEntry, with_date: bool) -> String {
    let status = if entry.is_active() {
        entry.status_icon()
    } else {
        "  "
    };
    let date = if with_date {
        format!("{} ", entry.start_time.format("%Y-%m-%d"))
    } else {
        String::new()
    };
    format!(
        "{}{}{} - {}{}{} ({})",
        status,
        date,
        entry.start_time.format("%H:%M"),
        entry.description,
        project_display(entry.project.as_ref()),
        tags_display(&entry.tags),
        entry.format_duration()
    )
}

/// Resolve a `-s/--start` time of day against the day `now` falls on.
///
/// The flag exists to back-date a task that was already underway, so a time
/// still to come is a mistake worth refusing rather than a task that starts in
/// the future and reads as negative until the clock catches up.
fn resolve_start(clock: NaiveTime, now: DateTime<Local>) -> Result<DateTime<Local>> {
    let started = now
        .date_naive()
        .and_time(clock)
        .and_local_timezone(Local)
        .earliest()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} does not exist today — the clock skips it for daylight saving",
                clock.format("%H:%M")
            )
        })?;
    if started > now {
        anyhow::bail!(
            "{} is still to come today; --start back-dates, it does not schedule",
            clock.format("%H:%M")
        );
    }
    Ok(started)
}

pub fn start(
    description: Vec<String>,
    project: Option<String>,
    started_at: Option<NaiveTime>,
) -> Result<()> {
    let raw_desc = description.join(" ");
    let (desc, tags) = parse_tags(&raw_desc);
    let now = Local::now();
    let start_time = match started_at {
        Some(clock) => resolve_start(clock, now)?,
        None => now,
    };

    // One lock for the check and the insert, so two starts cannot both see nothing.
    let already_tracking = with_data(|data| {
        if let Some(active) = data.active_entry() {
            return Ok(Some((active.description.clone(), active.start_time)));
        }
        data.add_entry(
            desc.clone(),
            project.clone(),
            tags.clone(),
            start_time,
            None,
        );
        Ok(None)
    })?;

    if let Some((active_desc, active_start)) = already_tracking {
        println!(
            "{}  Already tracking: \"{}\" (started at {})",
            icons::warning(),
            active_desc,
            active_start.format("%H:%M")
        );
        println!("Stop it first with: tt stop");
        return Ok(());
    }

    println!(
        "{}  Started: \"{}\"{}{} at {}",
        icons::active(),
        desc,
        project_display(project.as_ref()),
        tags_display(&tags),
        start_time.format("%H:%M:%S")
    );
    Ok(())
}

pub fn stop() -> Result<()> {
    let stopped = with_data(|data| {
        let info = data
            .active_entry()
            .map(|e| (e.description.clone(), e.format_duration()));

        if data.stop_active() {
            Ok(info)
        } else {
            Ok(None)
        }
    })?;

    if let Some((desc, dur)) = stopped {
        println!(
            "{}  Stopped: \"{}\" - Duration: {}",
            icons::stopped(),
            desc,
            dur
        );
    } else {
        println!("No active task to stop.");
    }
    Ok(())
}

/// One finished entry to record, as [`log`] takes it.
pub struct LogRequest {
    pub description: String,
    /// How long the entry ran; already parsed, so `log` cannot be handed a
    /// duration nobody validated.
    pub time: Duration,
    /// Tags to add on top of the ones parsed out of the description.
    pub extra_tags: Vec<String>,
    pub project: Option<String>,
    pub idle: Vec<IdleInterval>,
    pub trim: bool,
    /// Pins the timeline; see [`log`].
    pub ended_at: Option<DateTime<Local>>,
}

/// Record a finished entry, back-dated from its end. `ended_at` pins the
/// timeline and must be the mark's last heartbeat whenever `idle` intervals are
/// passed, or the recorded silence lands outside the entry.
pub fn log(request: LogRequest) -> Result<()> {
    let LogRequest {
        description,
        time,
        extra_tags,
        project,
        idle,
        trim,
        ended_at,
    } = request;
    let end_time = ended_at.unwrap_or_else(Local::now);
    let start_time = end_time - time;

    let (desc, mut tags) = parse_tags(&description);
    for tag in extra_tags {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    // Taken out of the closure, so the figure printed is the one written.
    let stored = with_data(|data| {
        let entry_id = data
            .add_entry(
                desc.clone(),
                project.clone(),
                tags.clone(),
                start_time,
                Some(end_time),
            )
            .id;
        if let Some(entry) = data.entries.iter_mut().find(|e| e.id == entry_id) {
            entry.idle = idle;
        }
        // Do not lift this out of the closure: the insert and the split are one
        // store transaction.
        if trim {
            let pieces = data.split_at_idle(entry_id);
            if !pieces.is_empty() {
                return Ok(pieces
                    .iter()
                    .filter_map(|id| data.get_entry(*id))
                    .map(|piece| piece.duration())
                    .sum());
            }
            // An empty vec is `trim_spans` declining, which leaves the entry whole.
        }
        Ok(time)
    })?;

    println!(
        "{} Logged: \"{}\"{}{} - Duration: {}",
        icons::logged(),
        desc,
        project_display(project.as_ref()),
        tags_display(&tags),
        duration::format(stored)
    );
    Ok(())
}

pub fn today() -> Result<()> {
    let data = load_data()?;
    let today_entries = data.today_entries();

    if today_entries.is_empty() {
        println!("No entries for today.");
        return Ok(());
    }

    println!("{} Today's entries:\n", icons::calendar());
    for entry in &today_entries {
        println!("{}", entry_line(entry, false));
    }
    println!("\nTotal: {}", duration::format(data.today_total()));
    Ok(())
}

pub fn list(limit: Option<usize>) -> Result<()> {
    let limit = limit.unwrap_or_else(|| config::load().list.default_limit.unwrap_or(20));
    let data = load_data()?;

    if data.entries.is_empty() {
        println!("No entries yet.");
        return Ok(());
    }

    println!("{} All entries:\n", icons::list());
    for entry in data.entries.iter().rev().take(limit) {
        println!("{}", entry_line(entry, true));
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let data = load_data()?;

    if let Some(active) = data.active_entry() {
        println!(
            "{}  Currently tracking: \"{}\"",
            icons::active(),
            active.description
        );
        if let Some(project) = &active.project {
            println!("   Project: {}", project);
        }
        if !active.tags.is_empty() {
            println!("   Tags: {}", active.format_tags());
        }
        println!("   Started at: {}", active.start_time.format("%H:%M:%S"));
        println!("   Duration: {}", active.format_duration());
    } else {
        println!("No active task. Start one with: tt start <description>");
    }
    Ok(())
}

pub fn active() -> Result<()> {
    let data = load_data()?;

    if data.active_entry().is_some() {
        println!("true");
    } else {
        println!("false");
    }

    Ok(())
}

/// `tt update` — see `src/update.rs` for the actual GitHub Releases lookup,
/// download and self-replacement. Dispatched ahead of the preamble in
/// `main.rs`, same as `report`: it never touches the data store.
pub fn update(check: bool, yes: bool) -> Result<()> {
    crate::update::perform_update(check, yes)
}

/// What `tt report` was asked for: the scope selectors, then the filter and the
/// output format. Mirrors [`crate::cli::Commands::Report`]'s fields.
pub struct ReportRequest {
    pub all: bool,
    pub week: bool,
    pub since: Option<NaiveDate>,
    pub until: Option<NaiveDate>,
    pub project: Option<String>,
    pub json: bool,
}

/// `tt report` — the rollup surface; see `src/report.rs` for the maths. Dispatched
/// ahead of the preamble in `main.rs`, so it migrates its own in-memory copy.
pub fn report(request: ReportRequest) -> Result<()> {
    let ReportRequest {
        all,
        week,
        since,
        until,
        project,
        json,
    } = request;
    let mut data = load_data()?;
    crate::tracker::migrate(&mut data);
    let today = Local::now().date_naive();
    let scope = report::resolve_scope(today, all, week, since, until, project.as_deref());
    let selected = report::select(&data, &scope, project.as_deref());
    let rolled = report::rollup(&selected);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report::to_json(&rolled, &scope.label))?
        );
    } else {
        print!("{}", report::render(&rolled, &scope.label));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use crate::storage;
    use crate::storage::env_guard;
    use crate::storage::env_sandbox as sandbox;
    use chrono::{TimeZone, Timelike};
    use clap::Parser;

    fn parse_log(args: &[&str]) -> Commands {
        let mut argv = vec!["tt", "log"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).expect("log arguments").command
    }

    fn run_log(command: Commands) {
        match command {
            Commands::Log {
                description,
                time,
                tags,
                project,
                idle,
                trim,
            } => log(LogRequest {
                description,
                time,
                extra_tags: tags,
                project,
                idle,
                trim,
                ended_at: None,
            })
            .unwrap(),
            _ => panic!("parse_log produced something other than a Log command"),
        }
    }

    #[test]
    fn idle_intervals_are_recorded_without_changing_the_logged_duration() {
        let _guard = env_guard();
        sandbox("log-idle");
        let start = Local::now().timestamp() - 3600;
        let end = start + 900;

        run_log(parse_log(&[
            "-d",
            "wrote the thing",
            "-t",
            "90m",
            &format!("--idle={}-{}", start, end),
        ]));

        let data = storage::load_data().unwrap();
        assert_eq!(
            data.entries.len(),
            1,
            "--idle alone must not split anything"
        );
        let entry = &data.entries[0];
        assert_eq!(
            entry.duration(),
            chrono::Duration::minutes(90),
            "--idle changed the logged duration"
        );
        assert_eq!(entry.idle.len(), 1);
        assert_eq!(entry.idle[0].start.timestamp(), start);
        assert_eq!(entry.idle[0].end.timestamp(), end);
    }

    #[test]
    fn two_idle_arguments_are_both_recorded_in_order() {
        let _guard = env_guard();
        sandbox("log-idle-twice");
        let base = Local::now().timestamp() - 7200;

        run_log(parse_log(&[
            "-d",
            "long session",
            "-t",
            "2h",
            &format!("--idle={}-{}", base + 600, base + 900),
            &format!("--idle={}-{}", base + 3000, base + 3600),
        ]));

        let data = storage::load_data().unwrap();
        let stamps: Vec<(i64, i64)> = data.entries[0]
            .idle
            .iter()
            .map(|gap| (gap.start.timestamp(), gap.end.timestamp()))
            .collect();
        assert_eq!(
            stamps,
            vec![(base + 600, base + 900), (base + 3000, base + 3600)]
        );
    }

    #[test]
    fn trim_splits_the_logged_entry_in_the_same_command() {
        let _guard = env_guard();
        let dir = sandbox("log-trim");
        // Two holes inside a two-hour span, so a correct trim leaves three pieces.
        let end = Local::now().timestamp();
        let start = end - 7200;
        let gaps = [(start + 600, start + 1500), (start + 4000, start + 5800)];

        run_log(parse_log(&[
            "-d",
            "long session",
            "-t",
            "2h",
            &format!("--idle={}-{}", gaps[0].0, gaps[0].1),
            &format!("--idle={}-{}", gaps[1].0, gaps[1].1),
            "--trim",
        ]));

        let data = storage::load_data().unwrap();
        assert_eq!(
            data.entries.len(),
            gaps.len() + 1,
            "one piece per span left"
        );
        let idle_total: i64 = gaps.iter().map(|(from, to)| to - from).sum();
        // Summed as durations: per-piece truncation would lose the sub-second parts.
        let logged = data
            .entries
            .iter()
            .fold(chrono::Duration::zero(), |acc, e| acc + e.duration());
        assert_eq!(
            logged,
            chrono::Duration::hours(2) - chrono::Duration::seconds(idle_total),
            "the pieces do not sum to the span minus the idle stretches"
        );
        let ids: Vec<u64> = data.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        assert_eq!(data.next_id, 3, "no id was spent twice");
        assert!(
            data.entries.iter().all(|e| e.idle.is_empty()),
            "a piece kept an interval it excludes"
        );
        let store = storage::get_data_path().unwrap();
        assert!(
            store.starts_with(&dir) && store.exists(),
            "the sandbox store"
        );
        assert!(
            !store.with_extension("json.tmp").exists(),
            "a temp store file survived the command"
        );
    }

    #[test]
    fn idle_without_trim_leaves_a_single_entry() {
        let _guard = env_guard();
        sandbox("log-no-trim");
        let end = Local::now().timestamp();
        let start = end - 3600;

        run_log(parse_log(&[
            "-d",
            "left alone",
            "-t",
            "60m",
            &format!("--idle={}-{}", start + 600, start + 1200),
        ]));

        let data = storage::load_data().unwrap();
        assert_eq!(data.entries.len(), 1, "recording is not trimming");
        assert_eq!(data.entries[0].duration(), chrono::Duration::minutes(60));
        assert_eq!(data.entries[0].idle.len(), 1, "the interval is still there");
    }
    /// `--start` back-dates the active entry rather than starting it now, so
    /// `tt status` already shows the elapsed time when it is first asked.
    #[test]
    fn start_back_dates_the_active_entry_to_the_given_clock_time() {
        let _guard = env_guard();
        sandbox("start-back-dated");
        let now = Local::now();
        let clock = (now - Duration::minutes(90)).time().with_second(0).unwrap();

        start(
            vec!["reading".into(), "the".into(), "spec".into()],
            Some("timetracker".into()),
            Some(clock),
        )
        .unwrap();

        let data = storage::load_data().unwrap();
        let entry = data.active_entry().expect("an active entry");
        assert_eq!(
            entry.start_time.time(),
            clock,
            "stored at the asked-for clock time"
        );
        assert_eq!(entry.project.as_deref(), Some("timetracker"));
    }

    #[test]
    fn a_start_time_still_to_come_is_refused_rather_than_scheduled() {
        let now = Local
            .with_ymd_and_hms(2026, 9, 3, 10, 0, 0)
            .single()
            .unwrap();
        assert!(
            resolve_start(NaiveTime::from_hms_opt(9, 30, 0).unwrap(), now).is_ok(),
            "half an hour ago is exactly what the flag is for"
        );
        assert!(
            resolve_start(NaiveTime::from_hms_opt(10, 30, 0).unwrap(), now).is_err(),
            "a future time is a mistake, not a scheduled task"
        );
    }
}
