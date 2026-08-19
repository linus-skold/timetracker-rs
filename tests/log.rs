//! `tt log` end to end, in a sandbox. The store and stdout are asserted against
//! each other, never separately: the message states the span stored, not requested.

mod common;

use common::Case;

fn printed_minutes(stdout: &str) -> i64 {
    let tail = stdout
        .split("- Duration: ")
        .nth(1)
        .unwrap_or_else(|| panic!("no logged duration in {stdout:?}"));
    let (hours, rest) = tail.split_once('h').expect("`<h>h <m>m`");
    let (minutes, _) = rest.trim_start().split_once('m').expect("`<h>h <m>m`");
    hours.trim().parse::<i64>().unwrap() * 60 + minutes.trim().parse::<i64>().unwrap()
}

#[test]
fn trim_reports_the_span_it_stored_and_not_the_span_it_was_asked_for() {
    let case = Case::new("log-trim-message");
    // `tt log` back-dates from its own `now`, so the hole is expressed relative to now.
    let now = common::now();
    let hole = (now - 40 * 60, now - 20 * 60);

    let run = case.run_bare(&[
        "log",
        "-d",
        "an hour with a hole in it",
        "-t",
        "60m",
        &format!("--idle={}-{}", hole.0, hole.1),
        "--trim",
    ]);
    run.assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 2, "the hole is interior, so it cuts in two");
    let stored: i64 = entries.iter().map(|entry| entry.seconds()).sum();
    // One second of slack per piece: each is back-dated a moment after this test read
    // the clock, so `num_seconds` truncates a fraction away. The *sum* is exact.
    assert!(
        (40 * 60 - entries.len() as i64..=40 * 60).contains(&stored),
        "60 minutes less the 20-minute hole, got {stored}s: {entries:?}"
    );
    assert_eq!(
        printed_minutes(&run.stdout),
        40,
        "the message states what the store holds, not the 60m it was asked for: {:?}",
        run.stdout
    );
}

#[test]
fn a_plain_log_still_reports_the_span_it_was_asked_for() {
    let case = Case::new("log-plain-message");

    let run = case.run_bare(&["log", "-d", "no holes here", "-t", "45m"]);
    run.assert_status(0);
    assert_eq!(printed_minutes(&run.stdout), 45);
    assert_eq!(case.store().entries[0].seconds(), 45 * 60);
}

#[test]
fn idle_covering_the_whole_span_leaves_the_entry_and_reports_it_whole() {
    // `trim_spans` declines rather than deleting the entry, so nothing is subtracted.
    let case = Case::new("log-trim-declines");
    let now = common::now();

    let run = case.run_bare(&[
        "log",
        "-d",
        "swallowed whole",
        "-t",
        "30m",
        &format!("--idle={}-{}", now - 90 * 60, now + 90 * 60),
        "--trim",
    ]);
    run.assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    assert_eq!(entries[0].seconds(), 30 * 60);
    assert_eq!(printed_minutes(&run.stdout), 30);
}
