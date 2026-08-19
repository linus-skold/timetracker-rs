//! The `tt agent` commands: the agent layer's phase marks.
//!
//! Presentation only; [`crate::marks`] owns every fact about the mark files.
//!
//! **`begin`, `touch`, `cancel` and `list` must touch no store** — `main.rs`
//! dispatches them ahead of its migrate preamble. `item` and `end` log an entry
//! through [`crate::cli::log`] and dispatch after it.
//!
//! The messages are a contract: their caller is an agent following prose.

use anyhow::{Context, Result};
use chrono::{DateTime, Local};

use crate::tracker::IdleInterval;

use crate::cli::{self, AgentCommands};
use crate::config;
use crate::icons;
use crate::marks::{self, Begin, Touch};

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
        } => item(
            project,
            issue,
            phase,
            summary.as_deref(),
            minutes.as_deref(),
        ),
        AgentCommands::End {
            project,
            issue,
            phase,
            summary,
            minutes,
            full,
            trim,
        } => end(
            project,
            issue,
            phase,
            summary.as_deref(),
            minutes.as_deref(),
            *full,
            *trim,
        ),
    }
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
    }
    Ok(())
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

/// `tt agent list`: every open mark, newest first, in `cli::list`'s shape —
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

/// `tt agent item <project> <issue|-> <phase> <summary> <minutes>`: log one
/// finished piece of work in one call. No mark file is read, written or cleared.
fn item(
    project: &str,
    issue: &str,
    phase: &str,
    summary: Option<&str>,
    minutes: Option<&str>,
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
        Vec::new(),
        false,
        None,
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

/// Log the entry both `item` and `end` end at. `extra_tags` stays empty: every
/// tag is already in the description. `ended_at` is the mark's last heartbeat for
/// a mark-derived close and `None` where there is no mark timeline to pin to.
#[allow(clippy::too_many_arguments)]
fn log_entry(
    project: &str,
    issue: &str,
    phase: &str,
    summary: &str,
    minutes: i64,
    idle: Vec<IdleInterval>,
    trim: bool,
    ended_at: Option<DateTime<Local>>,
) -> Result<()> {
    cli::log(
        description(project, issue, phase, summary),
        format!("{}m", round_quarter(minutes)),
        Vec::new(),
        Some(project.to_string()),
        idle,
        trim,
        ended_at,
    )
}

/// `tt agent end <project> <issue|-> <phase> <summary> [minutes|--full|--trim]`:
/// close a marked phase, measured to its **last heartbeat** and never to now. A
/// flagged silence with nothing said about it **refuses** the close.
///
/// Explicit minutes win over both flags and skip the mark's timestamps entirely;
/// `--full` logs the measured span, `--trim` the span minus every flagged gap.
fn end(
    project: &str,
    issue: &str,
    phase: &str,
    summary: Option<&str>,
    minutes: Option<&str>,
    full: bool,
    trim: bool,
) -> Result<()> {
    let Some(summary) = summary else {
        // Hand-checked: a required positional would exit 2, not 64.
        eprintln!(
            "tt: need a summary: tt agent end <project> <issue|-> <phase> <summary> [minutes]"
        );
        std::process::exit(64);
    };

    let dir = mark_dir()?;
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
                max_unvouched_minutes()
            } else {
                max_gap_minutes()
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

    log_entry(
        project,
        issue,
        phase,
        summary,
        minutes,
        idle,
        split_at_idle,
        anchor,
    )?;
    // Cleared only once the entry is recorded, on every successful close; a
    // refusal returned above with the mark and its beats left in place.
    marks::cancel_in(&dir, project, issue, phase)?;
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

/// How long a silence *between heartbeats* has to be to count, in minutes.
/// `TT_MAX_GAP_MINUTES`, else `agent.max_gap_minutes`, else 45.
fn max_gap_minutes() -> i64 {
    minutes(
        "TT_MAX_GAP_MINUTES",
        config::load().agent.max_gap_minutes,
        45,
    )
}

/// How long an **unvouched** phase — a mark with no heartbeat at all — may run
/// before the close is refused. `TT_MAX_UNVOUCHED_MINUTES`, else
/// `agent.max_unvouched_minutes`, else 120, longer than the interior-silence
/// threshold.
fn max_unvouched_minutes() -> i64 {
    minutes(
        "TT_MAX_UNVOUCHED_MINUTES",
        config::load().agent.max_unvouched_minutes,
        120,
    )
}

/// A minutes-valued setting: the environment wins over the config file, which
/// wins over the built-in default. Set-but-empty and unparseable read as unset.
fn minutes(name: &str, configured: Option<i64>, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .or(configured)
        .unwrap_or(default)
}

fn instant(epoch: i64) -> Result<DateTime<Local>> {
    DateTime::from_timestamp(epoch, 0)
        .map(|instant| instant.with_timezone(&Local))
        .with_context(|| format!("{epoch} is not a valid timestamp"))
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

/// Round minutes to the nearest quarter hour, halfway up, never below 15.
fn round_quarter(minutes: i64) -> i64 {
    (((minutes + 7) / 15) * 15).max(15)
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
pub const PHASES: [&str; 7] = ["plan", "impl", "qa", "review", "docs", "spike", "ops"];

/// The description `cli::log` is given: the summary, then one tag per axis the
/// `project` field cannot express — the item (omitted for the `-` sentinel), the
/// phase, and `#agent`. There is **no bare `#<project>` tag**; `cli::log` runs
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
    fn rounding_goes_up_to_the_nearest_quarter() {
        assert_eq!(round_quarter(37), 30);
        assert_eq!(round_quarter(38), 45);
        assert_eq!(round_quarter(43), 45);
        assert_eq!(round_quarter(45), 45);
    }

    #[test]
    fn rounding_never_goes_below_a_quarter_hour() {
        // A two-minute errand costs a quarter, and so does zero.
        assert_eq!(round_quarter(0), 15);
        assert_eq!(round_quarter(2), 15);
        assert_eq!(round_quarter(7), 15);
        assert_eq!(round_quarter(8), 15);
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
