//! Export and import against a sandboxed store, never the real one.

use super::*;
use crate::storage::{env_guard, env_sandbox, load_data, with_data};
use chrono::Duration;

fn seed(data: &mut TimeData, description: &str, running: bool) {
    let start = Local::now() - Duration::hours(2);
    let end = (!running).then(|| start + Duration::hours(1));
    data.add_entry(
        description.to_string(),
        Some("p".into()),
        vec![],
        start,
        end,
    );
}

fn exported_text(descriptions: &[&str]) -> String {
    let mut source = TimeData {
        schema_version: SCHEMA_VERSION,
        ..TimeData::default()
    };
    for description in descriptions {
        seed(&mut source, description, false);
    }
    source.fill_missing_uuids();
    render_export(&source, Local::now()).unwrap()
}

fn import_text(text: &str, replace: bool) -> Result<ImportSummary> {
    let incoming = parse_export(text)?;
    with_data(|data| apply_import(data, incoming, replace))
}

#[test]
fn export_then_import_into_an_empty_store_keeps_every_entry() {
    let _guard = env_guard();
    env_sandbox("transfer-roundtrip");
    let text = exported_text(&["one", "two"]);

    let summary = import_text(&text, false).unwrap();

    assert_eq!((summary.added, summary.skipped), (2, 0));
    let data = load_data().unwrap();
    let parsed = parse_export(&text).unwrap();
    for (stored, original) in data.entries.iter().zip(&parsed) {
        assert_eq!(stored.uuid, original.uuid);
        assert_eq!(stored.description, original.description);
        assert_eq!(stored.start_time, original.start_time);
        assert_eq!(stored.end_time, original.end_time);
    }
}

#[test]
fn importing_the_same_file_twice_adds_nothing_the_second_time() {
    let _guard = env_guard();
    env_sandbox("transfer-twice");
    let text = exported_text(&["one", "two"]);

    import_text(&text, false).unwrap();
    let second = import_text(&text, false).unwrap();

    assert_eq!((second.added, second.skipped), (0, 2));
    assert_eq!(load_data().unwrap().entries.len(), 2);
}

#[test]
fn an_entry_with_a_known_uuid_is_skipped_and_the_local_copy_stays() {
    let _guard = env_guard();
    env_sandbox("transfer-collision");
    let text = exported_text(&["remote name"]);
    let incoming = parse_export(&text).unwrap();
    with_data(|data| {
        seed(data, "local name", false);
        data.entries[0].uuid = incoming[0].uuid;
        Ok(())
    })
    .unwrap();

    let summary = import_text(&text, false).unwrap();

    assert_eq!((summary.added, summary.skipped), (0, 1));
    assert_eq!(load_data().unwrap().entries[0].description, "local name");
}

#[test]
fn imported_entries_get_fresh_local_ids_without_collisions() {
    let _guard = env_guard();
    env_sandbox("transfer-ids");
    with_data(|data| {
        seed(data, "local a", false);
        seed(data, "local b", false);
        Ok(())
    })
    .unwrap();

    import_text(&exported_text(&["x", "y"]), false).unwrap();

    let data = load_data().unwrap();
    let mut ids: Vec<u64> = data.entries.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4, "ids must be unique");
    assert!(data.entries.iter().all(|e| e.id < data.next_id));
}

#[test]
fn a_bad_file_is_rejected_and_the_store_stays_as_it_was() {
    let _guard = env_guard();
    env_sandbox("transfer-reject");
    with_data(|data| {
        seed(data, "local", false);
        Ok(())
    })
    .unwrap();
    let good = exported_text(&["x"]);
    let bad_files = [
        "not json".to_string(),
        good.replace("tt-export", "other"),
        good.replace("\"version\": 1", "\"version\": 2"),
        good.replace(
            &format!("\"schema_version\": {SCHEMA_VERSION}"),
            &format!("\"schema_version\": {}", SCHEMA_VERSION + 1),
        ),
        good.replace(
            &format!("\"schema_version\": {SCHEMA_VERSION}"),
            "\"schema_version\": 1",
        ),
    ];

    for text in &bad_files {
        assert!(import_text(text, false).is_err(), "{text}");
        assert!(import_text(text, true).is_err(), "{text}");
    }

    let data = load_data().unwrap();
    assert_eq!(data.entries.len(), 1);
    assert_eq!(data.entries[0].description, "local");
}

#[test]
fn a_duplicate_uuid_inside_the_file_is_rejected() {
    let text = exported_text(&["x"]);
    let mut file: serde_json::Value = serde_json::from_str(&text).unwrap();
    let entry = file["entries"][0].clone();
    file["entries"].as_array_mut().unwrap().push(entry);

    assert!(parse_export(&file.to_string()).is_err());
}

#[test]
fn a_running_import_is_refused_while_a_local_entry_runs() {
    let _guard = env_guard();
    env_sandbox("transfer-running");
    let mut source = TimeData {
        schema_version: SCHEMA_VERSION,
        ..TimeData::default()
    };
    seed(&mut source, "remote running", true);
    source.fill_missing_uuids();
    let text = render_export(&source, Local::now()).unwrap();
    with_data(|data| {
        seed(data, "local running", true);
        Ok(())
    })
    .unwrap();

    assert!(import_text(&text, false).is_err());
    assert_eq!(load_data().unwrap().entries.len(), 1);
    // Replace drops the local running entry, so it is allowed.
    assert_eq!(import_text(&text, true).unwrap().added, 1);
}

#[test]
fn replace_swaps_the_entries_and_writes_a_backup_of_the_old_store() {
    let _guard = env_guard();
    env_sandbox("transfer-replace");
    with_data(|data| {
        seed(data, "old", false);
        Ok(())
    })
    .unwrap();
    let before = std::fs::read_to_string(storage::get_data_path().unwrap()).unwrap();

    let summary = import_text(&exported_text(&["new"]), true).unwrap();

    assert_eq!((summary.added, summary.removed), (1, 1));
    let backup = summary.backup.expect("a backup path");
    assert_eq!(std::fs::read_to_string(backup).unwrap(), before);
    let data = load_data().unwrap();
    assert_eq!(data.entries.len(), 1);
    assert_eq!(data.entries[0].description, "new");
}
