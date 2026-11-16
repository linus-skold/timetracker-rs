//! The `tt agent` commands: the agent layer's phase marks. Presentation only;
//! [`crate::marks`] owns every fact about the mark files. **`begin`, `touch`,
//! `cancel`, `list` and a plain `audit` must touch no store** — `main.rs` dispatches
//! them ahead of its migrate preamble. The messages are a contract read by an agent.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local};

use crate::tracker::IdleInterval;

use crate::activity;
use crate::audit;
use crate::cli::{ActivityCommands, AgentCommands};
use crate::commands;
use crate::icons;
use crate::marks::{self, Begin, MarkRef, Thresholds, Touch};
use crate::storage;
use crate::tracker;

/// Run one `tt agent` subcommand.
pub fn run(command: &AgentCommands) -> Result<()> {
    match command {
        AgentCommands::Begin {
            project,
            issue,
            phase,
            agent,
        } => begin(mark_ref(project, issue, phase, agent.as_deref())),
        AgentCommands::Touch {
            project,
            issue,
            phase,
            agent,
        } => touch(mark_ref(project, issue, phase, agent.as_deref())),
        AgentCommands::Cancel {
            project,
            issue,
            phase,
            agent,
        } => cancel(mark_ref(project, issue, phase, agent.as_deref())),
        AgentCommands::List { project } => list(project.as_deref()),
        AgentCommands::Item {
            project,
            issue,
            phase,
            summary,
            minutes,
            data,
        } => item(
            project,
            issue,
            phase,
            summary.as_deref(),
            minutes.as_deref(),
            data.clone(),
        ),
        AgentCommands::End {
            project,
            issue,
            phase,
            summary,
            minutes,
            full,
            trim,
            agent,
            session,
            data,
        } => end(
            mark_ref(project, issue, phase, agent.as_deref()),
            Close {
                summary: summary.as_deref(),
                minutes: minutes.as_deref(),
                full: *full,
                trim: *trim,
                session: session.as_deref(),
                data: data.clone(),
            },
        ),
        AgentCommands::Resolve {
            session,
            start,
            issue,
            phase,
            summary,
        } => resolve(session, *start, issue, phase, summary),
        AgentCommands::Dismiss {
            session,
            project,
            span,
            reason,
        } => dismiss(session, project, span, reason.as_deref()),
        AgentCommands::Activity(command) => activity_command(command),
        AgentCommands::Audit {
            auto_log,
            json,
            project,
        } => run_audit(*auto_log, *json, project.as_deref()),
    }
}

/// Silently does nothing if the activity dir can't be resolved: a hook must not fail its event.
fn activity_command(command: &ActivityCommands) -> Result<()> {
    // Beats only, nothing filed: it must not depend on the activity dir.
    if let ActivityCommands::Prompt { project } = command {
        beat_project(project.as_deref());
        return Ok(());
    }
    let Some(dir) = activity::activity_dir() else {
        return Ok(());
    };
    match command {
        ActivityCommands::Begin {
            session_id,
            project,
        } => activity::begin_in(&dir, session_id, project.as_deref())?,
        ActivityCommands::End {
            session_id,
            project,
        } => {
            activity::end_in(&dir, session_id)?;
            beat_project(project.as_deref());
        }
        ActivityCommands::Subagent {
            session_id,
            project,
        } => {
            activity::subagent_in(&dir, session_id)?;
            // Without this beat `end` sees the wait for the report as silence.
            beat_project(project.as_deref());
        }
        ActivityCommands::Prompt { .. } => unreachable!("handled before the dir read"),
        ActivityCommands::Check {
            session_id,
            auto_log,
        } => return check_session(&dir, session_id, *auto_log),
    }
    Ok(())
}

/// `tt agent audit` narrowed to one session, for the `Stop` hook: silent when the session
/// is accounted for, unknown, or has no resolved project. `--auto-log` acts only when
/// `agent.auto_log_on_stop` is set; a written window is marked `(auto-logged)`.
fn check_session(dir: &std::path::Path, session_id: &str, auto_log: bool) -> Result<()> {
    let Some(session) = activity::read_session_in(dir, session_id) else {
        return Ok(());
    };
    let leases = open_leases();
    let dismissals = crate::dismissed::read_all();
    let now = chrono::Local::now();
    let thresholds = audit::thresholds();

    let flagged = {
        let mut data = storage::load_data()?;
        tracker::migrate(&mut data);
        audit::unaccounted(
            &[session],
            &leases,
            &data.entries,
            &dismissals,
            now,
            thresholds,
        )
    };

    let threshold = auto_log
        .then(audit::auto_log_on_stop_enabled)
        .and_then(|enabled| enabled.then(audit::auto_log_after_minutes))
        .flatten();

    // A warning surface, so the floor applies here.
    for item in audit::over_floor(&flagged, thresholds) {
        let minutes = item.end.signed_duration_since(item.start).num_minutes();
        if threshold.is_some_and(|threshold| minutes > threshold) {
            write_auto_log(item, false)?;
            println!("{} (auto-logged)", item.describe());
        } else {
            println!("{}", item.describe());
        }
    }
    Ok(())
}

/// Beat the open marks of the project a hook resolved; `None` beats nothing.
fn beat_project(project: Option<&str>) {
    if let (Some(project), Some(dir)) = (project, marks::mark_dir()) {
        marks::touch_project_in(&dir, project);
    }
}

/// Every open mark paired with its last heartbeat; no mark directory reads as none.
fn open_leases() -> Vec<marks::Lease> {
    marks::mark_dir()
        .map(|dir| marks::open_leases_in(&dir))
        .unwrap_or_default()
}

/// The mark directory, or an error: a `begin` must never silently record nothing.
fn mark_dir() -> Result<std::path::PathBuf> {
    marks::mark_dir().context("could not determine a cache directory for the marks")
}

/// The mark the clap fields address.
fn mark_ref<'a>(
    project: &'a str,
    issue: &'a str,
    phase: &'a str,
    agent: Option<&'a str>,
) -> MarkRef<'a> {
    MarkRef {
        project,
        issue,
        phase,
        agent,
    }
}

/// The phase as the messages name it: `<project>/<issue> <phase>`, `-` sentinel
/// included, plus `:<label>` for a mark with an agent label.
fn phase_name(mark: MarkRef) -> String {
    let name = format!("{}/{} {}", mark.project, mark.issue, mark.phase);
    match mark.agent {
        Some(agent) => format!("{name}:{agent}"),
        None => name,
    }
}

/// `tt agent begin <project> <issue|-> <phase>`: open a mark, or keep the one
/// already open. Idempotent: the original start wins, on stderr, exit 0.
fn begin(mark: MarkRef) -> Result<()> {
    let dir = mark_dir()?;
    match marks::begin_in(&dir, mark)? {
        Begin::Created(start) => {
            println!("marked {} at {}", phase_name(mark), start.format("%H:%M"))
        }
        Begin::AlreadyOpen(start) => {
            // `??:??` for a mark whose contents are not a timestamp.
            let since = start.map_or_else(
                || "??:??".to_string(),
                |start| start.format("%H:%M").to_string(),
            );
            eprintln!(
                "tt: already marked {} (since {}) — using the original start",
                phase_name(mark),
                since
            );
        }
        Begin::Closing => refuse_unfinished_close(mark),
    }
    Ok(())
}

/// Report a close that was started and never finished, and exit 75; nothing is cleared.
fn refuse_unfinished_close(mark: MarkRef) -> ! {
    eprintln!(
        "tt: {} has an unfinished close — a previous close may already have recorded its entry",
        phase_name(mark)
    );
    eprintln!(
        "tt: check tt report, then tt agent cancel {} to clear it.",
        mark.args()
    );
    std::process::exit(75);
}

/// `tt agent touch <project> <issue|-> <phase>`: one heartbeat; exit 64 on an unmarked phase.
fn touch(mark: MarkRef) -> Result<()> {
    let dir = mark_dir()?;
    match marks::touch_in(&dir, mark)? {
        Touch::Recorded => Ok(()),
        Touch::NoMark => {
            eprintln!("tt: no mark for {} — nothing to touch", phase_name(mark));
            std::process::exit(64);
        }
    }
}

/// `tt agent cancel <project> <issue|-> <phase>`: drop a mark without logging, present or not.
fn cancel(mark: MarkRef) -> Result<()> {
    let dir = mark_dir()?;
    marks::cancel_in(&dir, mark)?;
    println!("dropped mark for {}", phase_name(mark));
    Ok(())
}

/// `tt agent list [project]`: every open mark of the project, newest first, in
/// `commands::list`'s shape, or a bare `No open marks.`
fn list(project: Option<&str>) -> Result<()> {
    let mut leases = open_leases();
    if let Some(project) = project {
        leases.retain(|lease| marks::owned_by(&lease.mark, project));
    }
    if leases.is_empty() {
        println!("No open marks.");
        return Ok(());
    }

    println!("{} Open marks:\n", icons::agent());
    for row in marks::rows(&leases, audit::thresholds()) {
        println!("  {}", row);
    }
    Ok(())
}

/// Every unaccounted row at `now`, from the activity ledger, the open marks, the
/// dismissal ledger and the store. Two calls that must agree take one `now`: a live
/// session's row end tracks the clock.
fn unaccounted_at(now: DateTime<Local>) -> Result<Vec<audit::Unaccounted>> {
    let sessions = activity::activity_dir()
        .map(|dir| activity::read_sessions_in(&dir))
        .unwrap_or_default();
    let leases = open_leases();
    let dismissals = crate::dismissed::read_all();
    let mut data = storage::load_data()?;
    tracker::migrate(&mut data);
    Ok(audit::unaccounted(
        &sessions,
        &leases,
        &data.entries,
        &dismissals,
        now,
        audit::thresholds(),
    ))
}

/// `tt agent resolve --session <id> <start> <issue|-> <phase> "<summary>"`: cover the
/// row with that session and start with one entry spanning it exactly. The project is
/// the matched row's, never an argument, and the span is logged unrounded: a rounded
/// duration reaches back past the row's start. A pair matching no row writes nothing.
fn resolve(session: &str, start: i64, issue: &str, phase: &str, summary: &str) -> Result<()> {
    // A row is addressed by the sanitised key its session file is named with.
    let session = crate::paths::sanitise_key(session);
    let rows = unaccounted_at(Local::now())?;
    let Some(row) = rows
        .iter()
        .find(|row| row.session_id == session && row.start.timestamp() == start)
    else {
        eprintln!("tt: no unaccounted row for session {session} starting at {start}");
        eprintln!("tt: run tt agent audit --json to list the rows and the addresses they take.");
        std::process::exit(64);
    };

    commands::log(commands::LogRequest {
        description: description(&row.project, issue, phase, summary),
        // The row's own span, never rounded: `ended_at` is pinned, so a longer
        // duration would reach back past the row's start.
        time: row.end.signed_duration_since(row.start),
        extra_tags: Vec::new(),
        project: Some(row.project.clone()),
        idle: Vec::new(),
        // The row is one contiguous active stretch; nothing here may cut it again.
        trim: false,
        ended_at: Some(row.end),
        // Names the session, so this entry covers that agent's row and not a
        // concurrent agent's row over the same minutes.
        data: Some(stamp_session(None, &session)),
        confirm_on_stderr: false,
    })
}

/// `tt agent dismiss --session <id> <project> <start>-<end> ["<reason>"]`: record that
/// one session's stretch of clock time was not work. Records the span as given and
/// matches no row: it is a statement about clock time, not an operation on a row.
fn dismiss(session: &str, project: &str, span: &IdleInterval, reason: Option<&str>) -> Result<()> {
    let (from, until) = (span.start.timestamp(), span.end.timestamp());
    if until <= from {
        eprintln!("tt: a dismissal needs a span that ends after it starts, got {from}-{until}");
        std::process::exit(64);
    }
    let dir = crate::dismissed::dismissed_dir()
        .context("could not determine a cache directory for the dismissals")?;
    crate::dismissed::write_in(
        &dir,
        &crate::dismissed::Dismissal {
            project: project.to_string(),
            // The ledger matches an activity session by its file name, which is the
            // sanitised id; a raw id that sanitises differently would match no row.
            session_id: crate::paths::sanitise_key(session),
            start: from,
            end: until,
            reason: reason.map(str::to_string),
        },
    )?;
    println!(
        "dismissed {} {}-{} for session {}",
        project,
        span.start.format("%H:%M"),
        span.end.format("%H:%M"),
        session
    );
    Ok(())
}

/// `tt agent audit`: reconcile the activity ledger against marks and logged entries,
/// reporting activity with no evidence it was tracked. Missing directories read as empty.
/// `--json` prints the same rows as an array, `[]` included, and never the prose.
fn run_audit(auto_log: bool, json: bool, project: Option<&str>) -> Result<()> {
    let now = chrono::Local::now();
    // Narrowed before the auto-log loop, never after: `--project` bounds what this
    // run may write, not just what it prints.
    let narrow = |mut rows: Vec<audit::Unaccounted>| {
        if let Some(project) = project {
            rows.retain(|item| marks::same_project(&item.project, project));
        }
        rows
    };
    let flagged = narrow(unaccounted_at(now)?);
    let thresholds = audit::thresholds();

    // `auto_log_after_minutes` unset: `--auto-log` logs nothing, like a plain audit.
    // Over the floor only: `--auto-log` writes what the warning names, nothing more.
    let mut wrote_any = false;
    if auto_log && let Some(threshold) = audit::auto_log_after_minutes() {
        for item in audit::over_floor(&flagged, thresholds) {
            let minutes = item.end.signed_duration_since(item.start).num_minutes();
            if minutes > threshold {
                // `--json` promises one array on stdout, so the confirmation
                // goes to stderr.
                write_auto_log(item, json)?;
                wrote_any = true;
            }
        }
    }

    // Re-read: the entries just written now cover their own rows.
    let remaining = if wrote_any {
        narrow(unaccounted_at(now)?)
    } else {
        flagged
    };

    // Every row, floor or no floor: this is the list the sweep addresses rows from.
    if json {
        println!("{}", serde_json::to_string(&remaining)?);
        return Ok(());
    }

    let warned = audit::over_floor(&remaining, thresholds);
    if warned.is_empty() {
        println!("No unaccounted agent activity.");
        return Ok(());
    }

    println!("{} Unaccounted agent activity:\n", icons::warning());
    for item in warned {
        println!("  {}", item.describe());
    }
    Ok(())
}

/// `--auto-log`: write one fixed-phase `#auto` entry for an unaccounted window, the way
/// `tt agent item` would but **never** tagged `#agent`; phase and summary are literal text.
fn write_auto_log(item: &audit::Unaccounted, quiet: bool) -> Result<()> {
    let minutes = item.end.signed_duration_since(item.start).num_minutes();
    commands::log(commands::LogRequest {
        description: "unattended activity #auto".to_string(),
        // Never rounded, whatever `agent.round_minutes` says: `ended_at` is pinned,
        // so a longer span would reach back into the idle gap the audit just judged.
        time: Duration::minutes(minutes),
        extra_tags: Vec::new(),
        project: Some(item.project.clone()),
        idle: Vec::new(),
        // The row is one contiguous active stretch; nothing here may cut it again.
        trim: false,
        ended_at: Some(item.end),
        // Names the row's session, or this entry would silence a concurrent
        // same-project session's row as well.
        data: Some(stamp_session(None, &item.session_id)),
        confirm_on_stderr: quiet,
    })
}

/// `tt agent item <project> <issue|-> <phase> <summary> <minutes>`: log one finished piece
/// of work for a duration already known. No mark file is read, written or cleared.
fn item(
    project: &str,
    issue: &str,
    phase: &str,
    summary: Option<&str>,
    minutes: Option<&str>,
    data: Option<serde_json::Value>,
) -> Result<()> {
    let (Some(summary), Some(minutes)) = (summary, minutes) else {
        eprintln!("tt: usage: tt agent item <project> <issue|-> <phase> <summary> <minutes>");
        std::process::exit(64);
    };
    let minutes = whole_minutes(minutes);
    log_entry(
        project,
        issue,
        phase,
        summary,
        round_to(minutes, round_minutes()),
        Span::unmarked(),
        data,
    )
}

/// A minutes argument, or exit 64 with `minutes must be a whole number, got '<x>'`.
/// Stricter than `parse`: no sign, no whitespace, no `+`.
fn whole_minutes(raw: &str) -> i64 {
    let digits = !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit());
    match digits.then(|| raw.parse().ok()).flatten() {
        Some(minutes) => minutes,
        None => {
            eprintln!("tt: minutes must be a whole number, got '{raw}'");
            std::process::exit(64);
        }
    }
}

/// The timeline `log_entry` records the entry on. `ended_at` is the mark's last bare
/// heartbeat for a mark-derived close, `None` where there is no mark timeline.
struct Span {
    idle: Vec<IdleInterval>,
    trim: bool,
    ended_at: Option<DateTime<Local>>,
}

impl Span {
    fn unmarked() -> Self {
        Span {
            idle: Vec::new(),
            trim: false,
            ended_at: None,
        }
    }
}

/// Log the entry both `item` and `end` end at; every tag is already in the description.
/// `minutes` is already rounded, so the caller and the stored span agree.
fn log_entry(
    project: &str,
    issue: &str,
    phase: &str,
    summary: &str,
    minutes: i64,
    span: Span,
    data: Option<serde_json::Value>,
) -> Result<()> {
    commands::log(commands::LogRequest {
        description: description(project, issue, phase, summary),
        time: Duration::minutes(minutes),
        extra_tags: Vec::new(),
        project: Some(project.to_string()),
        idle: span.idle,
        trim: span.trim,
        ended_at: span.ended_at,
        data,
        confirm_on_stderr: false,
    })
}

/// Everything `tt agent end` closes a phase with apart from the phase itself, so the
/// phase stays three plain arguments.
struct Close<'a> {
    summary: Option<&'a str>,
    minutes: Option<&'a str>,
    full: bool,
    trim: bool,
    session: Option<&'a str>,
    data: Option<serde_json::Value>,
}

/// `tt agent end <project> <issue|-> <phase> <summary> [minutes|--full|--trim]`: close a
/// marked phase, measured to its last **bare** heartbeat wherever that sits in the beats
/// file; only a phase with no bare beat measures to now. A flagged silence with nothing
/// said about it **refuses** the close. Explicit minutes win over both flags and skip the
/// mark's timestamps; `--full` logs the measured span, `--trim` it minus every gap.
fn end(mark: MarkRef, close: Close) -> Result<()> {
    let Close {
        summary,
        minutes,
        full,
        trim,
        session,
        data,
    } = close;
    let Some(summary) = summary else {
        // Hand-checked: a required positional would exit 2, not 64.
        eprintln!(
            "tt: need a summary: tt agent end <project> <issue|-> <phase> <summary> [minutes]"
        );
        std::process::exit(64);
    };
    let mut data = stamp_label(data, mark.agent);
    if let Some(session) = session {
        data = Some(stamp_session(data, session));
    }

    let dir = mark_dir()?;
    if marks::is_closing_in(&dir, mark) {
        refuse_unfinished_close(mark);
    }

    let mut idle = Vec::new();
    let mut split_at_idle = false;
    // Stays `None` on the explicit-minutes path.
    let mut anchor = None;

    // What this close leaves for the audit: the mark's head on the explicit-minutes
    // path, one stretch per gap `--trim` cuts, nothing at all for `--full`.
    let mut remainder: Vec<(i64, i64)> = Vec::new();

    // Read once: the clamp and the stored start must use the same step.
    let step = round_minutes();
    let minutes = match minutes {
        Some(raw) => {
            let minutes = whole_minutes(raw);
            // The one mark read on this path, and only to state what it leaves
            // behind: explicit minutes still close a phase with no mark at all.
            if let Some(marked) = marks::read_phase_in(&dir, mark).ok().flatten() {
                let head = (marked.started, Local::now().timestamp() - minutes * 60);
                if head.1 > head.0 {
                    remainder.push(head);
                }
            }
            minutes
        }
        None => {
            let Some(marked) = marks::read_phase_in(&dir, mark)? else {
                eprintln!(
                    "tt: no mark for {} — pass minutes explicitly",
                    phase_name(mark)
                );
                std::process::exit(64);
            };

            // Clamped, so a heartbeat behind the mark is a zero-length phase.
            let ended = marked
                .beats
                .last()
                .copied()
                .unwrap_or_else(|| Local::now().timestamp())
                .max(marked.started);
            let measured = (ended - marked.started) / 60;
            // The gaps are epochs on this timeline, so the entry ends where it does.
            anchor = Some(instant(ended)?);

            // An unvouched phase is one whole-span silence against the longer grace.
            let Thresholds { gap, unvouched } = audit::thresholds();
            let gaps = match marked.beats.is_empty() {
                true => marks::gaps_over(marked.started, ended, &[], unvouched),
                false => marks::gaps_over(marked.started, ended, &marked.beats, gap),
            };
            if gaps.is_empty() {
                measured
            } else {
                // One interval per flagged gap whether or not the caller trimmed.
                // The duration floors to whole minutes, so the entry can start
                // after the mark; clamp the gaps to the entry.
                let entry_start = ended - round_to(measured, step) * 60;
                idle = gaps
                    .iter()
                    .filter(|&&(_, to)| to > entry_start)
                    .map(|&(from, to)| {
                        Ok(IdleInterval::new(
                            instant(from.max(entry_start))?,
                            instant(to)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                // Every over-threshold gap. Reported, never logged: `split_at_idle` subtracts.
                let silent: i64 = gaps.iter().map(|(from, to)| (to - from) / 60).sum();
                let trimmed = (measured - silent).max(0);

                if full {
                    measured
                } else if trim {
                    // The measured span, not `trimmed`: `split_at_idle` cuts the same gaps.
                    split_at_idle = true;
                    remainder = gaps.clone();
                    measured
                } else {
                    refuse(mark, &gaps, measured, trimmed);
                }
            }
        }
    };

    // Written before the entry, so a failure here logs nothing and the retry is safe.
    marks::start_closing_in(&dir, mark)?;
    log_entry(
        mark.project,
        mark.issue,
        mark.phase,
        summary,
        round_to(minutes, step),
        Span {
            idle,
            trim: split_at_idle,
            ended_at: anchor,
        },
        data,
    )?;
    record_remainder(mark, session, &remainder);
    // Cleared only once the entry is recorded; a refusal leaves the mark and its beats.
    if let Err(err) = marks::cancel_in(&dir, mark) {
        eprintln!(
            "tt: {} is recorded, but its mark could not be cleared: {err}",
            phase_name(mark)
        );
        eprintln!(
            "tt: do not retry the close — run tt agent cancel {} once the mark directory is writable.",
            mark.args()
        );
        std::process::exit(74);
    }
    Ok(())
}

/// Dismiss what a close left uncovered, for the session it names. Keyed by session
/// and never by mark: several labelled marks under one orchestrator clear against the
/// same session's rows. **Warns and returns**, whatever fails — the close is already
/// recorded, and `end`'s exit codes are a documented contract.
fn record_remainder(mark: MarkRef, session: Option<&str>, remainder: &[(i64, i64)]) {
    if remainder.is_empty() {
        return;
    }
    let Some(session) = session else {
        let minutes: i64 = remainder.iter().map(|(from, to)| (to - from) / 60).sum();
        eprintln!(
            "tt: {}m of {}'s span is not covered by this entry",
            minutes,
            phase_name(mark)
        );
        eprintln!(
            "tt: pass --session <id> to dismiss it, or clear it with tt agent audit --json --project {}",
            mark.project
        );
        return;
    };

    let Some(dir) = crate::dismissed::dismissed_dir() else {
        eprintln!("tt: no cache directory for the dismissals — the remainder stays unaccounted");
        return;
    };
    for &(start, end) in remainder {
        let record = crate::dismissed::Dismissal {
            project: mark.project.to_string(),
            session_id: crate::paths::sanitise_key(session),
            start,
            end,
            reason: Some(format!("not billed by the close of {}", phase_name(mark))),
        };
        if let Err(err) = crate::dismissed::write_in(&dir, &record) {
            eprintln!(
                "tt: the remainder of {} was not dismissed: {err}",
                phase_name(mark)
            );
        }
    }
}

/// The close's data with the mark's label stamped in, or exit 64 on data whose
/// `agent` key is not an object. Unlabelled data is passed through untouched.
fn stamp_label(data: Option<serde_json::Value>, agent: Option<&str>) -> Option<serde_json::Value> {
    let Some(label) = agent else {
        return data;
    };
    match crate::entry_data::with_agent_label(data, label) {
        Ok(data) => Some(data),
        Err(message) => {
            eprintln!("tt: {message}");
            std::process::exit(64);
        }
    }
}

/// The data with the activity session stamped in, or exit 64 on data whose `agent`
/// key is not an object — the same contract as [`stamp_label`].
fn stamp_session(data: Option<serde_json::Value>, session: &str) -> serde_json::Value {
    match crate::entry_data::with_agent_session(data, session) {
        Ok(data) => data,
        Err(message) => {
            eprintln!("tt: {message}");
            std::process::exit(64);
        }
    }
}

/// Refuse the close, naming the worst hole, and exit 65. Both totals are the
/// measured minutes, before any `agent.round_minutes` step.
fn refuse(mark: MarkRef, gaps: &[(i64, i64)], measured: i64, trimmed: i64) -> ! {
    // Strictly greater, so the **first** of two equal holes is the one named.
    let mut worst = (0, 0, 0);
    for &(from, to) in gaps {
        let minutes = (to - from) / 60;
        if minutes > worst.0 {
            worst = (minutes, from, to);
        }
    }

    eprintln!(
        "tt: {} has {} {}m gap ({}-{})",
        phase_name(mark),
        article(worst.0),
        worst.0,
        clock(worst.1),
        clock(worst.2)
    );
    // A zero trim means the holes cover the span: offer the explicit minutes alone.
    match trimmed {
        0 => eprintln!("tt: --full logs {measured}m"),
        trimmed => eprintln!("tt: --full logs {measured}m, --trim logs {trimmed}m"),
    }
    eprintln!("tt: or pass the real minutes instead.");
    std::process::exit(65);
}

/// `"a"` or `"an"` for a minute count, read the way it is spoken: `an` for any
/// number opening on `8`, and for `11` and `18` exactly.
fn article(minutes: i64) -> &'static str {
    if minutes == 11 || minutes == 18 || minutes.abs().to_string().starts_with('8') {
        "an"
    } else {
        "a"
    }
}

/// [`crate::time::instant`], naming the value that would not parse.
fn instant(epoch: i64) -> Result<DateTime<Local>> {
    crate::time::instant(epoch).with_context(|| format!("{epoch} is not a valid timestamp"))
}

/// One epoch as `HH:MM`, or `??:??` when it is not a valid instant.
fn clock(epoch: i64) -> String {
    instant(epoch).map_or_else(
        |_| "??:??".to_string(),
        |instant| instant.format("%H:%M").to_string(),
    )
}

// --- the shared convention -------------------------------------------------
//
// `item` and `end` log the same way: the rounding, the stripping and the tags live here.

/// Round minutes **up** to the next `step` minutes, never below `step`: a ceiling,
/// never nearest. A `step` of 0 or less rounds nothing.
fn round_to(minutes: i64, step: i64) -> i64 {
    if step <= 0 {
        return minutes;
    }
    (((minutes + step - 1) / step) * step).max(step)
}

/// The `agent.round_minutes` step every logged agent duration is rounded up to.
/// 0 (the default) logs the actual minutes.
fn round_minutes() -> i64 {
    crate::config::load().agent.round_minutes.unwrap_or(0)
}

/// Strip a `#` run that begins a word, so a summary mentioning "#12" does not become
/// a tag. A **mid-word** `#` is left alone: `C#` and `F#` are real words.
fn strip_stray_tags(summary: &str) -> String {
    let mut stripped = String::with_capacity(summary.len());
    let mut at_word_start = true;
    for c in summary.chars() {
        if c == '#' && at_word_start {
            // The whole run goes, and the position stays a word start.
            continue;
        }
        at_word_start = c.is_ascii_whitespace();
        stripped.push(c);
    }
    stripped
}

/// The phase vocabulary, in the order the docs list it. `src/report.rs` reads it back
/// off the stored tags, so this must stay the one list; it validates nothing.
pub const PHASES: [&str; 8] = [
    "plan", "impl", "qa", "review", "docs", "spike", "explore", "ops",
];

/// The description `commands::log` is given: the summary, then one tag per axis the
/// `project` field cannot express — the item (omitted for `-`), the phase, and
/// `#agent`. There is **no bare `#<project>` tag**; `commands::log` runs `parse_tags`.
pub(crate) fn description(project: &str, issue: &str, phase: &str, summary: &str) -> String {
    let mut description = strip_stray_tags(summary);
    if issue != "-" {
        description.push_str(&format!(" #{project}/{issue}"));
    }
    description.push_str(&format!(" #{phase} #agent"));
    description
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_article_follows_how_the_number_is_spoken() {
        for minutes in [8, 80, 85, 800] {
            assert_eq!(article(minutes), "an", "{minutes}");
        }
        assert_eq!(article(11), "an");
        assert_eq!(article(18), "an");
        for minutes in [7, 45, 70, 110, 118, 180, 1, 0] {
            assert_eq!(article(minutes), "a", "{minutes}");
        }
    }

    #[test]
    fn rounding_always_rounds_up_to_the_next_step() {
        assert_eq!(round_to(36, 5), 40);
        assert_eq!(round_to(37, 5), 40);
        assert_eq!(round_to(40, 5), 40, "already on a five-minute mark");
        assert_eq!(round_to(41, 5), 45);
        assert_eq!(round_to(45, 5), 45);
        assert_eq!(round_to(31, 15), 45);
        assert_eq!(round_to(45, 15), 45);
    }

    #[test]
    fn rounding_never_goes_below_one_step() {
        assert_eq!(round_to(0, 5), 5);
        assert_eq!(round_to(1, 5), 5);
        assert_eq!(round_to(4, 5), 5);
        assert_eq!(round_to(1, 15), 15);
    }

    #[test]
    fn a_step_of_zero_leaves_the_actual_minutes_alone() {
        assert_eq!(round_to(0, 0), 0);
        assert_eq!(round_to(1, 0), 1);
        assert_eq!(round_to(47, 0), 47);
        assert_eq!(round_to(47, -5), 47, "a negative step rounds nothing");
    }

    #[test]
    fn a_word_starting_with_a_hash_keeps_the_word_and_loses_the_hash() {
        // Without the strip, `parse_tags` would harvest `#12` as a tag.
        assert_eq!(strip_stray_tags("closed #12 at last"), "closed 12 at last");
        assert_eq!(strip_stray_tags("#12 closed"), "12 closed");
        assert_eq!(strip_stray_tags("closed ##12"), "closed 12");
        assert_eq!(strip_stray_tags("a\t#12"), "a\t12");
    }

    #[test]
    fn a_mid_word_hash_survives() {
        // `parse_tags` ignores it, and C#/F# are real words.
        assert_eq!(
            strip_stray_tags("ported the C# bridge"),
            "ported the C# bridge"
        );
        assert_eq!(strip_stray_tags("F#"), "F#");
    }

    #[test]
    fn the_description_carries_the_item_phase_and_agent_tags() {
        assert_eq!(
            description("loremind", "77", "impl", "store/links boundary"),
            "store/links boundary #loremind/77 #impl #agent"
        );
    }

    #[test]
    fn the_sentinel_issue_drops_the_item_tag_and_no_bare_project_tag_is_emitted() {
        let built = description("loremind", "-", "plan", "sketched the shape");
        assert_eq!(built, "sketched the shape #plan #agent");
        // The project is a real field with its own axis — never a tag.
        assert!(
            !built.contains("#loremind"),
            "a bare project tag was emitted: {built:?}"
        );
    }
}
