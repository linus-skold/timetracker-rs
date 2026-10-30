//! The Entries table scrolls only when the cursor leaves the page.

use super::App;
use super::keys::handle_key;
use super::rows::VisibleRow;
use super::tests::frame_lines;
use super::types::ViewMode;
use crate::storage::env_guard;
use crate::storage::env_sandbox as sandbox;
use crate::tracker::{TimeData, TimeEntry};
use chrono::{Datelike, Duration, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
/// The group is the oldest, so it sorts to the bottom.
fn app_with(name: &str, loose: u64, members: u64) -> App {
    seeded_app(name, ViewMode::Day, loose, members, 0)
}

/// Newest first: `newer` loose entries, then the group of `members`, then
/// `older` loose entries.
fn seeded_app(name: &str, view: ViewMode, newer: u64, members: u64, older: u64) -> App {
    sandbox(name);
    let loose = newer + older;
    let mut entries: Vec<TimeEntry> = (0..newer)
        .map(|i| {
            let minutes = (older + members + i) as i64 * 10;
            entry_at(i, format!("row-{i:02}"), None, minutes)
        })
        .collect();
    entries.extend(
        (0..older).map(|i| entry_at(newer + i, format!("row-old-{i:02}"), None, i as i64 * 10)),
    );
    entries.extend((0..members).map(|i| {
        entry_at(
            loose + i,
            format!("mem-{i:02}"),
            Some("tt/8"),
            (older + i) as i64 * 10,
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
    app.view_mode = view;
    app
}

fn press_key(app: &mut App, code: KeyCode) {
    handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

/// The index of the drawn line holding `needle`, among the body lines.
fn line_of(lines: &[String], needle: &str) -> usize {
    lines.iter().position(|l| l.contains(needle)).expect(needle)
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

/// Walk to the group as a user does, one drawn frame after each key.
fn walk_to_group_drawing(app: &mut App) {
    body(app);
    for _ in 0..app.selectable_len() {
        if matches!(app.selected_row(), Some(VisibleRow::GroupHeader(_))) {
            return;
        }
        press_key(app, KeyCode::Char('j'));
        body(app);
    }
    panic!("no group header reached");
}

/// Open the group with `key` and return how far its row moved up.
fn rows_the_group_moved_up(app: &mut App, key: KeyCode) -> usize {
    walk_to_group_drawing(app);
    open_drawn_group_and_measure(app, key)
}

/// As [`rows_the_group_moved_up`], with the cursor already on a drawn group.
fn open_drawn_group_and_measure(app: &mut App, key: KeyCode) -> usize {
    let before = line_of(&body(app), "tt/8");
    press_key(app, key);
    let after = body(app);
    assert_eq!(
        line_of(&after, "mem-00"),
        after.len() - 1,
        "the last child is not on the bottom line"
    );
    before - line_of(&after, "tt/8")
}

#[test]
fn week_view_opening_the_last_group_moves_it_up_by_its_children() {
    let _guard = env_guard();
    let mut app = seeded_app("scroll-week-a", ViewMode::Week, 30, 3, 0);
    for key in [KeyCode::Enter, KeyCode::Char('g')] {
        app.expanded_issues.clear();
        assert_eq!(rows_the_group_moved_up(&mut app, key), 3);
    }
}

#[test]
fn week_view_opening_the_second_last_group_moves_it_up_by_two() {
    let _guard = env_guard();
    let mut app = seeded_app("scroll-week-b", ViewMode::Week, 30, 3, 1);
    // Go to the last row, then up once: the group is second from the bottom.
    select_last(&mut app);
    body(&mut app);
    press_key(&mut app, KeyCode::Char('k'));
    assert!(matches!(
        app.selected_row(),
        Some(VisibleRow::GroupHeader(_))
    ));
    assert_eq!(open_drawn_group_and_measure(&mut app, KeyCode::Enter), 2);
}

#[test]
fn week_view_a_group_that_already_fits_does_not_scroll() {
    let _guard = env_guard();
    let mut app = seeded_app("scroll-week-fits", ViewMode::Week, 0, 2, 0);
    walk_to_group_drawing(&mut app);
    assert!(day_header_is_drawn(&mut app), "no day header to start with");
    press_key(&mut app, KeyCode::Enter);
    assert!(day_header_is_drawn(&mut app), "the view scrolled");
}

/// Whether the two-line day header of today is on screen.
fn day_header_is_drawn(app: &mut App) -> bool {
    let day = Local::now().format("%A").to_string();
    frame_lines(app, WIDTH, HEIGHT)
        .iter()
        .any(|line| line.contains(&day))
}

#[test]
fn week_view_counts_a_two_line_day_header_when_a_group_opens() {
    let _guard = env_guard();
    // Day header (2 lines), group, 3 members: 6 lines in a 5-line page.
    let mut app = seeded_app("scroll-week-header-height", ViewMode::Week, 0, 3, 0);
    walk_to_group_drawing(&mut app);
    press_key(&mut app, KeyCode::Enter);
    let after = body(&mut app);
    assert_eq!(line_of(&after, "mem-00"), after.len() - 1);
    assert!(after[0].contains("tt/8"), "the group row left the view");
}

/// Two days with a group on each, both on `tt/8`; the other day is a
/// neighbour inside the same week.
fn two_day_group_app(name: &str) -> App {
    let app = seeded_app(name, ViewMode::Week, 0, 2, 0);
    let mut data = app.data.clone();
    let delta = if Local::now().weekday() == chrono::Weekday::Mon {
        1
    } else {
        -1
    };
    let other_day: Vec<TimeEntry> = data
        .entries
        .iter()
        .map(|e| {
            let mut copy = e.clone();
            copy.id += 100;
            copy.description = format!("old-{}", e.description);
            copy.start_time += Duration::days(delta);
            copy.end_time = copy.end_time.map(|t| t + Duration::days(delta));
            copy
        })
        .collect();
    data.entries.extend(other_day);
    data.next_id = 200;
    crate::storage::save_data(&data).unwrap();
    let mut app = App::new().unwrap();
    app.view_mode = ViewMode::Week;
    app
}

#[test]
fn week_view_opening_the_later_day_group_keeps_the_cursor_on_its_header() {
    let _guard = env_guard();
    let mut app = two_day_group_app("scroll-week-two-days");
    let last_header = app
        .rows()
        .iter()
        .filter(|row| !matches!(row, VisibleRow::DayHeader { .. }))
        .enumerate()
        .filter(|(_, row)| matches!(row, VisibleRow::GroupHeader(_)))
        .map(|(idx, _)| idx)
        .last()
        .expect("two group headers");
    app.table_state.select(Some(last_header));
    let date_of = |app: &App| match app.selected_row() {
        Some(VisibleRow::GroupHeader(header)) => header.start.date_naive(),
        _ => panic!("the cursor left a group header"),
    };
    let before = date_of(&app);

    press_key(&mut app, KeyCode::Enter);

    assert_eq!(date_of(&app), before);
}

#[test]
fn week_view_a_header_near_the_bottom_keeps_the_view_when_moving_up() {
    let _guard = env_guard();
    let mut app = two_day_group_app("scroll-week-sticky-header");
    app.expanded_issues.insert("tt/8".to_string());
    select_last(&mut app);
    let before = body(&mut app);
    press_key(&mut app, KeyCode::Char('k'));
    let after = body(&mut app);
    assert_eq!(before.first(), after.first(), "the view scrolled");
}

#[test]
fn down_on_the_last_row_and_up_on_the_first_row_stay_put() {
    let _guard = env_guard();
    let mut app = app_with("scroll-no-wrap", 5, 0);
    press_key(&mut app, KeyCode::Char('k'));
    assert_eq!(app.table_state.selected(), Some(0));
    select_last(&mut app);
    let last = app.table_state.selected();
    press_key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.table_state.selected(), last);
}
