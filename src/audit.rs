//! Reconciles the hook-only activity ledger against marks and logged
//! entries — the `tt agent audit` command.
//!
//! An activity window counts as accounted for once every part of it falls inside
//! a same-project mark's lease or a same-project `#agent`- or `#auto`-tagged entry
//! — both subtract, so they compose. Neither is **unaccounted agent activity**:
//! real work that never got tracked at all.
//!
//! Reconciliation runs per session, and an open one is measured to its own last
//! evidence of life plus the lease's grace, clamped to now.

use chrono::{DateTime, Local};

use crate::activity::Session;
use crate::dismissed::Dismissal;
use crate::marks::{self, Lease, Thresholds};
use crate::time::instant;
use crate::tracker::TimeEntry;

/// One contiguous **active** stretch with no evidence it was tracked; a window's
/// idle holes are cut out before it becomes a row, so an entry covers a row whole.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Unaccounted {
    pub project: String,
    /// The activity session this stretch came from — with `start`, the address
    /// `tt agent resolve` and `tt agent dismiss` match a row on.
    #[serde(rename = "session")]
    pub session_id: String,
    /// Serialised as epoch seconds: `start` is half of a row's address, and a
    /// caller must never have to read it back out of a formatted time.
    #[serde(serialize_with = "as_epoch")]
    pub start: DateTime<Local>,
    #[serde(serialize_with = "as_epoch")]
    pub end: DateTime<Local>,
    pub subagents: usize,
    /// True only for the trailing row of a session bounded at its own last
    /// evidence of life, never for a session that closed with an `end=`.
    pub abandoned: bool,
}

impl Unaccounted {
    /// The row as the CLI and the TUI both print it; the address is the full session id, the hint is for the eye.
    pub fn describe(&self) -> String {
        let subagents = match self.subagents {
            0 => String::new(),
            1 => ", 1 subagent dispatch".to_string(),
            n => format!(", {n} subagent dispatches"),
        };
        format!(
            "{} - {} ({}{}) session {}{}",
            self.project,
            self.since(),
            crate::duration::format(self.end.signed_duration_since(self.start)),
            subagents,
            self.session_hint(),
            if self.abandoned { " [abandoned]" } else { "" }
        )
    }
}

fn as_epoch<S: serde::Serializer>(at: &DateTime<Local>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_i64(at.timestamp())
}

impl Unaccounted {
    /// `since <time>` for a start today, else `<date> <time>`.
    fn since(&self) -> String {
        if self.start.date_naive() == Local::now().date_naive() {
            format!("since {}", self.start.format("%H:%M"))
        } else {
            self.start.format("%-d %b, %H:%M").to_string()
        }
    }

    /// The leading characters of the session id, enough to read two rows apart.
    /// Never an address: the full id is what a command is given.
    fn session_hint(&self) -> String {
        self.session_id.chars().take(8).collect()
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

/// How long a silence *between subagent-dispatch heartbeats* has to be to
/// count as idle, in minutes — the same knob `tt agent end` judges interior
/// mark silence against. `TT_MAX_GAP_MINUTES`, else `agent.max_gap_minutes`,
/// else 45.
pub fn max_gap_minutes() -> i64 {
    crate::config::resolve_minutes(
        "TT_MAX_GAP_MINUTES",
        crate::config::load().agent.max_gap_minutes,
        45,
    )
}

/// Whether the `Stop` hook should auto-log a session's own unaccounted window,
/// per `agent.auto_log_on_stop`. `config::load` already resets this to `None` if
/// `auto_log_after_minutes` is not also set, so a bare read here is enough.
pub fn auto_log_on_stop_enabled() -> bool {
    crate::config::load().agent.auto_log_on_stop == Some(true)
}

/// How long a window must stay unaccounted for before `tt agent audit
/// --auto-log` writes a fallback `#auto` entry for it, in minutes.
/// `TT_AUTO_LOG_AFTER_MINUTES`, else `agent.auto_log_after_minutes`.
///
/// `None` disables auto-logging outright — both when neither is set (the
/// default) and when the configured value does not exceed
/// [`max_unvouched_minutes`]. The latter is a misconfiguration, and it must
/// fail toward "off" rather than toward auto-logging a window the audit
/// surfaces never had a chance to warn about first.
pub fn auto_log_after_minutes() -> Option<i64> {
    let configured = std::env::var("TT_AUTO_LOG_AFTER_MINUTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .or(crate::config::load().agent.auto_log_after_minutes)?;
    (configured > max_unvouched_minutes()).then_some(configured)
}

/// The thresholds every judgement below is made by, read from the environment and
/// config once at an entry point and passed down; nothing below this reads config.
pub fn thresholds() -> Thresholds {
    Thresholds {
        gap: max_gap_minutes(),
        unvouched: max_unvouched_minutes(),
    }
}

/// Half-open interval overlap: touching endpoints do not count.
fn overlaps(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

/// **Every** uncovered stretch, whatever it sums to: the floor is a warning
/// threshold, applied by [`over_floor`] and by nothing else, or a swept session's
/// remainder would stop being addressable. A session with no project cannot be
/// reconciled against anything, so it is skipped rather than assumed unaccounted.
pub fn unaccounted(
    sessions: &[Session],
    leases: &[Lease],
    entries: &[TimeEntry],
    dismissals: &[Dismissal],
    now: DateTime<Local>,
    thresholds: Thresholds,
) -> Vec<Unaccounted> {
    let now_epoch = now.timestamp();

    let mut found: Vec<Unaccounted> = sessions
        .iter()
        .flat_map(|session| {
            let Some(project) = session.project.as_deref() else {
                return Vec::new();
            };
            let (end, bounded_at) = session_end(session, now_epoch, thresholds);

            let stretches =
                uncovered_by_marks(project, session.start, end, leases, now_epoch, thresholds);
            let covered = uncovered_by_entries(project, &session.id, stretches, entries, now_epoch);
            let fragments = uncovered_by_dismissals(project, &session.id, covered, dismissals);
            // Every row is one contiguous active stretch: with no dispatches a
            // fragment passes through whole, otherwise it is cut at its idle holes.
            let active: Vec<(i64, i64)> = if session.subagent_at.is_empty() {
                fragments
            } else {
                fragments
                    .into_iter()
                    .flat_map(|fragment| {
                        marks::gaps_over(
                            fragment.0,
                            fragment.1,
                            &session.subagent_at,
                            thresholds.gap,
                        )
                        .into_iter()
                        .fold(vec![fragment], |pieces, hole| {
                            pieces
                                .into_iter()
                                .flat_map(|piece| subtract(piece, hole))
                                .collect()
                        })
                    })
                    .collect()
            };

            active
                .into_iter()
                .filter_map(|(from, to)| {
                    // A seam under a minute is a row nothing can ever cover: an
                    // entry shorter than a minute subtracts nothing.
                    if to - from < 60 {
                        return None;
                    }
                    // Per row, so the rows sum to the session's; with no timestamps its own count stands.
                    let subagents = if session.subagent_at.is_empty() {
                        session.subagents
                    } else {
                        session
                            .subagent_at
                            .iter()
                            .filter(|&&at| at >= from && at < to)
                            .count()
                    };

                    Some(Unaccounted {
                        project: project.to_string(),
                        session_id: session.id.clone(),
                        start: instant(from)?,
                        end: instant(to)?,
                        subagents,
                        // The trailing row of a bounded session: it ends at the bound,
                        // or at the last evidence when the grace was cut as idle.
                        abandoned: bounded_at
                            .is_some_and(|last_evidence| to == end || to == last_evidence),
                    })
                })
                .collect()
        })
        .collect();

    found.sort_by_key(|u| std::cmp::Reverse(u.start));
    found
}

/// The rows a warning surface shows: those of a session whose **own** uncovered
/// total reaches `thresholds.unvouched`. The floor gates the total, never a row, and
/// only here — `--json`, `resolve` and `dismiss` read every row.
pub fn over_floor(rows: &[Unaccounted], thresholds: Thresholds) -> Vec<&Unaccounted> {
    let mut totals: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for row in rows {
        *totals.entry(row.session_id.as_str()).or_default() +=
            row.end.signed_duration_since(row.start).num_minutes();
    }
    rows.iter()
        .filter(|row| {
            totals
                .get(row.session_id.as_str())
                .is_some_and(|total| *total >= thresholds.unvouched)
        })
        .collect()
}

/// Where a session is measured to: its own `end=`, else [`marks::grace_minutes`]
/// past its last evidence of life — its last dispatch, or its start with none —
/// clamped to `now`. The second value is that evidence's own instant, and `None`
/// while the grace still has time left. Reads no file mtime and no lease: the mtime
/// equals the last parsed line, and a lease's covered stretch is subtracted
/// downstream.
fn session_end(session: &Session, now: i64, thresholds: Thresholds) -> (i64, Option<i64>) {
    if let Some(end) = session.end {
        return (end, None);
    }
    let lively = !session.subagent_at.is_empty();
    let last = session
        .subagent_at
        .iter()
        .copied()
        .max()
        .unwrap_or(session.start)
        .max(session.start);
    let bound = last + marks::grace_minutes(lively, thresholds) * 60;
    (bound.min(now), (bound <= now).then_some(last))
}

/// What is left of `start → end` after removing every same-project lease's covered
/// interval — `mark.start` up to `now` or its expiry. Subtraction, never any-overlap.
fn uncovered_by_marks(
    project: &str,
    start: i64,
    end: i64,
    leases: &[Lease],
    now: i64,
    thresholds: Thresholds,
) -> Vec<(i64, i64)> {
    let mut remaining = vec![(start, end)];
    // The same segment rule the beat uses, so a lossy name still joins.
    for lease in leases
        .iter()
        .filter(|lease| marks::owned_by(&lease.mark, project))
    {
        let from = lease.mark.start.timestamp();
        let until = lease.expires_at(thresholds).timestamp().min(now);
        if until <= from {
            continue;
        }
        remaining = remaining
            .into_iter()
            .flat_map(|stretch| subtract(stretch, (from, until)))
            .collect();
    }
    remaining
}

/// What is left of `stretches` after removing every dismissal **of this session**
/// whose project matches. Another session's dismissal never subtracts here, or one
/// agent's "that was not work" would erase a concurrent agent's row.
fn uncovered_by_dismissals(
    project: &str,
    session: &str,
    stretches: Vec<(i64, i64)>,
    dismissals: &[Dismissal],
) -> Vec<(i64, i64)> {
    let mut remaining = stretches;
    for dismissal in dismissals.iter().filter(|dismissal| {
        dismissal.session_id == session && marks::same_project(&dismissal.project, project)
    }) {
        if dismissal.end <= dismissal.start {
            continue;
        }
        remaining = remaining
            .into_iter()
            .flat_map(|stretch| subtract(stretch, (dismissal.start, dismissal.end)))
            .collect();
    }
    remaining
}

/// `stretch` minus `cut`, as the zero, one or two stretches that survive.
fn subtract(stretch: (i64, i64), cut: (i64, i64)) -> Vec<(i64, i64)> {
    let (start, end) = stretch;
    let (cut_start, cut_end) = cut;
    if !overlaps(start, end, cut_start, cut_end) {
        return vec![stretch];
    }
    let mut kept = Vec::new();
    if start < cut_start {
        kept.push((start, cut_start));
    }
    if cut_end < end {
        kept.push((cut_end, end));
    }
    kept
}

/// What is left of `stretches` after removing every covering entry's span; an entry
/// covers when tagged `#agent` or `#auto`, and one still open covers up to `now`.
/// Subtraction of what the leases left, so the two coverage sources compose.
///
/// An entry carrying `agent.session` covers **that session only**, which is what lets
/// two agents clear their own row of the same minutes; one naming no session — `item`,
/// or a hand-written row — covers every session of the project.
fn uncovered_by_entries(
    project: &str,
    session: &str,
    stretches: Vec<(i64, i64)>,
    entries: &[TimeEntry],
    now: i64,
) -> Vec<(i64, i64)> {
    let mut remaining = stretches;
    for entry in entries
        .iter()
        .filter(|entry| entry.is_agent())
        .filter(|entry| {
            crate::entry_data::agent_session(entry.data.as_ref()).is_none_or(|its| its == session)
        })
        // The same segment rule the lease join uses.
        .filter(|entry| {
            entry
                .project
                .as_deref()
                .is_some_and(|p| marks::same_project(p, project))
        })
    {
        let from = entry.start_time.timestamp();
        let until = entry.end_time.map(|t| t.timestamp()).unwrap_or(now);
        if until <= from {
            continue;
        }
        remaining = remaining
            .into_iter()
            .flat_map(|stretch| subtract(stretch, (from, until)))
            .collect();
    }
    remaining
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn session(project: Option<&str>, start: i64, end: Option<i64>, subagents: usize) -> Session {
        Session {
            id: "sess-1".to_string(),
            project: project.map(str::to_string),
            start,
            end,
            subagents,
            subagent_at: Vec::new(),
        }
    }

    /// An open mark that has never beaten, so it leans on the unvouched grace.
    fn mark(project: &str, start: i64) -> Lease {
        Lease {
            mark: crate::marks::Mark {
                project: project.to_string(),
                issue: None,
                phase: "impl".to_string(),
                agent: None,
                start: at(start),
            },
            last_seen: None,
            vouched: false,
        }
    }

    /// The same mark, last beaten at `last_seen`.
    fn beaten(project: &str, start: i64, last_seen: i64) -> Lease {
        Lease {
            last_seen: Some(at(last_seen)),
            vouched: true,
            ..mark(project, start)
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
            data: None,
        }
    }

    /// The same entry, stamped with the session it was logged for.
    fn entry_for(
        session: &str,
        project: &str,
        start: i64,
        end: Option<i64>,
        tags: &[&str],
    ) -> TimeEntry {
        TimeEntry {
            data: Some(crate::entry_data::with_agent_session(None, session).unwrap()),
            ..entry(project, start, end, tags)
        }
    }

    fn at(epoch: i64) -> DateTime<Local> {
        Local.timestamp_opt(epoch, 0).unwrap()
    }

    const FLOOR: i64 = 120;
    const HOUR: i64 = 3600;

    /// The house pair, so no test below reads the developer's config.
    const HOUSE: Thresholds = Thresholds {
        gap: 45,
        unvouched: FLOOR,
    };

    /// Shadows [`super::unaccounted`] for every test below, with stated thresholds
    /// and the floor applied — what a warning surface sees. [`all_rows`] is the
    /// unfiltered view the addressing commands read.
    fn unaccounted(
        sessions: &[Session],
        leases: &[Lease],
        entries: &[TimeEntry],
        now: DateTime<Local>,
        floor_minutes: i64,
    ) -> Vec<Unaccounted> {
        dismissing(&[], sessions, leases, entries, now, floor_minutes)
    }

    /// Every row, floor or no floor.
    fn all_rows(
        sessions: &[Session],
        leases: &[Lease],
        entries: &[TimeEntry],
        now: DateTime<Local>,
        floor_minutes: i64,
    ) -> Vec<Unaccounted> {
        super::unaccounted(
            sessions,
            leases,
            entries,
            &[],
            now,
            Thresholds {
                gap: 45,
                unvouched: floor_minutes,
            },
        )
    }

    /// The same with a dismissal ledger, for the cases that need one.
    fn dismissing(
        dismissals: &[Dismissal],
        sessions: &[Session],
        leases: &[Lease],
        entries: &[TimeEntry],
        now: DateTime<Local>,
        floor_minutes: i64,
    ) -> Vec<Unaccounted> {
        let thresholds = Thresholds {
            gap: 45,
            unvouched: floor_minutes,
        };
        let rows = super::unaccounted(sessions, leases, entries, dismissals, now, thresholds);
        over_floor(&rows, thresholds).into_iter().cloned().collect()
    }

    fn dismissal(project: &str, session: &str, start: i64, end: i64) -> Dismissal {
        Dismissal {
            project: project.to_string(),
            session_id: session.to_string(),
            start,
            end,
            reason: None,
        }
    }

    #[test]
    fn one_instant_decides_both_the_stale_row_and_the_coverage() {
        let leases = vec![beaten("tt", 0, 3 * HOUR)];

        let fresh = 3 * HOUR + 45 * 60 + 59;
        assert!(uncovered_by_marks("tt", 0, fresh, &leases, fresh, HOUSE).is_empty());
        assert!(!crate::marks::rows_at(&leases, at(fresh), HOUSE)[0].contains("[stale]"));

        let expired = 3 * HOUR + 46 * 60;
        assert!(crate::marks::rows_at(&leases, at(expired), HOUSE)[0].contains("[stale]"));
        assert_eq!(
            uncovered_by_marks("tt", 0, expired + 1, &leases, expired + 1, HOUSE),
            vec![(expired, expired + 1)]
        );
    }

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

    /// The whole point of the split: a session swept down under the floor keeps
    /// its remaining rows, so they can still be resolved or dismissed.
    #[test]
    fn a_session_under_the_floor_keeps_its_rows_for_the_addressing_commands() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 50 * 60, Some(3 * HOUR), &["tt", "agent"])];
        let rows = all_rows(&sessions, &[], &entries, at(3 * HOUR), FLOOR);
        assert_eq!(
            rows.iter().map(|u| (u.start, u.end)).collect::<Vec<_>>(),
            vec![(at(0), at(50 * 60))]
        );
        assert!(
            over_floor(&rows, HOUSE).is_empty(),
            "50 minutes is under the 120-minute warning floor"
        );
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
    fn a_mark_open_for_days_with_no_beats_no_longer_covers() {
        let sessions = vec![session(Some("tt"), 110 * HOUR, Some(113 * HOUR), 0)];
        let abandoned = vec![mark("tt", 0)];
        assert_eq!(
            unaccounted(&sessions, &abandoned, &[], at(114 * HOUR), FLOOR).len(),
            1
        );

        // The same mark, beaten ten minutes ago, still vouches.
        let live = vec![beaten("tt", 0, 114 * HOUR - 600)];
        assert!(unaccounted(&sessions, &live, &[], at(114 * HOUR), FLOOR).is_empty());
    }

    /// The lease and the session bound share one grace, so a mark opened with
    /// the session expires exactly where the session's bound falls.
    #[test]
    fn a_still_open_session_marked_from_its_start_reports_nothing() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let abandoned = vec![mark("tt", 0)];
        assert!(
            unaccounted(&sessions, &abandoned, &[], at(114 * HOUR), FLOOR).is_empty(),
            "the bound leaves nothing the lease did not cover"
        );
    }

    #[test]
    fn two_consecutive_leases_covering_a_window_between_them_leave_nothing_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(4 * HOUR), 0)];
        // The first expires 2h in (unvouched), where the second picks up.
        let leases = vec![mark("tt", 0), beaten("tt", 2 * HOUR, 4 * HOUR)];
        assert!(unaccounted(&sessions, &leases, &[], at(4 * HOUR), FLOOR).is_empty());
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
    fn a_lease_for_a_lossy_project_name_still_covers_its_session() {
        let sessions = vec![session(Some("my proj"), 0, Some(3 * HOUR), 0)];
        let leases = vec![beaten("my_proj", 0, 3 * HOUR)];
        assert!(unaccounted(&sessions, &leases, &[], at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_mark_of_a_dot_related_project_does_not_cover() {
        for (session_project, mark_project) in [("app", "app.web"), ("app.web", "app")] {
            let sessions = vec![session(Some(session_project), 0, Some(3 * HOUR), 0)];
            let leases = vec![beaten(mark_project, 0, 3 * HOUR)];
            assert_eq!(
                unaccounted(&sessions, &leases, &[], at(3 * HOUR), FLOOR).len(),
                1,
                "a {mark_project} mark covered a {session_project} session"
            );
        }
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
    fn an_entry_covering_the_tail_leaves_the_head_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 2 * HOUR, Some(4 * HOUR), &["tt", "agent"])];
        let flagged = unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!((flagged[0].start, flagged[0].end), (at(0), at(2 * HOUR)));
    }

    #[test]
    fn an_entry_covering_the_middle_leaves_the_head_and_the_tail_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(6 * HOUR), 0)];
        let entries = vec![entry("tt", 2 * HOUR, Some(4 * HOUR), &["tt", "auto"])];
        let flagged = unaccounted(&sessions, &[], &entries, at(6 * HOUR), FLOOR);
        assert_eq!(
            flagged.iter().map(|u| (u.start, u.end)).collect::<Vec<_>>(),
            vec![(at(4 * HOUR), at(6 * HOUR)), (at(0), at(2 * HOUR))]
        );
    }

    /// The floor gates the session's total, not each fragment.
    #[test]
    fn six_uncovered_fragments_under_the_floor_are_all_reported() {
        let sessions = vec![session(Some("tt"), 0, Some(12 * HOUR), 0)];
        // A ten-minute covering entry every two hours, leaving six 110m holes.
        let entries: Vec<TimeEntry> = (0..6)
            .map(|i| {
                let cut = i * 120 * 60 + 110 * 60;
                entry("tt", cut, Some(cut + 10 * 60), &["tt", "agent"])
            })
            .collect();
        let flagged = unaccounted(&sessions, &[], &entries, at(12 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 6);
        for item in &flagged {
            assert_eq!(
                item.end.signed_duration_since(item.start).num_minutes(),
                110
            );
        }
    }

    /// A seam between one entry's end and the next's start is real but shorter
    /// than a minute, and no entry can ever cover it back.
    #[test]
    fn a_sub_minute_seam_is_never_a_row_of_its_own() {
        let sessions = vec![session(Some("tt"), 0, Some(6 * HOUR), 0)];
        let entries = vec![
            entry("tt", 0, Some(HOUR), &["tt", "agent"]),
            entry("tt", HOUR + 30, Some(2 * HOUR), &["tt", "agent"]),
            entry("tt", 2 * HOUR + 1, Some(3 * HOUR), &["tt", "agent"]),
        ];
        let flagged = unaccounted(&sessions, &[], &entries, at(6 * HOUR), FLOOR);
        assert_eq!(
            flagged.iter().map(|u| (u.start, u.end)).collect::<Vec<_>>(),
            vec![(at(3 * HOUR), at(6 * HOUR))]
        );
    }

    /// The floor sums `(to - from) / 60`, to which a sub-minute fragment already
    /// contributes 0, so filtering the rows cannot change what is reported.
    #[test]
    fn a_session_uncovered_only_by_sub_minute_seams_is_still_not_reported() {
        let sessions = vec![session(Some("tt"), 0, Some(6 * HOUR), 0)];
        let entries = vec![
            entry("tt", 0, Some(HOUR), &["tt", "agent"]),
            entry("tt", HOUR + 30, Some(6 * HOUR), &["tt", "agent"]),
        ];
        assert!(unaccounted(&sessions, &[], &entries, at(6 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_session_whose_coverage_leaves_three_minutes_reports_nothing() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 0, Some(3 * HOUR - 180), &["tt", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn an_entry_for_a_lossy_project_name_still_covers_its_session() {
        let sessions = vec![session(Some("my proj"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("my_proj", 0, Some(3 * HOUR), &["tt", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn an_entry_of_a_dot_related_project_does_not_cover() {
        for (session_project, entry_project) in [("app", "app.web"), ("app.web", "app")] {
            let sessions = vec![session(Some(session_project), 0, Some(3 * HOUR), 0)];
            let entries = vec![entry(entry_project, 0, Some(3 * HOUR), &["tt", "agent"])];
            assert_eq!(
                unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).len(),
                1,
                "an {entry_project} entry covered an {session_project} session"
            );
        }
    }

    #[test]
    fn each_fragment_counts_only_the_dispatches_inside_it() {
        let dispatches = vec![
            30 * 60,
            60 * 60,
            90 * 60,
            4 * HOUR + 30 * 60,
            5 * HOUR,
            5 * HOUR + 30 * 60,
        ];
        let sessions = vec![session_with_subagents(
            "tt",
            0,
            Some(6 * HOUR),
            dispatches.clone(),
        )];
        let entries = vec![entry("tt", 2 * HOUR, Some(4 * HOUR), &["tt", "agent"])];
        let flagged = unaccounted(&sessions, &[], &entries, at(6 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 2);
        assert_eq!(flagged.iter().map(|u| u.subagents).sum::<usize>(), 6);
        for item in &flagged {
            assert!(
                item.describe().contains("3 subagent dispatches"),
                "{}",
                item.describe()
            );
        }
    }

    #[test]
    fn a_still_open_session_with_no_dispatches_is_measured_to_its_bound() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let flagged = unaccounted(&sessions, &[], &[], at(114 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!(
            flagged[0].end,
            at(121 * 60),
            "start plus the unvouched grace"
        );
    }

    /// The bound is the last dispatch plus the gap grace, and that trailing
    /// grace is itself one idle hole, so the row stops at the dispatch.
    #[test]
    fn a_still_open_session_reports_nothing_past_its_last_dispatch() {
        let dispatches = vec![30 * 60, 60 * 60, 90 * 60];
        let sessions = vec![session_with_subagents("tt", 0, None, dispatches)];
        let flagged = unaccounted(&sessions, &[], &[], at(200 * HOUR), 60);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].end, at(90 * 60));
    }

    #[test]
    fn a_session_younger_than_its_grace_is_measured_to_now() {
        let dispatches = vec![30 * 60, 60 * 60, 90 * 60];
        let sessions = vec![session_with_subagents("tt", 0, None, dispatches)];
        let flagged = unaccounted(&sessions, &[], &[], at(100 * 60), 60);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].end, at(100 * 60));
        assert!(!flagged[0].abandoned, "its grace has not run out yet");
    }

    #[test]
    fn a_bounded_window_a_mark_cuts_into_drops_under_the_floor() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let marks = vec![mark("tt", HOUR)];
        assert!(
            unaccounted(&sessions, &marks, &[], at(121 * 60), FLOOR).is_empty(),
            "an hour of the 121-minute bound is left, under the 120-minute floor"
        );
    }

    #[test]
    fn a_row_reads_since_today_and_dated_before_today() {
        let row = Unaccounted {
            project: "tt".to_string(),
            session_id: "abcdef0123".to_string(),
            start: at(0),
            end: at(HOUR),
            subagents: 0,
            abandoned: false,
        };
        let expected = at(0).format("%-d %b, %H:%M").to_string();
        let text = row.describe();
        assert!(
            text.contains(&expected) && !text.contains("since"),
            "{text}"
        );

        let midnight = Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let row = Unaccounted {
            start: midnight,
            end: midnight + chrono::Duration::hours(1),
            ..row
        };
        assert!(
            row.describe().contains("since 00:00 ("),
            "{}",
            row.describe()
        );
    }

    #[test]
    fn only_the_trailing_row_of_a_bounded_session_is_marked_abandoned() {
        let sessions = vec![session(Some("smoke"), 0, None, 0)];
        let entries = vec![entry("smoke", 30 * 60, Some(31 * 60), &["smoke", "agent"])];
        let flagged = unaccounted(&sessions, &[], &entries, at(114 * HOUR), 60);
        assert_eq!(
            flagged
                .iter()
                .map(|u| (u.end, u.abandoned))
                .collect::<Vec<_>>(),
            vec![(at(61 * 60), true), (at(30 * 60), false)]
        );
        assert!(
            flagged[0].describe().ends_with(" [abandoned]"),
            "{}",
            flagged[0].describe()
        );
    }

    /// The bound sits a grace past the last dispatch and `gaps_over` cuts that
    /// grace, so the trailing row ends at the dispatch — and is still the row
    /// the session was abandoned at.
    #[test]
    fn the_trailing_row_of_a_bounded_session_with_dispatches_is_marked_abandoned() {
        let sessions = vec![session_with_subagents("tt", 0, None, vec![30 * 60])];
        let flagged = unaccounted(&sessions, &[], &[], at(200 * HOUR), 30);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].end, at(30 * 60));
        assert!(
            flagged[0].describe().ends_with(" [abandoned]"),
            "{}",
            flagged[0].describe()
        );
    }

    #[test]
    fn a_closed_session_reports_no_abandoned_row() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert!(!flagged[0].abandoned);
        assert!(!flagged[0].describe().contains("[abandoned]"));
    }

    #[test]
    fn a_still_open_entry_is_measured_to_now_when_checking_coverage() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let entries = vec![entry("tt", 0, None, &["tt", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    /// Two same-project sessions in one stretch of clock time are two agents,
    /// so each reports its own row and the two sum.
    #[test]
    fn two_overlapping_sessions_report_a_row_each() {
        let sessions = vec![
            session(Some("tt"), 0, Some(3 * HOUR), 0),
            Session {
                id: "sess-2".to_string(),
                ..session(Some("tt"), 0, None, 0)
            },
        ];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(
            flagged.iter().map(|u| (u.start, u.end)).collect::<Vec<_>>(),
            vec![(at(0), at(3 * HOUR)), (at(0), at(121 * 60))]
        );
        assert_eq!(
            flagged
                .iter()
                .map(|u| u.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-1", "sess-2"],
            "each row names the session it came from"
        );
        assert_ne!(
            flagged[0].describe(),
            flagged[1].describe(),
            "two rows of one project must read differently"
        );
    }

    #[test]
    fn two_overlapping_sessions_keep_their_own_dispatches() {
        let sessions = vec![
            session_with_subagents(
                "tt",
                0,
                Some(3 * HOUR),
                (1..9).map(|i| i * 20 * 60).collect(),
            ),
            session(Some("tt"), 0, None, 0),
        ];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(
            flagged
                .iter()
                .map(|u| (u.start, u.end, u.subagents))
                .collect::<Vec<_>>(),
            vec![(at(0), at(3 * HOUR), 8), (at(0), at(121 * 60), 0)]
        );
    }

    /// `resolve` stamps the session on its entry, so covering one agent's row
    /// leaves a concurrent agent's row over the same minutes to be resolved too.
    #[test]
    fn an_entry_naming_a_session_covers_that_session_only() {
        let sessions = vec![
            session(Some("tt"), 0, Some(3 * HOUR), 0),
            Session {
                id: "sess-2".to_string(),
                ..session(Some("tt"), 0, Some(3 * HOUR), 0)
            },
        ];
        let entries = vec![entry_for(
            "sess-1",
            "tt",
            0,
            Some(3 * HOUR),
            &["tt", "agent"],
        )];
        let flagged = unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR);
        assert_eq!(
            flagged
                .iter()
                .map(|u| u.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-2"]
        );
    }

    /// An entry that names no session — `item`, `--auto-log`, a hand-written
    /// row — keeps covering every session of its project.
    #[test]
    fn an_entry_naming_no_session_covers_every_session_of_the_project() {
        let sessions = vec![
            session(Some("tt"), 0, Some(3 * HOUR), 0),
            Session {
                id: "sess-2".to_string(),
                ..session(Some("tt"), 0, Some(3 * HOUR), 0)
            },
        ];
        let entries = vec![entry("tt", 0, Some(3 * HOUR), &["tt", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_dismissal_clears_the_session_it_names_and_no_other() {
        let sessions = vec![
            session(Some("tt"), 0, Some(3 * HOUR), 0),
            Session {
                id: "sess-2".to_string(),
                ..session(Some("tt"), 0, Some(3 * HOUR), 0)
            },
        ];
        let dismissals = vec![dismissal("tt", "sess-1", 0, 3 * HOUR)];
        let flagged = dismissing(&dismissals, &sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(
            flagged
                .iter()
                .map(|u| u.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-2"],
            "a concurrent session's row over the same minutes must survive"
        );
    }

    /// The subtraction runs before the floor sum, so a dismissed stretch never
    /// counts toward the session's own total either.
    #[test]
    fn a_dismissed_stretch_does_not_count_toward_the_floor() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let dismissals = vec![dismissal("tt", "sess-1", HOUR, 3 * HOUR)];
        assert!(dismissing(&dismissals, &sessions, &[], &[], at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_dismissal_of_another_project_does_not_subtract() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let dismissals = vec![dismissal("other", "sess-1", 0, 3 * HOUR)];
        assert_eq!(
            dismissing(&dismissals, &sessions, &[], &[], at(3 * HOUR), FLOOR).len(),
            1
        );
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

    // --- idle-gap subtraction (issue #26) -----------------------------

    fn session_with_subagents(
        project: &str,
        start: i64,
        end: Option<i64>,
        subagent_at: Vec<i64>,
    ) -> Session {
        Session {
            id: "sess-1".to_string(),
            project: Some(project.to_string()),
            start,
            end,
            subagents: subagent_at.len(),
            subagent_at,
        }
    }

    #[test]
    fn a_window_with_no_subagent_dispatches_is_reported_whole() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!((flagged[0].start, flagged[0].end), (at(0), at(3 * HOUR)));
    }

    #[test]
    fn a_window_active_for_ten_minutes_is_under_the_floor() {
        let sessions = vec![session_with_subagents(
            "tt",
            0,
            Some(3 * HOUR),
            vec![10 * 60],
        )];
        assert!(unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_fragment_is_split_at_its_idle_holes() {
        let minute = 60;
        let dispatches: Vec<i64> = [30, 60, 90, 120, 180, 210, 240, 270, 330, 360, 390, 420, 450]
            .iter()
            .map(|m| m * minute)
            .collect();
        let sessions = vec![session_with_subagents("tt", 0, Some(8 * HOUR), dispatches)];
        let flagged = unaccounted(&sessions, &[], &[], at(8 * HOUR), FLOOR);
        assert_eq!(
            flagged.iter().map(|u| (u.start, u.end)).collect::<Vec<_>>(),
            vec![
                (at(330 * minute), at(480 * minute)),
                (at(180 * minute), at(270 * minute)),
                (at(0), at(120 * minute)),
            ]
        );
    }

    /// The two holes of the split above, once entries cover the active rows.
    #[test]
    fn a_hole_left_by_a_covering_entry_is_never_reported() {
        let minute = 60;
        let dispatches: Vec<i64> = [30, 60, 90, 120, 180, 210, 240, 270, 330, 360, 390, 420, 450]
            .iter()
            .map(|m| m * minute)
            .collect();
        let sessions = vec![session_with_subagents("tt", 0, Some(8 * HOUR), dispatches)];
        let entries = vec![
            entry("tt", 0, Some(120 * minute), &["tt", "auto"]),
            entry("tt", 180 * minute, Some(270 * minute), &["tt", "auto"]),
            entry("tt", 330 * minute, Some(480 * minute), &["tt", "auto"]),
        ];
        assert!(unaccounted(&sessions, &[], &entries, at(8 * HOUR), 60).is_empty());
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
