//! `tt agent audit` and `tt agent audit --auto-log`, end to end through the
//! real binary and a sandboxed store/marks/activity/dismissed directory.

mod common;
use common::{Case, StoreRow, audit_start, dated_clock, now};

const HOUR: i64 = 3600;

#[test]
fn a_clean_sandbox_reports_nothing_unaccounted() {
    let case = Case::new("audit-clean");
    let run = case.run(&["audit"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");
}

#[test]
fn a_session_past_the_floor_with_no_coverage_is_reported() {
    let case = Case::new("audit-flagged");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit"]);
    run.assert_status(0);
    run.assert_stdout_has("Unaccounted agent activity");
    run.assert_stdout_has("smoke");
}

#[test]
fn a_covering_mark_removes_it_from_the_report() {
    let case = Case::new("audit-covered-by-mark");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, None);
    case.write_mark("smoke.-.impl", start - 60);

    let run = case.run(&["audit"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");
}

#[test]
fn auto_log_is_a_no_op_when_the_setting_is_unset() {
    let case = Case::new("audit-auto-log-unset");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    // Same report as a plain audit: nothing got logged.
    run.assert_stdout_has("Unaccounted agent activity");
    run.assert_stdout_has("smoke");
    assert!(
        case.store().entries.is_empty(),
        "no entry should have been written"
    );
}

#[test]
fn auto_log_writes_a_fixed_phase_auto_entry_over_the_threshold() {
    let case = Case::new("audit-auto-log-writes");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n"); // 3h, floor stays 120
    let start = now() - 4 * HOUR; // 240m, over both the 120m floor and the 180m auto-log threshold
    case.write_session("sess-1", "smoke", start, Some(now()));

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");

    let store = case.store();
    assert_eq!(store.entries.len(), 1);
    let entry = &store.entries[0];
    assert_eq!(entry.description, "unattended activity");
    assert_eq!(entry.project.as_deref(), Some("smoke"));
    assert_eq!(entry.tags, vec!["auto".to_string()]);
    assert!(
        !entry.tags.contains(&"agent".to_string()),
        "an auto-logged entry must never carry #agent"
    );
}

/// An auto-logged entry pins `ended_at`, so rounding its span would push the
/// start back into the idle gap the audit just judged.
#[test]
fn auto_log_never_rounds_even_with_round_minutes_set() {
    let case = Case::new("audit-auto-log-unrounded");
    case.write_config(
        "[agent]\nmax_unvouched_minutes = 20\nauto_log_after_minutes = 21\nround_minutes = 5\n",
    );
    let start = now() - 47 * 60;
    case.write_session("sess-1", "smoke", start, Some(now()));

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);

    let store = case.store();
    assert_eq!(store.entries.len(), 1);
    assert_eq!(
        store.entries[0].seconds(),
        47 * 60,
        "the unaccounted window is logged as measured"
    );
}

#[test]
fn a_window_under_the_auto_log_threshold_is_reported_but_not_logged() {
    let case = Case::new("audit-auto-log-under-threshold");
    case.write_config("[agent]\nauto_log_after_minutes = 300\n"); // 5h
    let start = now() - 3 * HOUR; // over the 120m floor, under the 300m auto-log threshold
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    run.assert_stdout_has("Unaccounted agent activity");
    run.assert_stdout_has("smoke");
    assert!(
        case.store().entries.is_empty(),
        "under the auto-log threshold: reported, never logged"
    );
}

#[test]
fn a_misconfigured_threshold_at_or_under_the_floor_disables_auto_log() {
    let case = Case::new("audit-auto-log-misconfigured");
    // Not strictly greater than the (default 120m) floor: must disable, not clamp.
    case.write_config("[agent]\nauto_log_after_minutes = 120\n");
    let start = now() - 4 * HOUR;
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    run.assert_stdout_has("smoke");
    assert!(
        case.store().entries.is_empty(),
        "a misconfigured threshold must fail toward off, not auto-log anyway"
    );
}

#[test]
fn running_auto_log_twice_logs_the_window_once() {
    let case = Case::new("audit-auto-log-idempotent");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let start = now() - 4 * HOUR;
    case.write_session("sess-1", "smoke", start, Some(now()));

    case.run(&["audit", "--auto-log"]).assert_status(0);
    assert_eq!(case.store().entries.len(), 1, "first run logs one entry");

    let second = case.run(&["audit", "--auto-log"]);
    second.assert_status(0);
    second.assert_stdout_has("No unaccounted agent activity.");
    assert_eq!(
        case.store().entries.len(),
        1,
        "the second run must not log a duplicate"
    );
}

/// Two orchestrators working one project in the same minute are two agents,
/// and the audit must show the sum of their work.
#[test]
fn two_overlapping_same_project_sessions_report_a_row_each() {
    let case = Case::new("audit-two-sessions");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "twin", start, Some(now()));
    case.write_session("sess-2", "twin", start, Some(now()));

    let run = case.run(&["audit"]);
    run.assert_status(0);
    assert_eq!(
        run.stdout.lines().filter(|l| l.contains("twin")).count(),
        2,
        "{:?}",
        run.stdout
    );
}

/// An open session with no dispatch to lean on vouches for the unvouched
/// grace past its start and no further, however long ago that start was.
#[test]
fn an_open_session_with_no_dispatches_reports_one_abandoned_row_at_its_grace() {
    let case = Case::new("audit-abandoned");
    let start = now() - 3 * 24 * HOUR;
    case.write_session("sess-1", "stalled", start, None);

    let run = case.run(&["audit"]);
    run.assert_status(0);
    let rows: Vec<&str> = run
        .stdout
        .lines()
        .filter(|l| l.contains("stalled"))
        .collect();
    assert_eq!(rows.len(), 1, "{:?}", run.stdout);
    assert!(
        rows[0].contains(&dated_clock(start))
            && !rows[0].contains("since")
            && rows[0].contains("(2h 1m)")
            && rows[0].ends_with("[abandoned]"),
        "{}",
        rows[0]
    );
}

/// The last dispatch plus the gap grace bounds the window, and the trailing
/// grace is itself an idle hole, so the entry stops at the dispatch.
#[test]
fn auto_log_writes_the_bounded_window_of_an_abandoned_session() {
    let case = Case::new("audit-abandoned-auto-log");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let start = now() - 3 * 24 * HOUR;
    let dispatches: Vec<i64> = (1..=10).map(|i| start + i * 30 * 60).collect();
    case.write_session_with_dispatches("sess-1", "stalled", start, None, &dispatches);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");

    let store = case.store();
    assert_eq!(store.entries.len(), 1);
    let entry = &store.entries[0];
    assert_eq!(
        entry.seconds(),
        300 * 60,
        "the bounded window, not the run to now"
    );
    assert_eq!(
        entry.end_time.unwrap().timestamp(),
        *dispatches.last().unwrap()
    );
}

/// The rows a `--json` run parsed back, in the order the audit printed them.
fn rows(run: &common::Run) -> Vec<serde_json::Value> {
    serde_json::from_str(&run.stdout).unwrap_or_else(|err| panic!("{err}: {:?}", run.stdout))
}

#[test]
fn json_carries_each_row_s_session_and_exact_epochs() {
    let case = Case::new("audit-json");
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("sess-1", "smoke", start, Some(end));

    let run = case.run(&["audit", "--json"]);
    run.assert_status(0);
    let rows = rows(&run);
    assert_eq!(rows.len(), 1, "{:?}", run.stdout);
    assert_eq!(rows[0]["session"], "sess-1");
    assert_eq!(rows[0]["project"], "smoke");
    assert_eq!(rows[0]["start"], start);
    assert_eq!(rows[0]["end"], end);
    assert_eq!(rows[0]["abandoned"], false);
    assert_eq!(rows[0]["subagents"], 0);
}

#[test]
fn json_is_an_empty_array_when_nothing_is_unaccounted() {
    let case = Case::new("audit-json-clean");
    let run = case.run(&["audit", "--json"]);
    run.assert_status(0);
    assert_eq!(run.stdout.trim(), "[]");
}

#[test]
fn project_keeps_that_project_s_rows_and_drops_the_rest() {
    let case = Case::new("audit-project");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "mine", start, Some(now()));
    case.write_session("sess-2", "theirs", start, Some(now()));

    let run = case.run(&["audit", "--project", "mine"]);
    run.assert_status(0);
    run.assert_stdout_has("mine");
    assert!(!run.stdout.contains("theirs"), "{:?}", run.stdout);

    let rows = rows(&case.run(&["audit", "--json", "--project", "theirs"]));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["project"], "theirs");
}

/// The floor is a warning threshold, not a filter on the ledger: a sweep that
/// logs most of a session leaves the rest addressable.
#[test]
fn a_session_under_the_floor_is_silent_in_text_but_listed_in_json() {
    let case = Case::new("audit-floor-json");
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("sess-1", "smoke", start, Some(end));
    // Covers everything but the first 50 minutes, leaving a total under the floor.
    case.write_store(&[StoreRow {
        description: "swept the rest",
        project: Some("smoke"),
        tags: &["impl", "agent"],
        start: start + 50 * 60,
        end: Some(end),
    }]);

    let text = case.run(&["audit"]);
    text.assert_status(0);
    text.assert_stdout_has("No unaccounted agent activity.");

    let listed = rows(&case.run(&["audit", "--json"]));
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["start"], start);
    assert_eq!(listed[0]["end"], start + 50 * 60);

    let resolved = case.run(&[
        "resolve",
        "--session",
        "sess-1",
        &start.to_string(),
        "-",
        "impl",
        "the rest of it",
    ]);
    resolved.assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].seconds(), 50 * 60);
    assert_eq!(rows(&case.run(&["audit", "--json"])).len(), 0);
}

/// An auto-logged entry names the session it was written for, so it cannot
/// silence a concurrent same-project session's row.
#[test]
fn auto_log_covers_only_the_session_it_wrote_for() {
    let case = Case::new("audit-auto-log-session");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let end = now();
    // Over the threshold, so it is auto-logged; and under it, so it is not.
    case.write_session("sess-1", "twin", end - 4 * HOUR, Some(end));
    case.write_session("sess-2", "twin", end - 3 * HOUR, Some(end));

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    assert_eq!(case.store().entries.len(), 1, "only the longer window");

    let left = rows(&case.run(&["audit", "--json"]));
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["session"], "sess-2");
}

/// `--json` promises one array on stdout, so a write's confirmation cannot
/// share the stream.
#[test]
fn json_stays_parseable_when_auto_log_writes_in_the_same_run() {
    let case = Case::new("audit-json-auto-log");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let start = now() - 4 * HOUR;
    case.write_session("sess-1", "smoke", start, Some(now()));

    let run = case.run(&["audit", "--json", "--auto-log"]);
    run.assert_status(0);
    assert_eq!(run.stdout.trim(), "[]", "stdout: {:?}", run.stdout);
    assert!(
        run.stderr.contains("Logged:"),
        "the confirmation belongs on stderr: {:?}",
        run.stderr
    );
    assert_eq!(case.store().entries.len(), 1);
}

/// A row is addressed by the sanitised key its session file is named with,
/// whichever form the caller holds.
#[test]
fn resolve_matches_a_session_whose_id_needed_sanitising() {
    let case = Case::new("audit-resolve-sanitised");
    let start = now() - 3 * HOUR;
    case.write_session("weird_id", "smoke", start, Some(now()));

    let run = case.run(&[
        "resolve",
        "--session",
        "weird/id",
        &start.to_string(),
        "-",
        "impl",
        "the same row",
    ]);
    run.assert_status(0);
    assert_eq!(case.store().entries.len(), 1);
}

#[test]
fn dismiss_refuses_a_span_that_ends_where_it_starts() {
    let case = Case::new("audit-dismiss-empty-span");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, Some(now()));

    let run = case.run(&[
        "dismiss",
        "--session",
        "sess-1",
        "smoke",
        &format!("{start}-{start}"),
    ]);
    run.assert_status(64);
    assert_eq!(common::count_files(&case.dismissed), 0);
    assert!(!run.stdout.contains("dismissed"), "{:?}", run.stdout);
}

/// `--project` bounds what a run may **write**, not only what it prints.
#[test]
fn auto_log_with_a_project_never_writes_another_project_s_window() {
    let case = Case::new("audit-auto-log-project");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let start = now() - 4 * HOUR;
    case.write_session("sess-1", "theirs", start, Some(now()));

    let run = case.run(&["audit", "--auto-log", "--project", "mine"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");
    assert!(
        case.store().entries.is_empty(),
        "another project's window must never be billed by this run"
    );

    // Still there, and still unaccounted, for its own project's sweep.
    let theirs = case.run(&["audit", "--project", "theirs"]);
    theirs.assert_stdout_has("theirs");
}

/// A session id is addressed by the sanitised key its file is named with, so a
/// raw id that sanitises differently still clears the row it names.
#[test]
fn a_dismissal_matches_a_session_whose_id_needed_sanitising() {
    let case = Case::new("audit-dismiss-sanitised");
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("weird_id", "smoke", start, Some(end));

    case.run(&[
        "dismiss",
        "--session",
        "weird/id",
        "smoke",
        &format!("{start}-{end}"),
    ])
    .assert_status(0);

    let after = case.run(&["audit"]);
    after.assert_status(0);
    after.assert_stdout_has("No unaccounted agent activity.");
}

#[test]
fn dismissing_a_flagged_window_clears_it_without_billing_the_time() {
    let case = Case::new("audit-dismiss");
    case.write_store(&[]);
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("sess-1", "smoke", start, Some(end));
    case.run(&["audit"]).assert_stdout_has("smoke");

    let dismissed = case.run(&[
        "dismiss",
        "--session",
        "sess-1",
        "smoke",
        &format!("{start}-{end}"),
        "a long lunch",
    ]);
    dismissed.assert_status(0);

    let after = case.run(&["audit"]);
    after.assert_status(0);
    after.assert_stdout_has("No unaccounted agent activity.");
    assert!(
        case.store().entries.is_empty(),
        "a dismissal must never bill the time"
    );
    assert_eq!(common::count_files(&case.dismissed), 1);
}

#[test]
fn a_dismissal_leaves_a_concurrent_session_s_row_alone() {
    let case = Case::new("audit-dismiss-concurrent");
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("sess-1", "twin", start, Some(end));
    case.write_session("sess-2", "twin", start, Some(end));

    case.run(&[
        "dismiss",
        "--session",
        "sess-1",
        "twin",
        &format!("{start}-{end}"),
    ])
    .assert_status(0);

    let rows = rows(&case.run(&["audit", "--json"]));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["session"], "sess-2");
}

#[test]
fn resolve_covers_the_row_it_was_given_exactly() {
    let case = Case::new("audit-resolve");
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("sess-1", "smoke", start, Some(end));

    let run = case.run(&[
        "resolve",
        "--session",
        "sess-1",
        &start.to_string(),
        "7",
        "impl",
        "reconstructed the reconcile",
    ]);
    run.assert_status(0);

    let store = case.store();
    assert_eq!(store.entries.len(), 1);
    let entry = &store.entries[0];
    assert_eq!(entry.start_time.timestamp(), start);
    assert_eq!(entry.end_time.unwrap().timestamp(), end);
    assert_eq!(entry.project.as_deref(), Some("smoke"));
    assert_eq!(entry.description, "reconstructed the reconcile");
    assert_eq!(entry.tags, ["smoke/7", "impl", "agent"]);

    let after = case.run(&["audit"]);
    after.assert_status(0);
    after.assert_stdout_has("No unaccounted agent activity.");
}

/// Two orchestrators on one project are two rows, two resolutions and two
/// entries over the same minutes; the report shows the sum.
#[test]
fn two_overlapping_sessions_resolve_to_two_entries_that_sum() {
    let case = Case::new("audit-resolve-twins");
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("sess-1", "twin", start, Some(end));
    case.write_session("sess-2", "twin", start, Some(end));

    for session in ["sess-1", "sess-2"] {
        case.run(&[
            "resolve",
            "--session",
            session,
            &start.to_string(),
            "-",
            "impl",
            "worked the same minutes",
        ])
        .assert_status(0);
    }

    let store = case.store();
    assert_eq!(store.entries.len(), 2);
    for entry in &store.entries {
        assert_eq!(entry.start_time.timestamp(), start);
        assert_eq!(entry.end_time.unwrap().timestamp(), end);
    }
    let sessions: Vec<&str> = store
        .entries
        .iter()
        .map(|entry| {
            entry.data.as_ref().unwrap()["agent"]["session"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(sessions, vec!["sess-1", "sess-2"]);

    case.run(&["audit"])
        .assert_stdout_has("No unaccounted agent activity.");

    let report = case.run_bare(&["report", "--project", "twin"]);
    report.assert_status(0);
    report.assert_stdout_has("6h 0m");
}

/// An entry that names no session covers every session of its project, so one
/// `item` still clears both rows.
#[test]
fn an_item_entry_naming_no_session_covers_both_overlapping_sessions() {
    let case = Case::new("audit-item-covers-both");
    let start = now() - 3 * HOUR;
    let end = now();
    case.write_session("sess-1", "twin", start, Some(end));
    case.write_session("sess-2", "twin", start, Some(end));

    case.run(&["item", "twin", "-", "impl", "one entry for the lot", "180"])
        .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(entry.data, None, "item names no session");
    case.run(&["audit"])
        .assert_stdout_has("No unaccounted agent activity.");
}

/// A rounded-up duration would reach back past the row's start, the reason
/// `--auto-log` bypasses rounding too.
#[test]
fn resolve_never_rounds_even_with_round_minutes_set() {
    let case = Case::new("audit-resolve-unrounded");
    case.write_config("[agent]\nmax_unvouched_minutes = 20\nround_minutes = 30\n");
    let start = now() - 47 * 60;
    let end = now();
    case.write_session("sess-1", "smoke", start, Some(end));

    case.run(&[
        "resolve",
        "--session",
        "sess-1",
        &start.to_string(),
        "-",
        "impl",
        "measured, not rounded",
    ])
    .assert_status(0);

    let store = case.store();
    assert_eq!(store.entries[0].seconds(), 47 * 60);
}

#[test]
fn resolve_on_a_pair_matching_no_row_exits_64_and_writes_nothing() {
    let case = Case::new("audit-resolve-unmatched");
    case.write_store(&[]);
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, Some(now()));

    let run = case.run(&[
        "resolve",
        "--session",
        "sess-2",
        &start.to_string(),
        "-",
        "impl",
        "not my row",
    ]);
    run.assert_status(64);
    assert!(case.store().entries.is_empty());

    let wrong_start = case.run(&[
        "resolve",
        "--session",
        "sess-1",
        &(start + 5).to_string(),
        "-",
        "impl",
        "not my row",
    ]);
    wrong_start.assert_status(64);
    assert!(case.store().entries.is_empty());
}

/// The minutes an 8h session with two 60-minute idle holes reports, as three
/// contiguous active stretches. Counted against the dispatches below it: a
/// removed dispatch leaves a 60m hole between its neighbours.
const ROWS: [(i64, i64); 3] = [(0, 120), (180, 270), (330, 480)];

/// Every 30 minutes over the 8h window, minus the one at 150m and the one at
/// 300m.
fn dispatches(start: i64) -> Vec<i64> {
    (1..16)
        .map(|i| i * 30)
        .filter(|m| ![150, 300].contains(m))
        .map(|m| start + m * 60)
        .collect()
}

/// `Lease::close_command` prints the mark's sanitised project, so the entry an
/// operator logs by following a `[stale]` row has to be one the audit's entry
/// join accepts.
#[test]
fn an_entry_logged_with_the_printed_project_silences_the_next_check() {
    let case = Case::new("audit-close-line-covers");
    let start = now() - 5 * HOUR;
    case.write_session("sess-1", "my proj", start, Some(now()));
    case.write_mark("my_proj.-.impl", start);

    let flagged = case.run(&["activity", "check", "sess-1"]);
    flagged.assert_status(0);
    flagged.assert_stdout_has("my proj");

    // The close line the stale row prints, run verbatim.
    let listed = case.run(&["list", "my_proj"]);
    listed.assert_stdout_has("tt agent end my_proj - impl \"<summary>\" <minutes>");
    case.run(&["end", "my_proj", "-", "impl", "did the thing", "300"])
        .assert_status(0);

    let silenced = case.run(&["activity", "check", "sess-1"]);
    silenced.assert_status(0);
    assert_eq!(
        silenced.stdout, "",
        "the entry the printed close line logs must cover the fragment it was printed for"
    );
}

/// The floor gates the session's total, so fragments under it are reported —
/// and `auto_log_after_minutes`, which must exceed the floor, still gates each
/// fragment on its own.
#[test]
fn fragments_under_the_floor_are_reported_but_never_auto_logged() {
    let case = Case::new("audit-fragment-floor");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let start = now() - 6 * HOUR;
    case.write_session("sess-1", "smoke", start, Some(now()));
    // Two unvouched marks, each covering the two hours after it was opened,
    // leaving a 50m head and an 80m middle.
    case.write_mark("smoke.1.impl", start + 50 * 60);
    case.write_mark("smoke.2.impl", start + 4 * HOUR + 10 * 60);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    assert_eq!(
        run.stdout.lines().filter(|l| l.contains("smoke")).count(),
        2,
        "both fragments are under the floor and both belong in the report: {:?}",
        run.stdout
    );
    assert!(
        case.store().entries.is_empty(),
        "a fragment under auto_log_after_minutes must never be written"
    );
}

/// An idle hole is never reported: the rows are the active stretches around
/// it, and an `#auto` entry spans exactly the row it was written for, so no
/// later audit can flag the hole.
#[test]
fn a_session_with_two_idle_holes_reports_three_active_rows() {
    let case = Case::new("audit-idle-holes");
    // The floor and the auto-log threshold both under the shortest row, so
    // every row clears them.
    case.write_config("[agent]\nmax_unvouched_minutes = 60\nauto_log_after_minutes = 61\n");
    let start = now() - 8 * HOUR;
    case.write_session_with_dispatches(
        "sess-1",
        "repro",
        start,
        Some(start + 8 * HOUR),
        &dispatches(start),
    );

    let run = case.run(&["audit"]);
    run.assert_status(0);
    let rows: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.contains("repro"))
        .collect();
    assert_eq!(rows.len(), 3, "{:?}", run.stdout);
    for (row, (from, to)) in rows.iter().rev().zip(ROWS) {
        assert!(
            row.contains(&audit_start(start + from * 60)),
            "{row} is not the row starting at {from}m"
        );
        let minutes = to - from;
        assert!(
            row.contains(&format!("({}h {}m", minutes / 60, minutes % 60)),
            "{row} is not {minutes}m long"
        );
    }

    let logged = case.run(&["audit", "--auto-log"]);
    logged.assert_status(0);
    logged.assert_stdout_has("No unaccounted agent activity.");
    let mut spans: Vec<(i64, i64)> = case
        .store()
        .entries
        .iter()
        .map(|entry| {
            (
                entry.start_time.timestamp() - start,
                entry.end_time.unwrap().timestamp() - start,
            )
        })
        .collect();
    spans.sort();
    assert_eq!(
        spans,
        ROWS.map(|(from, to)| (from * 60, to * 60)).to_vec(),
        "each entry spans exactly the row it was written for"
    );

    let again = case.run(&["audit", "--auto-log"]);
    again.assert_status(0);
    again.assert_stdout_has("No unaccounted agent activity.");
    assert_eq!(
        case.store().entries.len(),
        3,
        "a hole left between the rows must never be re-reported or re-logged"
    );
}
