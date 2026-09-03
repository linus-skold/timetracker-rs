//! The `tt agent` commands: the agent layer's phase marks.
//!
//! Presentation only; [`crate::marks`] owns every fact about the mark files.
//!
//! **`begin`, `touch`, `cancel`, `list` and a plain `audit` must touch no
//! store** — `main.rs` dispatches them ahead of its migrate preamble. `item`,
//! `end` and `audit --auto-log` log an entry through [`crate::commands::log`] and
//! dispatch after it.
//!
//! The messages are a contract: their caller is an agent following prose.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local};

use crate::tracker::IdleInterval;

use crate::activity;
use crate::audit;
use crate::cli::{ActivityCommands, AgentCommands};
use crate::commands;
use crate::icons;
use crate::marks::{self, Begin, Touch};
use crate::storage;
use crate::tracker;

/// Run one `tt agent` subcommand.
pub fn run(command: &AgentCommands) -> Result<()> {
    match command {
        AgentCommands::Begin {
            project,
            issue,
            phase,
        } => begin(project, issue, phase),
        AgentCommands::Touch {
            project,
            issue,
            phase,
        } => touch(project, issue, phase),
        AgentCommands::Cancel {
            project,
            issue,
            phase,
        } => cancel(project, issue, phase),
        AgentCommands::List => list(),
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
            data,
        } => end(
            project,
            issue,
            phase,
            Close {
                summary: summary.as_deref(),
                minutes: minutes.as_deref(),
                full: *full,
                trim: *trim,
                data: data.clone(),
            },
        ),
        AgentCommands::Activity(command) => activity_command(command),
        AgentCommands::Audit { auto_log } => run_audit(*auto_log),
    }
}

/// Silently does nothing if the activity dir can't be resolved — a hook must
/// never fail the harness event it's attached to.
fn activity_command(command: &ActivityCommands) -> Result<()> {
    let Some(dir) = activity::activity_dir() else {
        return Ok(());
    };
    match command {
        ActivityCommands::Begin {
            session_id,
            project,
        } => activity::begin_in(&dir, session_id, project.as_deref())?,
        ActivityCommands::End { session_id } => activity::end_in(&dir, session_id)?,
        ActivityCommands::Subagent { session_id } => {
            activity::subagent_in(&dir, session_id)?;
            // The hook is the one witness to when a subagent's work stopped;
            // without this beat `end` sees the wait for the report as silence.
            if let Some(marks) = marks::mark_dir() {
                marks::touch_all_in(&marks);
            }
        }
        ActivityCommands::Check {
            session_id,
            auto_log,
        } => return check_session(&dir, session_id, *auto_log),
    }
    Ok(())
}

/// `tt agent activity check <session_id> [--auto-log]`: the same
/// reconciliation as `tt agent audit`, narrowed to one session, so the `Stop`
/// hook can warn immediately rather than waiting for the next `audit` run.
/// Silent when the session is accounted for, unknown, or has no resolved
/// project.
///
/// `--auto-log` is passed unconditionally by the hook; `tt` itself decides
/// whether to act on it via `agent.auto_log_on_stop` (see
/// docs/decisions/0003-auto-log-on-stop.md) — a flag with the setting off is
/// the same as a plain check. A window that gets written is marked
/// `(auto-logged)` in its line, so the hook can tell the two cases apart.
fn check_session(dir: &std::path::Path, session_id: &str, auto_log: bool) -> Result<()> {
    let Some(session) = activity::read_session_in(dir, session_id) else {
        return Ok(());
    };
    let marks = marks::open_marks();
    let now = chrono::Local::now();
    let floor = audit::max_unvouched_minutes();

    let flagged = {
        let mut data = storage::load_data()?;
        tracker::migrate(&mut data);
        audit::unaccounted(&[session], &marks, &data.entries, now, floor)
    };

    let threshold = auto_log
        .then(audit::auto_log_on_stop_enabled)
        .and_then(|enabled| enabled.then(audit::auto_log_after_minutes))
        .flatten();

    for item in &flagged {
        let minutes = item.end.signed_duration_since(item.start).num_minutes();
        if threshold.is_some_and(|threshold| minutes > threshold) {
            write_auto_log(item)?;
            println!("{} (auto-logged)", item.describe());
        } else {
            println!("{}", item.describe());
        }
    }
    Ok(())
}

/// The mark directory, or an error — a `begin` must never silently record
/// nothing.
fn mark_dir() -> Result<std::path::PathBuf> {
    marks::mark_dir().context("could not determine a cache directory for the marks")
}

/// The phase as the messages name it: `<project>/<issue> <phase>`, with the `-`
/// sentinel left in place. Only `list` collapses it.
fn phase_name(project: &str, issue: &str, phase: &str) -> String {
    format!("{}/{} {}", project, issue, phase)
}

/// `tt agent begin <project> <issue|-> <phase>`: open a mark, or keep the one
/// already open. Idempotent: the original start wins, on stderr, exit 0.
fn begin(project: &str, issue: &str, phase: &str) -> Result<()> {
    let dir = mark_dir()?;
    match marks::begin_in(&dir, project, issue, phase)? {
        Begin::Created(start) => println!(
            "marked {} at {}",
            phase_name(project, issue, phase),
            start.format("%H:%M")
        ),
        Begin::AlreadyOpen(start) => {
            // `??:??` for a mark whose contents are not a timestamp.
            let since = start.map_or_else(
                || "??:??".to_string(),
                |start| start.format("%H:%M").to_string(),
            );
            eprintln!(
                "tt: already marked {} (since {}) — using the original start",
                phase_name(project, issue, phase),
                since
            );
        }
        Begin::Closing => refuse_unfinished_close(project, issue, phase),
    }
    Ok(())
}

/// Report a close that was started and never finished, and exit 75. The leftover
/// is named, never cleared: only the operator can tell whether its entry landed.
fn refuse_unfinished_close(project: &str, issue: &str, phase: &str) -> ! {
    eprintln!(
        "tt: {} has an unfinished close — a previous close may already have recorded its entry",
        phase_name(project, issue, phase)
    );
    eprintln!("tt: check tt report, then tt agent cancel {project} {issue} {phase} to clear it.");
    std::process::exit(75);
}

/// `tt agent touch <project> <issue|-> <phase>`: record one heartbeat. Exits 64
/// on an unmarked phase; anyhow would exit 1, so the code is set explicitly.
fn touch(project: &str, issue: &str, phase: &str) -> Result<()> {
    let dir = mark_dir()?;
    match marks::touch_in(&dir, project, issue, phase)? {
        Touch::Recorded => Ok(()),
        Touch::NoMark => {
            eprintln!(
                "tt: no mark for {} — nothing to touch",
                phase_name(project, issue, phase)
            );
            std::process::exit(64);
        }
    }
}

/// `tt agent cancel <project> <issue|-> <phase>`: drop a mark without logging.
/// Succeeds whether or not there was anything to drop.
fn cancel(project: &str, issue: &str, phase: &str) -> Result<()> {
    let dir = mark_dir()?;
    marks::cancel_in(&dir, project, issue, phase)?;
    println!("dropped mark for {}", phase_name(project, issue, phase));
    Ok(())
}

/// `tt agent list`: every open mark, newest first, in `commands::list`'s shape —
/// header, blank line, rows at the status-glyph indent, or a bare
/// `No open marks.`
fn list() -> Result<()> {
    let marks = marks::open_marks();
    if marks.is_empty() {
        println!("No open marks.");
        return Ok(());
    }

    println!("{} Open marks:\n", icons::agent());
    for row in marks::rows(&marks) {
        println!("  {}", row);
    }
    Ok(())
}

/// `tt agent audit`: reconcile the activity ledger against marks and logged
/// entries, reporting activity with no evidence it was ever tracked. Missing
/// or unreadable activity/mark directories read as empty rather than erroring
/// — an audit must never fail over there being nothing to audit yet.
fn run_audit(auto_log: bool) -> Result<()> {
    let sessions = activity::activity_dir()
        .map(|dir| activity::read_sessions_in(&dir))
        .unwrap_or_default();
    let marks = marks::open_marks();
    let floor = audit::max_unvouched_minutes();
    let now = chrono::Local::now();

    let flagged = {
        let mut data = storage::load_data()?;
        tracker::migrate(&mut data);
        audit::unaccounted(&sessions, &marks, &data.entries, now, floor)
    };

    // `auto_log_after_minutes` unset: `--auto-log` is accepted but logs
    // nothing, exactly like a plain audit — see AgentConfig::auto_log_after_minutes.
    let mut wrote_any = false;
    if auto_log && let Some(threshold) = audit::auto_log_after_minutes() {
        for item in &flagged {
            let minutes = item.end.signed_duration_since(item.start).num_minutes();
            if minutes > threshold {
                write_auto_log(item)?;
                wrote_any = true;
            }
        }
    }

    // Re-read: the entries just written now cover their own windows, so the
    // report below must not still call them unaccounted.
    let remaining = if wrote_any {
        let mut data = storage::load_data()?;
        tracker::migrate(&mut data);
        audit::unaccounted(&sessions, &marks, &data.entries, now, floor)
    } else {
        flagged
    };

    if remaining.is_empty() {
        println!("No unaccounted agent activity.");
        return Ok(());
    }

    println!("{} Unaccounted agent activity:\n", icons::warning());
    for item in &remaining {
        println!("  {}", item.describe());
    }
    Ok(())
}

/// `--auto-log`: write one fixed-phase `#auto` entry for an unaccounted
/// window, the way `tt agent item` would — except **never** tagged `#agent`,
/// since this was not an agent's own self-report. Phase and summary are
/// always the same literal text; nothing here guesses either. See
/// docs/decisions/0002-auto-logging-unaccounted-activity.md.
fn write_auto_log(item: &audit::Unaccounted) -> Result<()> {
    let minutes = item.end.signed_duration_since(item.start).num_minutes();
    commands::log(commands::LogRequest {
        description: "unattended activity #auto".to_string(),
        time: Duration::minutes(round_five(minutes)),
        extra_tags: Vec::new(),
        project: Some(item.project.clone()),
        idle: item.idle.clone(),
        trim: true,
        ended_at: Some(item.end),
        data: None,
    })
}

/// `tt agent item <project> <issue|-> <phase> <summary> <minutes>`: log one
/// finished piece of work in one call, for a duration already known some
/// other way. No mark file is read, written or cleared — this is the
/// fallback for when there was nothing to `begin`/`end` around; prefer that
/// pair whenever the work can be marked as it happens instead.
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
        minutes,
        Span::unmarked(),
        data,
    )
}

/// A minutes argument, or exit 64 with `minutes must be a whole number, got
/// '<x>'` — hand-parsed because clap would exit 2 with its own usage block.
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

/// The timeline `log_entry` records the entry on: the flagged silence, whether
/// to cut it out, and where the span ends. `ended_at` is the mark's last
/// heartbeat for a mark-derived close and `None` where there is no mark
/// timeline to pin to.
struct Span {
    idle: Vec<IdleInterval>,
    trim: bool,
    ended_at: Option<DateTime<Local>>,
}

impl Span {
    /// A span with no mark behind it: no silence, nothing to trim, no anchor.
    fn unmarked() -> Self {
        Span {
            idle: Vec::new(),
            trim: false,
            ended_at: None,
        }
    }
}

/// Log the entry both `item` and `end` end at. `extra_tags` stays empty: every
/// tag is already in the description.
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
        time: Duration::minutes(round_five(minutes)),
        extra_tags: Vec::new(),
        project: Some(project.to_string()),
        idle: span.idle,
        trim: span.trim,
        ended_at: span.ended_at,
        data,
    })
}

/// Everything `tt agent end` was asked to close a phase with, apart from which
/// phase it is: the summary, the duration override, the two silence flags and
/// the custom data. One struct, so the phase stays three plain arguments.
struct Close<'a> {
    summary: Option<&'a str>,
    minutes: Option<&'a str>,
    full: bool,
    trim: bool,
    data: Option<serde_json::Value>,
}

/// `tt agent end <project> <issue|-> <phase> <summary> [minutes|--full|--trim]`:
/// close a marked phase, measured to its **last heartbeat** and never to now. A
/// flagged silence with nothing said about it **refuses** the close.
///
/// Explicit minutes win over both flags and skip the mark's timestamps entirely;
/// `--full` logs the measured span, `--trim` the span minus every flagged gap.
fn end(project: &str, issue: &str, phase: &str, close: Close) -> Result<()> {
    let Close {
        summary,
        minutes,
        full,
        trim,
        data,
    } = close;
    let Some(summary) = summary else {
        // Hand-checked: a required positional would exit 2, not 64.
        eprintln!(
            "tt: need a summary: tt agent end <project> <issue|-> <phase> <summary> [minutes]"
        );
        std::process::exit(64);
    };

    let dir = mark_dir()?;
    if marks::is_closing_in(&dir, project, issue, phase) {
        refuse_unfinished_close(project, issue, phase);
    }

    let mut idle = Vec::new();
    let mut split_at_idle = false;
    // Stays `None` on the explicit-minutes path, which reads no timestamps.
    let mut anchor = None;

    let minutes = match minutes {
        Some(raw) => whole_minutes(raw),
        None => {
            let Some(marked) = marks::read_phase_in(&dir, project, issue, phase)? else {
                eprintln!(
                    "tt: no mark for {} — pass minutes explicitly",
                    phase_name(project, issue, phase)
                );
                std::process::exit(64);
            };

            // Clamped, so a heartbeat behind the mark is a zero-length phase.
            let ended = marked
                .ended
                .unwrap_or_else(|| Local::now().timestamp())
                .max(marked.started);
            let measured = (ended - marked.started) / 60;
            // The gaps below are epochs on this timeline, so the entry has to
            // end where the timeline does.
            anchor = Some(instant(ended)?);

            // A mark with heartbeats is judged against the interior-silence
            // threshold, a mark with none against the longer unvouched one.
            let threshold = if marked.beats.is_empty() {
                audit::max_unvouched_minutes()
            } else {
                audit::max_gap_minutes()
            };
            let gaps = marks::gaps_over(marked.started, ended, &marked.beats, threshold);
            if gaps.is_empty() {
                measured
            } else {
                // One interval per flagged gap, recorded whether or not the
                // caller trimmed, so the TUI can still trim it later.
                idle = gaps
                    .iter()
                    .map(|&(from, to)| Ok(IdleInterval::new(instant(from)?, instant(to)?)))
                    .collect::<Result<Vec<_>>>()?;
                // Every over-threshold gap, not just the worst one the refusal
                // names. Reported, never logged: `split_at_idle` subtracts.
                let silent: i64 = gaps.iter().map(|(from, to)| (to - from) / 60).sum();
                let trimmed = (measured - silent).max(0);

                if full {
                    measured
                } else if trim {
                    // The measured span, not `trimmed`: `split_at_idle` cuts the
                    // same gaps, and a pre-trimmed span subtracts them twice.
                    split_at_idle = true;
                    measured
                } else {
                    refuse(project, issue, phase, &gaps, measured, trimmed);
                }
            }
        }
    };

    // Written before the entry, so a mark directory that cannot be written fails
    // here with nothing logged and the retry safe.
    marks::start_closing_in(&dir, project, issue, phase)?;
    log_entry(
        project,
        issue,
        phase,
        summary,
        minutes,
        Span {
            idle,
            trim: split_at_idle,
            ended_at: anchor,
        },
        data,
    )?;
    // Cleared only once the entry is recorded, on every successful close; a
    // refusal returned above with the mark and its beats left in place.
    if let Err(err) = marks::cancel_in(&dir, project, issue, phase) {
        eprintln!(
            "tt: {} is recorded, but its mark could not be cleared: {err}",
            phase_name(project, issue, phase)
        );
        eprintln!(
            "tt: do not retry the close — run tt agent cancel {project} {issue} {phase} once the mark directory is writable."
        );
        std::process::exit(74);
    }
    Ok(())
}

/// Refuse the close, naming the worst hole, and exit 65. Both the silent total
/// and the interval go on one line, **unrounded**.
fn refuse(
    project: &str,
    issue: &str,
    phase: &str,
    gaps: &[(i64, i64)],
    measured: i64,
    trimmed: i64,
) -> ! {
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
        phase_name(project, issue, phase),
        article(worst.0),
        worst.0,
        clock(worst.1),
        clock(worst.2)
    );
    eprintln!("tt: --full logs {measured}m, --trim logs {trimmed}m");
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

/// [`crate::time::instant`] with the context this layer owes the operator: a
/// mark that will not parse names the value that would not.
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
// `item` and `end` must log an entry the same way, so the rounding, the stray-`#`
// stripping and the three tags live here once.

/// Round minutes **up** to the next 5 minutes, never below 5. Always a
/// ceiling, never nearest — a logged span never reads shorter than what was
/// actually spent.
fn round_five(minutes: i64) -> i64 {
    (((minutes + 4) / 5) * 5).max(5)
}

/// Strip a `#` run that begins a word, so a summary mentioning "#12" does not
/// become a tag. A **mid-word** `#` is left alone: `C#` and `F#` are real words.
///
/// [`parse_tags`]: crate::tracker::parse_tags
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

/// The phase vocabulary, in the order the docs list it. `src/report.rs` reads it
/// back off the stored tags, so it must stay the one list. Not used to validate
/// the `phase` argument: any word is accepted.
pub const PHASES: [&str; 8] = [
    "plan", "impl", "qa", "review", "docs", "spike", "explore", "ops",
];

/// The description `commands::log` is given: the summary, then one tag per axis the
/// `project` field cannot express — the item (omitted for the `-` sentinel), the
/// phase, and `#agent`. There is **no bare `#<project>` tag**; `commands::log` runs
/// `parse_tags` over this string to build the entry's tags.
fn description(project: &str, issue: &str, phase: &str, summary: &str) -> String {
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
        // Every number opening on the digit 8, whatever its magnitude.
        for minutes in [8, 80, 85, 800] {
            assert_eq!(article(minutes), "an", "{minutes}");
        }
        // The two teens read as one vowel-initial word.
        assert_eq!(article(11), "an");
        assert_eq!(article(18), "an");
        // And their multiples, which are not: "a hundred and ten".
        for minutes in [7, 45, 70, 110, 118, 180, 1, 0] {
            assert_eq!(article(minutes), "a", "{minutes}");
        }
    }

    #[test]
    fn rounding_always_rounds_up_to_the_next_five_minutes() {
        assert_eq!(round_five(36), 40);
        assert_eq!(round_five(37), 40);
        assert_eq!(round_five(40), 40, "already on a five-minute mark");
        assert_eq!(round_five(41), 45);
        assert_eq!(round_five(45), 45);
    }

    #[test]
    fn rounding_never_goes_below_five_minutes() {
        // A two-minute errand costs five, and so does zero.
        assert_eq!(round_five(0), 5);
        assert_eq!(round_five(1), 5);
        assert_eq!(round_five(2), 5);
        assert_eq!(round_five(4), 5);
    }

    #[test]
    fn a_word_starting_with_a_hash_keeps_the_word_and_loses_the_hash() {
        // Without the strip, `parse_tags` would harvest `#12` as a tag.
        assert_eq!(strip_stray_tags("closed #12 at last"), "closed 12 at last");
        assert_eq!(strip_stray_tags("#12 closed"), "12 closed");
        // A run goes whole, and the whitespace it followed survives.
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
