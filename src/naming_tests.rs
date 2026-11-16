use super::*;
use crate::storage::{env_guard, env_sandbox, load_data, save_data};
use crate::tracker::TimeData;
use chrono::Local;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

#[test]
fn a_name_is_trimmed_and_lowercased() {
    assert_eq!(normalize_name("  pRojeCt1 "), "project1");
    assert_eq!(normalize_name("Project1"), "project1");
    assert_eq!(normalize_name("project1"), "project1");
}

#[test]
fn a_missing_project_stays_missing() {
    assert_eq!(normalize_project(None), None);
    assert_eq!(
        normalize_project(Some("Acme".into())),
        Some("acme".to_string())
    );
}

#[test]
fn tags_that_match_after_lowercasing_collapse_in_order() {
    let tags = normalize_tags(strings(&["Bug", "api", "BUG", "API", "Web"]));
    assert_eq!(tags, strings(&["bug", "api", "web"]));
}

#[test]
fn add_and_update_write_lowercase() {
    let mut data = TimeData::default();
    let now = Local::now();
    let id = data
        .add_entry(
            "work".into(),
            Some("Project1".into()),
            strings(&["Project1", "project1"]),
            now,
            None,
        )
        .id;
    let entry = data.get_entry(id).unwrap();
    assert_eq!(entry.project.as_deref(), Some("project1"));
    assert_eq!(entry.tags, strings(&["project1"]));

    data.update_entry(
        id,
        "work".into(),
        Some("pRojeCt2".into()),
        strings(&["Web"]),
        now,
        None,
    );
    let entry = data.get_entry(id).unwrap();
    assert_eq!(entry.project.as_deref(), Some("project2"));
    assert_eq!(entry.tags, strings(&["web"]));
}

#[test]
fn loading_lowercases_old_data_and_keeps_the_rest() {
    let _guard = env_guard();
    env_sandbox("naming-load");
    let mut data = TimeData::default();
    let now = Local::now();
    data.add_entry("Keep THIS Case".into(), None, Vec::new(), now, None);
    {
        // Written past add_entry to stand in for an old store.
        let entry = &mut data.entries[0];
        entry.project = Some("Project1".into());
        entry.tags = strings(&["Project1/ABC-1", "project1/abc-1", "Bug"]);
    }
    save_data(&data).unwrap();

    let loaded = load_data().unwrap();
    let entry = &loaded.entries[0];
    assert_eq!(entry.project.as_deref(), Some("project1"));
    assert_eq!(entry.tags, strings(&["project1/abc-1", "bug"]));
    assert_eq!(entry.description, "Keep THIS Case");
}
