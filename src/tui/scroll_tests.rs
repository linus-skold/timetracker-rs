//! The Entries table scrolls only when the cursor leaves the page.

use super::App;
use super::rows::VisibleRow;
use super::tests::frame_lines;
use super::types::ViewMode;
use crate::storage::env_guard;
use crate::storage::env_sandbox as sandbox;
use crate::tracker::{TimeData, TimeEntry};
use chrono::{Duration, Local};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 14;

/// An entry `minutes` after midnight today; `tag` joins it to a group.
fn entry_at(id: u64, description: String, tag: Option<&str>, minutes: i64) -> TimeEntry {
    let midnight = Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    let start = midnight + Duration::minutes(minutes);
    TimeEntry {
        id,
        description,
        project: Some("tt".to_string()),
        tags: tag.iter().map(|t| t.to_string()).collect(),
        start_time: start,
        end_time: Some(start + Duration::minutes(5)),
        idle: Vec::new(),
        data: None,
    }
}

/// A Day view app with `loose` loose entries and `members` entries on `tt/8`.
fn app_with(name: &str, loose: u64, members: u64) -> App {
    sandbox(name);
    let mut entries: Vec<TimeEntry> = (0..loose)
        .map(|i| entry_at(i, format!("row-{i:02}"), None, (members + i) as i64 * 10))
        .collect();
    // The group is the oldest, so it sorts to the bottom.
    entries.extend((0..members).map(|i| {
        entry_at(
            loose + i,
            format!("mem-{i:02}"),
            Some("tt/8"),
            i as i64 * 10,
        )
    }));
    let next_id = entries.len() as u64;
    crate::storage::save_data(&TimeData {
        entries,
        next_id,
        schema_version: 1,
    })
    .unwrap();
    let mut app = App::new().unwrap();
    app.view_mode = ViewMode::Day;
    app
}

fn is_body_line(line: &str) -> bool {
    line.contains("row-") || line.contains("mem-") || line.contains("tt/8")
}

/// The drawn table body lines, top to bottom.
fn body(app: &mut App) -> Vec<String> {
    frame_lines(app, WIDTH, HEIGHT)
        .into_iter()
        .filter(|line| is_body_line(line))
        .map(|line| line.replacen(">>", "  ", 1))
        .collect()
}

fn select_last(app: &mut App) {
    let last = app.selectable_len() - 1;
    app.table_state.select(Some(last));
}

fn group_header_row(app: &App) -> usize {
    app.rows()
        .iter()
        .position(|row| matches!(row, VisibleRow::GroupHeader(_)))
        .expect("a group header")
}

/// The selectable index of the group header (Day view has no day headers).
fn select_group(app: &mut App) -> usize {
    let row = group_header_row(app);
    app.table_state.select(Some(row));
    row
}

#[test]
fn moving_up_from_the_last_row_keeps_the_view_still() {
    let _guard = env_guard();
    let mut app = app_with("scroll-up-still", 40, 0);
    select_last(&mut app);
    let before = body(&mut app);

    app.previous();
    let after = body(&mut app);

    assert_eq!(before.first(), after.first(), "the view scrolled");
}

#[test]
fn moving_up_from_the_top_visible_row_scrolls_by_one() {
    let _guard = env_guard();
    let mut app = app_with("scroll-up-top", 40, 0);
    select_last(&mut app);
    let before = body(&mut app);
    for _ in 0..before.len() - 1 {
        app.previous();
    }
    assert_eq!(body(&mut app).first(), before.first(), "scrolled too soon");

    app.previous();
    let after = body(&mut app);

    assert_ne!(before.first(), after.first(), "the view did not scroll");
    assert_eq!(
        after.get(1),
        before.first(),
        "the view scrolled by more than one"
    );
}

#[test]
fn opening_a_group_near_the_bottom_scrolls_its_members_into_view() {
    let _guard = env_guard();
    let mut app = app_with("scroll-open-bottom", 30, 3);
    let group = select_group(&mut app);
    let page = body(&mut app).len();
    // Put the group row on the last line of the page.
    app.entry_scroll_offset = (group + 1).saturating_sub(page);
    assert!(body(&mut app).last().unwrap().contains("tt/8"));

    app.toggle_group_at_cursor();
    let after = body(&mut app);

    assert!(after.iter().any(|l| l.contains("tt/8")), "group row left");
    assert_eq!(after.iter().filter(|l| l.contains("mem-")).count(), 3);
}

#[test]
fn a_group_longer_than_the_page_keeps_its_row_at_the_top() {
    let _guard = env_guard();
    let mut app = app_with("scroll-open-long", 30, 30);
    let group = select_group(&mut app);
    let page = body(&mut app).len();
    app.entry_scroll_offset = (group + 1).saturating_sub(page);
    body(&mut app);

    app.toggle_group_at_cursor();
    let after = body(&mut app);

    assert!(after[0].contains("tt/8"), "group row is not the top row");
}
