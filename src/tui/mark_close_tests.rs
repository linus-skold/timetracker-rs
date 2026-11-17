//! Closing or dropping an open agent mark from the TUI.

use super::App;
use super::keys::handle_key;
use super::tests::{begin_mark, mark_sandbox};
use super::types::InputMode;
use crate::storage::env_guard;
use crate::storage::env_sandbox as sandbox;
use crate::tracker::TimeData;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

const MARK_KEY: &str = "proj.12.impl";

fn press(app: &mut App, code: KeyCode) {
    handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

/// An app with one open mark, in a fresh sandbox with an empty store.
fn app_with_mark(name: &str) -> (App, PathBuf) {
    sandbox(name);
    crate::storage::save_data(&TimeData {
        entries: Vec::new(),
        next_id: 0,
        schema_version: 1,
    })
    .unwrap();
    let dir = mark_sandbox();
    begin_mark(&dir, MARK_KEY, 30);
    (App::new().unwrap(), dir)
}

/// `x`, `Enter`, `e`: pick the first mark and open the close form.
fn open_close_form(app: &mut App) {
    press(app, KeyCode::Char('x'));
    press(app, KeyCode::Enter);
    press(app, KeyCode::Char('e'));
}

fn logged_entries() -> Vec<crate::tracker::TimeEntry> {
    crate::storage::load_data().unwrap().entries
}

/// Replace the mark with one begun `minutes_ago`: same key, another start.
fn reopen_mark(dir: &std::path::Path, minutes_ago: i64) {
    std::fs::remove_file(dir.join(MARK_KEY)).unwrap();
    begin_mark(dir, MARK_KEY, minutes_ago);
}

#[test]
fn the_picker_reaches_every_open_mark() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-many");
    for key in ["proj.13.qa", "proj.14.docs", "proj.15.plan"] {
        begin_mark(&dir, key, 5);
    }
    app.marks_stamp = None;
    app.sync_from_marks();
    assert_eq!(app.marks.len(), 4);

    press(&mut app, KeyCode::Char('x'));
    for _ in 0..3 {
        press(&mut app, KeyCode::Char('j'));
    }
    press(&mut app, KeyCode::Enter);

    assert_eq!(app.picked_mark.as_ref(), Some(&app.marks[3]));
}

#[test]
fn discard_leaves_a_reopened_mark_alone() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-discard-reopened");

    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Enter);
    reopen_mark(&dir, 5);
    press(&mut app, KeyCode::Char('d'));

    assert!(dir.join(MARK_KEY).exists());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|message| message.contains("closed already"))
    );
}

#[test]
fn save_logs_nothing_for_a_reopened_mark() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-save-reopened");

    open_close_form(&mut app);
    type_str(&mut app, "wrote it");
    reopen_mark(&dir, 5);
    press(&mut app, KeyCode::Enter);

    assert!(logged_entries().is_empty());
    assert!(dir.join(MARK_KEY).exists());
}

#[test]
fn an_unfinished_close_blocks_the_save() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-sentinel");

    open_close_form(&mut app);
    type_str(&mut app, "wrote it");
    let mark = app.closing_mark.clone().unwrap();
    crate::marks::start_closing_in(&dir, mark.as_key()).unwrap();
    press(&mut app, KeyCode::Enter);

    assert!(logged_entries().is_empty());
    assert!(dir.join(MARK_KEY).exists());
}

#[test]
fn the_prefilled_end_is_the_last_beat() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-beat");
    let beat = chrono::Local::now() - chrono::Duration::minutes(10);
    std::fs::create_dir_all(dir.join("beats")).unwrap();
    std::fs::write(
        dir.join("beats").join(MARK_KEY),
        format!("{}\n", beat.timestamp()),
    )
    .unwrap();

    open_close_form(&mut app);

    assert_eq!(
        app.input_end_time.value(),
        beat.format("%Y-%m-%d %H:%M").to_string()
    );
}

#[test]
fn the_prefilled_duration_spans_the_minutes_the_form_shows() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-minutes");
    // Start 10:00:50 and end 10:30:10 show as 10:00 and 10:30: 30m, not 29m.
    let top_of_hour = chrono::Local::now().timestamp() / 3600 * 3600 - 7200;
    std::fs::write(dir.join(MARK_KEY), format!("{}\n", top_of_hour + 50)).unwrap();
    std::fs::create_dir_all(dir.join("beats")).unwrap();
    std::fs::write(
        dir.join("beats").join(MARK_KEY),
        format!("{}\n", top_of_hour + 30 * 60 + 10),
    )
    .unwrap();
    app.marks_stamp = None;
    app.sync_from_marks();

    open_close_form(&mut app);

    assert_eq!(
        app.input_duration.value(),
        crate::duration::format(chrono::Duration::minutes(30))
    );
}

#[test]
fn x_without_marks_opens_nothing_and_says_so() {
    let _guard = env_guard();
    sandbox("mark-close-none");
    let mut app = App::new().unwrap();

    press(&mut app, KeyCode::Char('x'));

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.status_message.as_deref(), Some("No open marks."));
}

#[test]
fn discard_drops_the_mark_and_logs_nothing() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-discard");

    press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.input_mode, InputMode::MarkPicker);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.input_mode, InputMode::MarkAction);
    press(&mut app, KeyCode::Char('d'));

    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(!dir.join(MARK_KEY).exists());
    assert!(app.marks.is_empty(), "the list refreshed at once");
    assert!(logged_entries().is_empty());
}

#[test]
fn edit_and_save_logs_one_entry_and_drops_the_mark() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-save");

    open_close_form(&mut app);
    assert_eq!(app.input_mode, InputMode::AddingEntry);
    assert_eq!(app.input_project.value(), "proj");
    assert_eq!(app.input_tags.value(), "proj/12 impl agent");
    type_str(&mut app, "wrote it");
    press(&mut app, KeyCode::Enter);

    let entries = logged_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].description, "wrote it");
    assert_eq!(entries[0].project.as_deref(), Some("proj"));
    assert!(entries[0].tags.contains(&"impl".to_string()));
    assert!(entries[0].end_time.is_some());
    assert!(!dir.join(MARK_KEY).exists());
    assert!(app.marks.is_empty());
    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn escape_in_the_close_form_keeps_the_mark_open() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-cancel");

    open_close_form(&mut app);
    type_str(&mut app, "wrote it");
    press(&mut app, KeyCode::Esc);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.closing_mark.is_none());
    assert!(dir.join(MARK_KEY).exists());
    assert!(logged_entries().is_empty());
}

#[test]
fn a_mark_removed_before_save_logs_nothing() {
    let _guard = env_guard();
    let (mut app, dir) = app_with_mark("mark-close-vanished");

    open_close_form(&mut app);
    type_str(&mut app, "wrote it");
    // An agent ended the mark while the form was open.
    std::fs::remove_file(dir.join(MARK_KEY)).unwrap();
    press(&mut app, KeyCode::Enter);

    assert!(logged_entries().is_empty());
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|message| message.contains("closed already"))
    );
}
