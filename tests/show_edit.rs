//! `tt show`, `tt edit` and the `tt today` table, end to end in a sandbox.

mod common;

use common::{Case, Run, StoreRow};
use serde_json::{Value, json};

/// Two finished entries, started recently so both fall on today.
fn seeded(name: &str) -> Case {
    let case = Case::new(name);
    let now = common::now();
    case.write_store(&[
        StoreRow {
            description: "first task",
            project: Some("tt"),
            tags: &["tt", "impl"],
            start: now - 120,
            end: Some(now - 90),
        },
        StoreRow {
            description: "second task",
            project: None,
            tags: &[],
            start: now - 60,
            end: Some(now - 30),
        },
    ]);
    case
}

fn shown(case: &Case, reference: &str) -> Value {
    let run = case.run_bare(&["show", reference]);
    run.assert_status(0);
    serde_json::from_str(&run.stdout).expect("show prints JSON")
}

fn edit(case: &Case, reference: Option<&str>, patch: &Value) -> Run {
    let patch = patch.to_string();
    let mut args = vec!["edit"];
    args.extend(reference);
    args.extend(["--json", &patch]);
    case.run_bare(&args)
}

fn assert_edit_fails(run: &Run, needle: &str) {
    assert_ne!(run.status, Some(0), "stdout: {:?}", run.stdout);
    run.assert_stderr_has(needle);
}

#[test]
fn show_output_round_trips_into_edit() {
    let case = seeded("edit-round-trip");
    let before = shown(&case, "1");

    let run = edit(&case, Some("1"), &before);

    run.assert_status(0);
    assert_eq!(shown(&case, "1"), before);
    assert_eq!(
        serde_json::from_str::<Value>(&run.stdout).unwrap(),
        before,
        "edit prints the entry the way show does"
    );
}

#[test]
fn a_patch_changes_only_the_keys_it_names() {
    let case = seeded("edit-partial");
    let before = shown(&case, "1");

    edit(
        &case,
        Some("1"),
        &json!({"description": "renamed", "project": null, "tags": ["x"]}),
    )
    .assert_status(0);

    let after = shown(&case, "1");
    assert_eq!(after["description"], "renamed");
    assert_eq!(after["project"], Value::Null);
    assert_eq!(after["tags"], json!(["x"]));
    for untouched in ["id", "uuid", "start_time", "end_time", "idle", "data"] {
        assert_eq!(after[untouched], before[untouched], "{untouched}");
    }
    assert_eq!(shown(&case, "2")["description"], "second task");
}

#[test]
fn data_end_time_and_idle_can_be_set_and_cleared() {
    let case = seeded("edit-data-end");
    let now = common::now();
    let start = shown(&case, "2")["start_time"].clone();

    edit(
        &case,
        Some("2"),
        &json!({
            "data": {"pr": 7},
            "end_time": null,
            "idle": [{"start": common::stamp(now - 55), "end": common::stamp(now - 50)}]
        }),
    )
    .assert_status(0);
    let active = shown(&case, "2");
    assert_eq!(active["data"], json!({"pr": 7}));
    assert_eq!(active["end_time"], Value::Null);
    assert_eq!(active["idle"].as_array().unwrap().len(), 1);
    assert_eq!(active["start_time"], start);

    edit(&case, Some("2"), &json!({"data": null})).assert_status(0);
    assert_eq!(shown(&case, "2")["data"], Value::Null);
}

#[test]
fn the_entry_can_be_named_by_an_id_or_uuid_key_in_the_json() {
    let case = seeded("edit-json-selector");
    let uuid = shown(&case, "2")["uuid"].as_str().unwrap().to_string();

    edit(&case, None, &json!({"id": 2, "description": "by id"})).assert_status(0);
    assert_eq!(shown(&case, "2")["description"], "by id");

    edit(
        &case,
        None,
        &json!({"uuid": uuid, "description": "by uuid"}),
    )
    .assert_status(0);
    assert_eq!(shown(&case, "2")["description"], "by uuid");
    assert_eq!(shown(&case, "1")["description"], "first task");
}

#[test]
fn an_unknown_key_is_refused_by_name() {
    let case = seeded("edit-unknown-key");

    let run = edit(&case, Some("1"), &json!({"colour": "red"}));

    assert_edit_fails(&run, "colour");
}

#[test]
fn a_selector_that_points_elsewhere_is_refused() {
    let case = seeded("edit-mismatch");
    let before = shown(&case, "1");
    let other_uuid = shown(&case, "2")["uuid"].clone();

    let wrong_id = edit(&case, Some("1"), &json!({"id": 2, "description": "x"}));
    let wrong_uuid = edit(
        &case,
        Some("1"),
        &json!({"uuid": other_uuid, "description": "x"}),
    );
    let uuid_change = edit(
        &case,
        Some("1"),
        &json!({"uuid": "00000000-0000-4000-8000-000000000000", "description": "x"}),
    );

    assert_edit_fails(&wrong_id, "different entries");
    assert_edit_fails(&wrong_uuid, "different entries");
    assert_edit_fails(&uuid_change, "cannot change");
    assert_eq!(shown(&case, "1"), before, "a refused edit writes nothing");
}

#[test]
fn bad_times_and_bad_data_are_refused() {
    let case = seeded("edit-validation");
    let before = shown(&case, "1");
    let earlier = common::stamp(common::now() - 7200);

    let end_before_start = edit(&case, Some("1"), &json!({"end_time": earlier}));
    let start_after_end = edit(
        &case,
        Some("1"),
        &json!({"start_time": common::stamp(common::now())}),
    );
    let array_data = edit(&case, Some("1"), &json!({"data": [1, 2]}));
    let text_data = edit(&case, Some("1"), &json!({"data": "text"}));
    let bad_time = edit(&case, Some("1"), &json!({"start_time": "yesterday"}));

    assert_edit_fails(&end_before_start, "end_time is before start_time");
    assert_edit_fails(&start_after_end, "end_time is before start_time");
    assert_edit_fails(&array_data, "data");
    assert_edit_fails(&text_data, "data");
    assert_edit_fails(&bad_time, "start_time");
    assert_eq!(shown(&case, "1"), before);
}

#[test]
fn a_second_active_entry_is_refused() {
    let case = seeded("edit-two-active");
    edit(&case, Some("2"), &json!({"end_time": null})).assert_status(0);

    let run = edit(&case, Some("1"), &json!({"end_time": null}));

    assert_edit_fails(&run, "at most one");
}

#[test]
fn an_unknown_reference_or_no_reference_is_refused() {
    let case = seeded("edit-unknown-ref");

    let missing_id = edit(&case, Some("99"), &json!({"description": "x"}));
    let missing_uuid = edit(
        &case,
        None,
        &json!({"uuid": "00000000-0000-4000-8000-000000000000"}),
    );
    let no_selector = edit(&case, None, &json!({"description": "x"}));
    let not_an_object = case.run_bare(&["edit", "1", "--json", "[1]"]);

    assert_edit_fails(&missing_id, "no entry matches");
    assert_edit_fails(&missing_uuid, "no entry matches");
    assert_edit_fails(&no_selector, "name the entry");
    assert_edit_fails(&not_an_object, "JSON object");
}

#[test]
fn today_prints_a_table_of_uuid_title_span_and_duration() {
    let case = seeded("today-table");
    edit(&case, Some("2"), &json!({"end_time": null})).assert_status(0);
    let first = shown(&case, "1");
    let second = shown(&case, "2");

    let run = case.run_bare(&["today"]);

    run.assert_status(0);
    for header in ["ID", "Title", "Start–End", "Duration"] {
        run.assert_stdout_has(header);
    }
    run.assert_stdout_has(first["uuid"].as_str().unwrap());
    run.assert_stdout_has("first task");
    let start = |entry: &Value| {
        let stamp = entry["start_time"].as_str().unwrap();
        chrono::DateTime::parse_from_rfc3339(stamp)
            .unwrap()
            .with_timezone(&chrono::Local)
    };
    let end = chrono::DateTime::parse_from_rfc3339(first["end_time"].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Local);
    run.assert_stdout_has(&format!(
        "{}–{}",
        start(&first).format("%H:%M"),
        end.format("%H:%M")
    ));
    run.assert_stdout_has(&format!("{}– active", start(&second).format("%H:%M")));
    run.assert_stdout_has("Total:");
}

#[test]
fn idle_outside_the_entry_or_backwards_is_refused() {
    let case = seeded("edit-idle-bounds");
    let before = shown(&case, "1");
    let now = common::now();
    let gap = |from: i64, to: i64| json!({"idle": [{"start": common::stamp(from), "end": common::stamp(to)}]});

    let backwards = edit(&case, Some("1"), &gap(now - 95, now - 100));
    let before_start = edit(&case, Some("1"), &gap(now - 200, now - 100));
    let after_end = edit(&case, Some("1"), &gap(now - 100, now - 10));

    for run in [&backwards, &before_start, &after_end] {
        assert_edit_fails(run, "idle interval");
    }
    assert_eq!(shown(&case, "1"), before);
}
