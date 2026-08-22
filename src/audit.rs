//! Reconciles the hook-only activity ledger against marks and logged
//! entries — the `tt agent audit` command. See
//! `docs/decisions/0001-agent-activity-tracking.md`.
//!
//! An activity window counts as accounted for when either a mark was open
//! for its project overlapping any part of it, or a closed `#agent`- or
//! `#auto`-tagged entry covers it (see
//! `docs/decisions/0002-auto-logging-unaccounted-activity.md` for the
//! latter). Neither is **unaccounted agent activity**: real work that never
//! got tracked at all.

use chrono::{DateTime, Local};

use crate::activity::Session;
use crate::marks::Mark;
use crate::tracker::TimeEntry;

/// One activity window with no evidence it was tracked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unaccounted {
    pub project: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub subagents: usize,
}

impl Unaccounted {
    /// `<project> - since HH:MM (Xh Ym)`, with a trailing subagent-dispatch
    /// count when there were any. Shared by the CLI (`tt agent audit`,
    /// `tt agent activity check`) and the TUI's Agents panel, so the two
    /// cannot disagree on how this reads.
    pub fn describe(&self) -> String {
        let subagents = match self.subagents {
            0 => String::new(),
            1 => ", 1 subagent dispatch".to_string(),
            n => format!(", {n} subagent dispatches"),
        };
        format!(
            "{} - since {} ({}{})",
            self.project,
            self.start.format("%H:%M"),
            crate::duration::format(self.end.signed_duration_since(self.start)),
            subagents
        )
    }
}

/// How long an activity window may run with no covering mark or entry before
/// it counts as unaccounted, in minutes. Shared with the same setting
/// `tt agent end` judges an unvouched phase against: `TT_MAX_UNVOUCHED_MINUTES`,
/// else `agent.max_unvouched_minutes`, else 120.
pub fn max_unvouched_minutes() -> i64 {
    crate::config::resolve_minutes(
        "TT_MAX_UNVOUCHED_MINUTES",
        crate::config::load().agent.max_unvouched_minutes,
        120,
    )
}

/// How long a window must stay unaccounted for before `tt agent audit
/// --auto-log` (see docs/decisions/0002-auto-logging-unaccounted-activity.md)
/// writes a fallback `#auto` entry for it, in minutes.
/// `TT_AUTO_LOG_AFTER_MINUTES`, else `agent.auto_log_after_minutes`.
///
/// `None` disables auto-logging outright — both when neither is set (the
/// default) and when the configured value does not exceed
/// [`max_unvouched_minutes`]. The latter is a misconfiguration, and it must
/// fail toward "off" rather than toward auto-logging a window the audit
/// surfaces never had a chance to warn about first.
///
/// Unused outside tests until `--auto-log` (issue #19) reads it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn auto_log_after_minutes() -> Option<i64> {
    let configured = std::env::var("TT_AUTO_LOG_AFTER_MINUTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .or(crate::config::load().agent.auto_log_after_minutes)?;
    (configured > max_unvouched_minutes()).then_some(configured)
}

/// Half-open interval overlap: touching endpoints do not count.
fn overlaps(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

/// Every session below `floor_minutes` is not flagged — a one-line question
/// should not trip the warning. A session with no project cannot be
/// reconciled against anything, so it is skipped rather than assumed
/// unaccounted.
pub fn unaccounted(
    sessions: &[Session],
    marks: &[Mark],
    entries: &[TimeEntry],
    now: DateTime<Local>,
    floor_minutes: i64,
) -> Vec<Unaccounted> {
    let now_epoch = now.timestamp();

    let mut found: Vec<Unaccounted> = sessions
        .iter()
        .filter_map(|session| {
            let project = session.project.as_deref()?;
            let end_epoch = session.end.unwrap_or(now_epoch);
            if (end_epoch - session.start) / 60 < floor_minutes {
                return None;
            }

            let covered = covered_by_mark(project, session.start, end_epoch, marks, now_epoch)
                || covered_by_entry(project, session.start, end_epoch, entries, now_epoch);
            if covered {
                return None;
            }

            Some(Unaccounted {
                project: project.to_string(),
                start: instant(session.start)?,
                end: instant(end_epoch)?,
                subagents: session.subagents,
            })
        })
        .collect();

    found.sort_by_key(|u| std::cmp::Reverse(u.start));
    found
}

/// A mark still open covers everything from its own start up to `now`.
fn covered_by_mark(project: &str, start: i64, end: i64, marks: &[Mark], now: i64) -> bool {
    marks
        .iter()
        .filter(|mark| mark.project.eq_ignore_ascii_case(project))
        .any(|mark| overlaps(mark.start.timestamp(), now, start, end))
}

/// A closed entry covers a window when it's tagged `#agent` (an agent's own
/// self-report) or `#auto` (a prior `--auto-log` run) — never on `#auto`
/// alone being absent from `#agent`'s definition; the two provenances are
/// deliberately distinct tags, checked together only here.
fn covered_by_entry(project: &str, start: i64, end: i64, entries: &[TimeEntry], now: i64) -> bool {
    entries
        .iter()
        .filter(|entry| entry.has_tag("agent") || entry.has_tag("auto"))
        .filter(|entry| {
            entry
                .project
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(project))
        })
        .any(|entry| {
            let entry_end = entry.end_time.map(|t| t.timestamp()).unwrap_or(now);
            overlaps(entry.start_time.timestamp(), entry_end, start, end)
        })
}

fn instant(epoch: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp(epoch, 0).map(|instant| instant.with_timezone(&Local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn session(project: Option<&str>, start: i64, end: Option<i64>, subagents: usize) -> Session {
        Session {
            project: project.map(str::to_string),
            start,
            end,
            subagents,
        }
    }

    fn mark(project: &str, start: i64) -> Mark {
        Mark {
            project: project.to_string(),
            issue: None,
            phase: "impl".to_string(),
            start: at(start),
        }
    }

    fn entry(project: &str, start: i64, end: Option<i64>, tags: &[&str]) -> TimeEntry {
        TimeEntry {
            id: 1,
            description: "x".to_string(),
            project: Some(project.to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: at(start),
            end_time: end.map(at),
            idle: Vec::new(),
        }
    }

    fn at(epoch: i64) -> DateTime<Local> {
        Local.timestamp_opt(epoch, 0).unwrap()
    }

    const FLOOR: i64 = 120;
    const HOUR: i64 = 3600;

    #[test]
    fn a_session_with_no_project_is_never_flagged() {
        let sessions = vec![session(None, 0, Some(3 * HOUR), 0)];
        assert!(unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_session_under_the_floor_is_never_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(60 * 60), 0)]; // 1h, floor 2h
        assert!(unaccounted(&sessions, &[], &[], at(HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_session_over_the_floor_with_no_coverage_is_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 2)];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].project, "tt");
        assert_eq!(flagged[0].subagents, 2);
    }

    #[test]
    fn a_session_covered_by_an_overlapping_open_mark_is_not_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let marks = vec![mark("tt", HOUR)];
        assert!(unaccounted(&sessions, &marks, &[], at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_mark_for_a_different_project_does_not_cover() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let marks = vec![mark("other", HOUR)];
        assert_eq!(
            unaccounted(&sessions, &marks, &[], at(3 * HOUR), FLOOR).len(),
            1
        );
    }

    #[test]
    fn a_mark_that_started_after_the_window_ended_does_not_cover() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let marks = vec![mark("tt", 4 * HOUR)];
        assert_eq!(
            unaccounted(&sessions, &marks, &[], at(5 * HOUR), FLOOR).len(),
            1
        );
    }

    #[test]
    fn a_session_covered_by_a_closed_agent_tagged_entry_is_not_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 0, Some(3 * HOUR), &["tt", "impl", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_logged_entry_without_the_agent_tag_does_not_cover() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 0, Some(3 * HOUR), &["tt", "impl"])];
        assert_eq!(
            unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).len(),
            1
        );
    }

    #[test]
    fn an_auto_tagged_entry_covers_the_same_as_an_agent_tagged_one() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 0, Some(3 * HOUR), &["tt", "auto", "auto"])];
        assert!(
            unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty(),
            "a prior --auto-log entry must stop the window from re-flagging"
        );
    }

    #[test]
    fn a_partial_overlap_with_a_logged_entry_still_covers() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        // Entry only covers the tail of the window, but any overlap counts.
        let entries = vec![entry("tt", 2 * HOUR, Some(4 * HOUR), &["tt", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_still_open_session_is_measured_to_now() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].end, at(3 * HOUR));
    }

    #[test]
    fn a_still_open_entry_is_measured_to_now_when_checking_coverage() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let entries = vec![entry("tt", 0, None, &["tt", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn results_come_back_newest_first() {
        let sessions = vec![
            session(Some("older"), 0, Some(3 * HOUR), 0),
            session(Some("newer"), HOUR, Some(4 * HOUR), 0),
        ];
        let flagged = unaccounted(&sessions, &[], &[], at(4 * HOUR), FLOOR);
        assert_eq!(
            flagged
                .iter()
                .map(|u| u.project.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
    }

    // --- auto_log_after_minutes --------------------------------------
    //
    // Serialised via `crate::storage::env_guard` against every other test
    // that touches env; it is process-wide.
    use crate::storage::env_guard;

    fn set(var: &str, value: &str) {
        unsafe { std::env::set_var(var, value) };
    }

    fn unset(var: &str) {
        unsafe { std::env::remove_var(var) };
    }

    #[test]
    fn unset_leaves_auto_logging_disabled() {
        let _guard = env_guard();
        unset("TT_AUTO_LOG_AFTER_MINUTES");
        unset("TT_MAX_UNVOUCHED_MINUTES");
        crate::storage::env_sandbox("audit-auto-log-unset");

        assert_eq!(auto_log_after_minutes(), None);
    }

    #[test]
    fn a_value_over_the_unvouched_floor_is_honoured() {
        let _guard = env_guard();
        crate::storage::env_sandbox("audit-auto-log-honoured");
        unset("TT_MAX_UNVOUCHED_MINUTES"); // default floor: 120
        set("TT_AUTO_LOG_AFTER_MINUTES", "480");

        assert_eq!(auto_log_after_minutes(), Some(480));
        unset("TT_AUTO_LOG_AFTER_MINUTES");
    }

    #[test]
    fn a_value_at_or_under_the_unvouched_floor_disables_auto_logging() {
        let _guard = env_guard();
        crate::storage::env_sandbox("audit-auto-log-misconfigured");
        set("TT_MAX_UNVOUCHED_MINUTES", "120");
        set("TT_AUTO_LOG_AFTER_MINUTES", "120");
        assert_eq!(
            auto_log_after_minutes(),
            None,
            "equal to the floor must not enable auto-logging"
        );

        set("TT_AUTO_LOG_AFTER_MINUTES", "60");
        assert_eq!(
            auto_log_after_minutes(),
            None,
            "under the floor must not enable auto-logging"
        );

        unset("TT_MAX_UNVOUCHED_MINUTES");
        unset("TT_AUTO_LOG_AFTER_MINUTES");
    }
}
