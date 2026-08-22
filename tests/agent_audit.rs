//! `tt agent audit` and `tt agent audit --auto-log`, end to end through the
//! real binary and a sandboxed store/marks/activity directory. See
//! docs/decisions/0001-agent-activity-tracking.md and
//! docs/decisions/0002-auto-logging-unaccounted-activity.md.

mod common;
use common::{Case, now};

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
    case.write_session("sess-1", "smoke", start, None);

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
    case.write_session("sess-1", "smoke", start, None);

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
