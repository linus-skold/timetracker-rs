//! Project grouping in the Entries table's row model.

use super::App;
use super::keys::handle_key;
use super::rows::VisibleRow;
use super::types::ViewMode;
use crate::storage::env_guard;
use crate::storage::env_sandbox as sandbox;
use crate::storage::save_data;
use crate::tracker::{TimeData, TimeEntry};
use chrono::{Duration, Local, NaiveDate};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// A half-hour entry starting at `hour` on `date`.
fn at(
    id: u64,
    description: &str,
    project: Option<&str>,
    tags: &[&str],
    date: NaiveDate,
    hour: u32,
) -> TimeEntry {
    let start = date
        .and_hms_opt(hour, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    TimeEntry {
        id,
        uuid: uuid::Uuid::new_v4(),
        description: description.to_string(),
        project: project.map(str::to_string),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        start_time: start,
        end_time: Some(start + Duration::minutes(30)),
        idle: Vec::new(),
        data: None,
    }
}

fn app_with(name: &str, entries: Vec<TimeEntry>, view: ViewMode) -> App {
    sandbox(name);
    let next_id = entries.len() as u64;
    save_data(&TimeData {
        entries,
        next_id,
        schema_version: 1,
    })
    .unwrap();
    let mut app = App::new().unwrap();
    app.view_mode = view;
    app
}

fn press(app: &mut App, code: KeyCode) {
    handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

fn total_of(app: &App, members: &[usize]) -> String {
    crate::duration::format(
        members
            .iter()
            .filter_map(|i| app.data.entries.get(*i))
            .fold(Duration::zero(), |acc, e| acc + e.duration()),
    )
}

/// One line a row: `day`, `project <name> <total>`, `group <tag> <count>` and
/// `entry <description>`, each behind its tree strokes (`+-` open, `` `- ``
/// last, and `|  ` or blanks for the project stroke an issue member sits in).
fn shape(app: &App) -> Vec<String> {
    let connector = |last: bool| if last { "`- " } else { "+- " };
    app.rows()
        .iter()
        .map(|row| match row {
            VisibleRow::DayHeader { date } => format!("day {date}"),
            VisibleRow::ProjectHeader(h) => {
                format!("project {} {}", h.project, total_of(app, &h.members))
            }
            VisibleRow::GroupHeader(h) => format!(
                "group {}{} {}",
                h.branch.map(connector).unwrap_or_default(),
                h.tag,
                h.members.len()
            ),
            VisibleRow::Entry { index, member } => {
                let stroke = match member.as_ref().and_then(|m| m.outer_last) {
                    None => "",
                    Some(true) => "   ",
                    Some(false) => "|  ",
                };
                let glyph = member.as_ref().map(|m| connector(m.last)).unwrap_or("");
                format!(
                    "entry {stroke}{glyph}{}",
                    app.data.entries[*index].description
                )
            }
        })
        .collect()
}

/// Newest first: beta, two no-project entries, then alpha's loose entry and
/// its two-entry issue.
fn mixed_entries() -> Vec<TimeEntry> {
    vec![
        at(0, "a1", Some("alpha"), &["tt/8"], today(), 9),
        at(1, "a2", Some("alpha"), &["tt/8"], today(), 10),
        at(2, "a3", Some("alpha"), &[], today(), 11),
        at(3, "n1", None, &[], today(), 12),
        at(4, "n2", Some("  "), &[], today(), 13),
        at(5, "b1", Some("beta"), &[], today(), 14),
    ]
}

#[test]
fn grouping_is_off_by_default_and_leaves_the_rows_as_they_were() {
    let _guard = env_guard();
    let app = app_with("proj-off", mixed_entries(), ViewMode::Day);

    assert!(!app.group_by_project);
    assert_eq!(
        shape(&app),
        vec![
            "entry b1",
            "entry n2",
            "entry n1",
            "entry a3",
            "group tt/8 2"
        ]
    );
}

#[test]
fn each_project_gets_a_header_with_its_total_and_no_project_is_one_bucket() {
    let _guard = env_guard();
    let mut app = app_with("proj-on", mixed_entries(), ViewMode::Day);
    app.group_by_project = true;

    assert_eq!(
        shape(&app),
        vec![
            "project beta 0h 30m",
            "entry `- b1",
            "project (no project) 1h 0m",
            "entry +- n2",
            "entry `- n1",
            "project alpha 1h 30m",
            "entry +- a3",
            "group `- tt/8 2",
        ]
    );
}

#[test]
fn an_expanded_issue_nests_inside_its_project() {
    let _guard = env_guard();
    let mut app = app_with("proj-nested", mixed_entries(), ViewMode::Day);
    app.group_by_project = true;
    app.expanded_issues.insert("tt/8".to_string());

    let rows = shape(&app);
    assert_eq!(
        rows[5..],
        [
            "project alpha 1h 30m",
            "entry +- a3",
            "group `- tt/8 2",
            "entry    +- a2",
            "entry    `- a1",
        ]
    );
}

#[test]
fn an_issue_that_is_not_the_last_child_keeps_the_project_stroke_running() {
    let _guard = env_guard();
    // The issue sits above the loose entry, so the loose entry closes the tree.
    let mut app = app_with(
        "proj-stroke",
        vec![
            at(0, "loose", Some("alpha"), &[], today(), 9),
            at(1, "i1", Some("alpha"), &["tt/8"], today(), 10),
            at(2, "i2", Some("alpha"), &["tt/8"], today(), 11),
        ],
        ViewMode::Day,
    );
    app.group_by_project = true;
    app.expanded_issues.insert("tt/8".to_string());

    assert_eq!(
        shape(&app),
        vec![
            "project alpha 1h 30m",
            "group +- tt/8 2",
            "entry |  +- i2",
            "entry |  `- i1",
            "entry `- loose",
        ]
    );
}

#[test]
fn week_view_holds_one_header_per_project_inside_every_day() {
    let _guard = env_guard();
    let monday = TimeData::week_start(today());
    let tuesday = monday + Duration::days(1);
    let mut app = app_with(
        "proj-week",
        vec![
            at(0, "m-alpha", Some("alpha"), &[], monday, 9),
            at(1, "m-beta", Some("beta"), &[], monday, 10),
            at(2, "t-alpha", Some("alpha"), &[], tuesday, 9),
            at(3, "t-alpha-2", Some("alpha"), &[], tuesday, 10),
        ],
        ViewMode::Week,
    );
    app.selected_date = monday;
    app.group_by_project = true;

    assert_eq!(
        shape(&app),
        vec![
            format!("day {tuesday}"),
            "project alpha 1h 0m".to_string(),
            "entry +- t-alpha-2".to_string(),
            "entry `- t-alpha".to_string(),
            format!("day {monday}"),
            "project beta 0h 30m".to_string(),
            "entry `- m-beta".to_string(),
            "project alpha 0h 30m".to_string(),
            "entry `- m-alpha".to_string(),
        ]
    );
}

#[test]
fn toggling_a_project_header_hides_and_shows_its_members() {
    let _guard = env_guard();
    let mut app = app_with("proj-toggle", mixed_entries(), ViewMode::Day);
    app.group_by_project = true;
    // Selectable rows: beta, b1, (no project), n2, n1, alpha, a3, group.
    app.table_state.select(Some(0));

    app.toggle_group_at_cursor();
    assert_eq!(
        shape(&app)[..2],
        ["project beta 0h 30m", "project (no project) 1h 0m"]
    );
    assert_eq!(app.table_state.selected(), Some(0));
    assert_eq!(app.selectable_len(), 7);

    app.toggle_group_at_cursor();
    assert_eq!(shape(&app)[..2], ["project beta 0h 30m", "entry `- b1"]);
    assert_eq!(app.table_state.selected(), Some(0));
}

#[test]
fn toggling_on_a_project_member_collapses_the_project_onto_its_header() {
    let _guard = env_guard();
    let mut app = app_with("proj-toggle-member", mixed_entries(), ViewMode::Day);
    app.group_by_project = true;
    // Index 4 is `n1`, under the no-project header at index 2.
    app.table_state.select(Some(4));

    app.toggle_group_at_cursor();

    assert!(
        app.collapsed_projects
            .contains(crate::tui::summary::NO_PROJECT)
    );
    assert_eq!(app.table_state.selected(), Some(2));
    assert!(matches!(
        app.selected_row(),
        Some(VisibleRow::ProjectHeader(_))
    ));
}

#[test]
fn the_p_key_flips_grouping_and_the_cached_rows_follow() {
    let _guard = env_guard();
    let mut app = app_with("proj-key", mixed_entries(), ViewMode::Day);
    let flat = shape(&app);

    press(&mut app, KeyCode::Char('p'));
    assert!(app.group_by_project);
    assert_ne!(shape(&app), flat);

    app.collapsed_projects.insert("beta".to_string());
    assert!(!shape(&app).contains(&"entry `- b1".to_string()));

    press(&mut app, KeyCode::Char('p'));
    assert_eq!(shape(&app), flat);
}

#[test]
fn the_cursor_stays_on_its_entry_when_grouping_flips() {
    let _guard = env_guard();
    let mut app = app_with("proj-cursor", mixed_entries(), ViewMode::Day);
    // Flat order: b1, n2, n1, a3, group. The cursor is on n1.
    app.table_state.select(Some(2));

    app.toggle_group_by_project();

    assert_eq!(
        app.selected_entry().map(|e| e.description.clone()),
        Some("n1".to_string())
    );
}

#[test]
fn turning_grouping_off_on_a_project_header_lands_on_its_first_member() {
    let _guard = env_guard();
    let mut app = app_with("proj-off-header", mixed_entries(), ViewMode::Day);
    app.group_by_project = true;
    // Index 5 is the alpha header; its newest member is `a3`.
    app.table_state.select(Some(5));

    app.toggle_group_by_project();

    assert_eq!(
        app.selected_entry().map(|e| e.description.clone()),
        Some("a3".to_string())
    );
}

#[test]
fn turning_grouping_on_from_an_issue_header_in_a_collapsed_project_lands_on_the_project() {
    let _guard = env_guard();
    let mut app = app_with("proj-on-header", mixed_entries(), ViewMode::Day);
    app.collapsed_projects.insert("alpha".to_string());
    // Flat order: b1, n2, n1, a3, group.
    app.table_state.select(Some(4));

    app.toggle_group_by_project();

    match app.selected_row() {
        Some(VisibleRow::ProjectHeader(header)) => assert_eq!(header.project, "alpha"),
        _ => panic!("the cursor is not on the alpha header"),
    }
}

#[test]
fn the_same_issue_in_two_projects_keeps_the_cursor_on_its_own_header() {
    let _guard = env_guard();
    let mut app = app_with(
        "proj-same-tag",
        vec![
            at(0, "b1", Some("beta"), &["tt/8"], today(), 9),
            at(1, "b2", Some("beta"), &["tt/8"], today(), 10),
            at(2, "a1", Some("alpha"), &["tt/8"], today(), 12),
            at(3, "a2", Some("alpha"), &["tt/8"], today(), 13),
        ],
        ViewMode::Day,
    );
    app.group_by_project = true;
    // Rows: alpha, its group, beta, its group. The cursor is on beta's group.
    app.table_state.select(Some(3));

    app.toggle_group_at_cursor();

    // alpha, group, a2, a1, beta, group: beta's group is the sixth.
    assert_eq!(app.table_state.selected(), Some(5));
}

#[test]
fn the_reveal_counts_every_row_under_a_project_including_expanded_issues() {
    let _guard = env_guard();
    let mut app = app_with("proj-reveal", mixed_entries(), ViewMode::Day);
    app.group_by_project = true;
    app.expanded_issues.insert("tt/8".to_string());

    // alpha holds a3, the issue header and its two members; three entries.
    let rows = app.rows();
    assert_eq!(
        super::project_rows::rows_under_project_header(&rows, 5),
        Some(4)
    );
    assert_eq!(
        super::project_rows::rows_under_project_header(&rows, 1),
        None
    );
}
