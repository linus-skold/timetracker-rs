//! `tt report` end to end in a sandbox. The rollup axis is the entry's **project
//! field**, never a tag, and every timestamp is fabricated at an absolute epoch so
//! `--week`, `--since` and `--until` are testable without a clock.

mod common;

use chrono::{Datelike, Duration, Local, NaiveDate};
use common::{Case, StoreRow, stamp};

/// Midnight-anchored epochs in the local zone, as the store's dates are local.
fn epoch_on(date: NaiveDate, hour: i64) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight exists")
        .and_local_timezone(Local)
        .single()
        .expect("an unambiguous local midnight")
        .timestamp()
        + hour * 3600
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn days_ago(n: i64) -> NaiveDate {
    today() - Duration::days(n)
}

/// One agent-shaped row: the tags a `tt agent` command writes, plus its project.
fn agent_row(
    description: &'static str,
    project: &'static str,
    tags: &'static [&'static str],
    date: NaiveDate,
    hour: i64,
    minutes: i64,
) -> StoreRow {
    let start = epoch_on(date, hour);
    StoreRow {
        description,
        project: Some(project),
        tags,
        start,
        end: Some(start + minutes * 60),
    }
}

#[test]
fn the_default_scope_reports_today_and_leaves_other_days_out() {
    let case = Case::new("report-today");
    case.write_store(&[
        agent_row(
            "did today",
            "tt",
            &["tt/12", "impl", "agent"],
            today(),
            9,
            30,
        ),
        agent_row(
            "did last week",
            "tt",
            &["tt/11", "impl", "agent"],
            days_ago(9),
            9,
            60,
        ),
    ]);

    let run = case.run_bare(&["report"]);
    run.assert_status(0);
    run.assert_stdout_has("0h 30m");
    assert!(
        run.stdout.contains("tt/12"),
        "today's item is listed: {:?}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("tt/11"),
        "a nine-day-old entry is not today's: {:?}",
        run.stdout
    );
}

#[test]
fn an_agent_shaped_entry_reports_under_its_project_and_never_under_agent() {
    let case = Case::new("report-project-axis");
    case.write_store(&[agent_row(
        "premium brand primitives",
        "vinge",
        &["vinge/12", "impl", "agent"],
        today(),
        9,
        45,
    )]);

    let run = case.run_bare(&["report"]);
    run.assert_status(0);
    assert!(
        run.stdout.contains("vinge"),
        "grouped under the project field: {:?}",
        run.stdout
    );
    // A project row is unindented, so a line *starting* with `agent` is the bug.
    assert!(
        !run.stdout.lines().any(|line| line.starts_with("agent")),
        "the provenance tag is not a project: {:?}",
        run.stdout
    );
}

#[test]
fn week_includes_the_monday_boundary_entry() {
    let case = Case::new("report-week");
    // The same arithmetic `TimeData::week_start` uses, so the boundary matches it.
    let monday = today() - Duration::days(today().weekday().num_days_from_monday() as i64);

    case.write_store(&[
        agent_row(
            "on the Monday",
            "tt",
            &["tt/1", "plan", "agent"],
            monday,
            9,
            30,
        ),
        // The day before the week started, which --week must exclude.
        agent_row(
            "the Sunday before",
            "tt",
            &["tt/2", "plan", "agent"],
            monday - Duration::days(1),
            9,
            30,
        ),
    ]);

    let run = case.run_bare(&["report", "--week"]);
    run.assert_status(0);
    assert!(run.stdout.contains("tt/1"), "{:?}", run.stdout);
    assert!(
        !run.stdout.contains("tt/2"),
        "the Sunday before is last week: {:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!("Week of {monday}")),
        "labelled by its Monday: {:?}",
        run.stdout
    );
}

#[test]
fn since_and_until_bound_the_range_at_both_ends() {
    let case = Case::new("report-since-until");
    case.write_store(&[
        agent_row(
            "too early",
            "tt",
            &["tt/1", "plan", "agent"],
            days_ago(10),
            9,
            30,
        ),
        agent_row(
            "inside",
            "tt",
            &["tt/2", "plan", "agent"],
            days_ago(5),
            9,
            30,
        ),
        agent_row(
            "too late",
            "tt",
            &["tt/3", "plan", "agent"],
            days_ago(1),
            9,
            30,
        ),
    ]);

    let run = case.run_bare(&[
        "report",
        "--since",
        &days_ago(7).to_string(),
        "--until",
        &days_ago(3).to_string(),
    ]);
    run.assert_status(0);
    assert!(run.stdout.contains("tt/2"), "{:?}", run.stdout);
    assert!(
        !run.stdout.contains("tt/1"),
        "before --since: {:?}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("tt/3"),
        "after --until: {:?}",
        run.stdout
    );
}

#[test]
fn until_without_a_scope_is_a_usage_error() {
    let case = Case::new("report-until-alone");
    case.write_store(&[]);

    let run = case.run_bare(&["report", "--until", &today().to_string()]);
    assert_ne!(run.status, Some(0), "a usage error, not a quiet day report");
}

#[test]
fn project_filters_on_the_field_not_on_a_tag() {
    let case = Case::new("report-project-filter");
    case.write_store(&[
        agent_row(
            "vinge work",
            "vinge",
            &["vinge/6", "impl", "agent"],
            today(),
            9,
            30,
        ),
        agent_row(
            "tt work",
            "tt",
            &["tt/12", "impl", "agent"],
            today(),
            11,
            45,
        ),
    ]);

    let run = case.run_bare(&["report", "--project", "vinge"]);
    run.assert_status(0);
    assert!(run.stdout.contains("vinge/6"), "{:?}", run.stdout);
    assert!(
        !run.stdout.contains("tt/12"),
        "the other project is filtered out: {:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("— #vinge"),
        "the label names the filter: {:?}",
        run.stdout
    );
}

#[test]
fn json_carries_the_documented_keys_over_the_real_binary() {
    let case = Case::new("report-json");
    case.write_store(&[
        agent_row(
            "did the thing",
            "vinge",
            &["vinge/6", "plan", "agent"],
            today(),
            9,
            30,
        ),
        StoreRow {
            description: "hand-written, no project",
            project: None,
            tags: &[],
            start: epoch_on(today(), 14),
            end: Some(epoch_on(today(), 14) + 15 * 60),
        },
    ]);

    let run = case.run_bare(&["report", "--json"]);
    run.assert_status(0);

    let json: serde_json::Value =
        serde_json::from_str(&run.stdout).expect("--json emits parseable JSON");

    assert_eq!(json["total_seconds"], 45 * 60);
    assert_eq!(json["overlaps"], 0, "09:00 and 14:00 do not collide");
    assert!(json["label"].is_string());

    let projects = json["projects"].as_object().expect("an object");
    assert!(projects.contains_key("vinge"));
    assert!(
        projects.contains_key("(no project)"),
        "the field-less bucket, not `(untagged)`: {:?}",
        projects.keys().collect::<Vec<_>>()
    );
    assert!(!projects.contains_key("agent"));

    let item = &json["projects"]["vinge"]["items"]["vinge/6"];
    assert_eq!(item["seconds"], 30 * 60);
    assert_eq!(item["phases"]["plan"], 30 * 60);
    assert_eq!(item["active"], false);
    assert!(item["seconds"].is_i64(), "seconds are integers, not floats");
}

#[test]
fn overlapping_back_dated_entries_are_warned_about_and_disjoint_ones_are_not() {
    let case = Case::new("report-overlaps");
    let start = epoch_on(today(), 9);

    case.write_store(&[
        StoreRow {
            description: "first",
            project: Some("tt"),
            tags: &["tt/1", "impl"],
            start,
            end: Some(start + 60 * 60),
        },
        StoreRow {
            description: "second, half inside the first",
            project: Some("tt"),
            tags: &["tt/2", "impl"],
            start: start + 30 * 60,
            end: Some(start + 90 * 60),
        },
    ]);

    let run = case.run_bare(&["report"]);
    run.assert_status(0);
    assert!(
        run.stdout.contains("1 overlapping span(s)"),
        "the collision is reported: {:?}",
        run.stdout
    );

    let disjoint = Case::new("report-no-overlaps");
    disjoint.write_store(&[
        agent_row("morning", "tt", &["tt/1", "impl", "agent"], today(), 9, 30),
        agent_row(
            "afternoon",
            "tt",
            &["tt/2", "impl", "agent"],
            today(),
            14,
            30,
        ),
    ]);
    let clean = disjoint.run_bare(&["report"]);
    clean.assert_status(0);
    assert!(
        !clean.stdout.contains("overlapping"),
        "nothing to warn about: {:?}",
        clean.stdout
    );
}

#[test]
fn an_empty_scope_says_so_rather_than_printing_an_empty_tree() {
    let case = Case::new("report-empty");
    case.write_store(&[agent_row(
        "long ago",
        "tt",
        &["tt/1", "impl", "agent"],
        days_ago(30),
        9,
        30,
    )]);

    let run = case.run_bare(&["report"]);
    run.assert_status(0);
    assert!(
        run.stdout.starts_with("No entries for Today,"),
        "{:?}",
        run.stdout
    );
}

#[test]
fn all_reports_every_entry_regardless_of_date() {
    let case = Case::new("report-all");
    case.write_store(&[
        agent_row(
            "ancient",
            "tt",
            &["tt/1", "impl", "agent"],
            days_ago(200),
            9,
            30,
        ),
        agent_row("today", "tt", &["tt/2", "impl", "agent"], today(), 9, 30),
    ]);

    let run = case.run_bare(&["report", "--all"]);
    run.assert_status(0);
    run.assert_stdout_has("All entries");
    assert!(run.stdout.contains("tt/1"), "{:?}", run.stdout);
    assert!(run.stdout.contains("tt/2"), "{:?}", run.stdout);
    run.assert_stdout_has("1h 0m");
}

#[test]
fn report_leaves_the_store_untouched_byte_for_byte() {
    // A read: no rewrite and no exclusive lock, so it cannot block `tt agent end`.
    let case = Case::new("report-read-only");
    case.write_store(&[agent_row(
        "did the thing",
        "tt",
        &["tt/12", "impl", "agent"],
        today(),
        9,
        30,
    )]);

    let path = case.data_dir().join("data.json");
    let before = std::fs::read(&path).expect("the fabricated store");
    let before_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();

    case.run_bare(&["report"]).assert_status(0);

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the store was rewritten by a read"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before_mtime,
        "the store's mtime moved, so something wrote it"
    );
    // And no lock file was left behind by a writer that never needed to be one.
    assert!(
        !case.data_dir().join("data.lock").exists(),
        "a read took the store's exclusive lock"
    );
}

#[test]
fn a_pre_project_store_still_reports_its_inferred_projects() {
    // `report` migrates its own in-memory copy, so a pre-`project` store rolls up
    // under the project inferred from its tags, without that being written back.
    let case = Case::new("report-pre-project-store");
    let start = epoch_on(today(), 9);
    case.write_raw_store(&format!(
        r#"{{"entries":[{{"id":1,"description":"premium brand primitives","project":null,"tags":["vinge","vinge/12","impl"],"start_time":"{}","end_time":"{}","idle":[]}}],"next_id":2,"schema_version":0}}"#,
        stamp(start),
        stamp(start + 45 * 60)
    ));

    let run = case.run_bare(&["report"]);
    run.assert_status(0);
    assert!(
        run.stdout.contains("vinge"),
        "the inferred project is the axis: {:?}",
        run.stdout
    );
    run.assert_stdout_has("0h 45m");

    // Inferred for the reader only: the file is still at version 0.
    let stored = std::fs::read_to_string(case.data_dir().join("data.json")).unwrap();
    assert!(
        stored.contains(r#""schema_version":0"#),
        "the migration was written back: {stored}"
    );
}
