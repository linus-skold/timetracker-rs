//! Entry uuids end to end, in a sandbox: the migration, fresh entries, split
//! pieces and the id/uuid lookup behind `tt show`.

mod common;

use common::{Case, StoreRow};
use std::collections::HashSet;

const OLD_STORE: &str = r#"{"entries":[
    {"id":1,"description":"one","project":"tt","tags":["tt"],"start_time":"2026-08-18T09:00:00+02:00","end_time":"2026-08-18T10:00:00+02:00","idle":[]},
    {"id":2,"description":"two","project":"tt","tags":["tt"],"start_time":"2026-08-18T11:00:00+02:00","end_time":"2026-08-18T12:00:00+02:00","idle":[]}
],"next_id":3,"schema_version":0}"#;

fn uuids(case: &Case) -> Vec<String> {
    case.store()
        .entries
        .into_iter()
        .map(|entry| entry.uuid.expect("every saved entry has a uuid"))
        .collect()
}

fn schema_version(case: &Case) -> u64 {
    let body = std::fs::read_to_string(case.data_dir().join("data.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    value["schema_version"].as_u64().unwrap()
}

fn assert_all_distinct(values: &[String]) {
    let unique: HashSet<&String> = values.iter().collect();
    assert_eq!(unique.len(), values.len(), "uuids repeat: {values:?}");
}

#[test]
fn migration_gives_every_entry_a_unique_uuid_and_is_idempotent() {
    let case = Case::new("uuid-migrate");
    case.write_raw_store(OLD_STORE);

    case.run_bare(&["list"]).assert_status(0);
    let first = uuids(&case);
    assert_eq!(first.len(), 2);
    assert_all_distinct(&first);
    assert_eq!(schema_version(&case), 2);

    case.run_bare(&["list"]).assert_status(0);
    assert_eq!(uuids(&case), first, "a second run must keep the uuids");
}

#[test]
fn migration_also_covers_a_store_already_at_version_1() {
    let case = Case::new("uuid-migrate-v1");
    let now = common::now();
    case.write_store(&[StoreRow {
        description: "old v1 row",
        project: None,
        tags: &[],
        start: now - 600,
        end: Some(now - 300),
    }]);

    case.run_bare(&["status"]).assert_status(0);

    assert_eq!(uuids(&case).len(), 1);
    assert_eq!(schema_version(&case), 2);
}

#[test]
fn new_entries_get_distinct_uuids() {
    let case = Case::new("uuid-new-entries");

    case.run_bare(&["log", "-d", "first", "-t", "10m"])
        .assert_status(0);
    case.run_bare(&["log", "-d", "second", "-t", "10m"])
        .assert_status(0);
    case.run_bare(&["start", "third"]).assert_status(0);

    let all = uuids(&case);
    assert_eq!(all.len(), 3);
    assert_all_distinct(&all);
}

#[test]
fn split_pieces_get_distinct_uuids() {
    let case = Case::new("uuid-split");
    let now = common::now();

    case.run_bare(&[
        "log",
        "-d",
        "a hole in it",
        "-t",
        "60m",
        &format!("--idle={}-{}", now - 40 * 60, now - 20 * 60),
        "--trim",
    ])
    .assert_status(0);

    let all = uuids(&case);
    assert_eq!(all.len(), 2, "the hole cuts the entry in two");
    assert_all_distinct(&all);
}

#[test]
fn show_finds_an_entry_by_id_by_uuid_and_by_uppercase_uuid() {
    let case = Case::new("uuid-lookup");
    case.write_raw_store(OLD_STORE);
    case.run_bare(&["list"]).assert_status(0);
    let second = uuids(&case)[1].clone();

    let by_id = case.run_bare(&["show", "2"]);
    let by_uuid = case.run_bare(&["show", &second]);
    let by_upper = case.run_bare(&["show", &second.to_uppercase()]);

    by_id.assert_status(0);
    by_id.assert_stdout_has("\"description\": \"two\"");
    assert_eq!(by_uuid.stdout, by_id.stdout);
    assert_eq!(by_upper.stdout, by_id.stdout);
}

#[test]
fn show_fails_for_an_unknown_reference() {
    let case = Case::new("uuid-unknown");
    case.write_raw_store(OLD_STORE);

    for reference in ["99", "not-a-uuid", "00000000-0000-4000-8000-000000000000"] {
        let run = case.run_bare(&["show", reference]);
        assert_ne!(run.status, Some(0), "{reference}");
        run.assert_stderr_has("no entry matches");
    }
}
