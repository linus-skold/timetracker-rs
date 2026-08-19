use anyhow::Result;
use chrono::{Local, NaiveDate};
use std::io::{self, Stdout};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::TableState};
use crate::marks::Mark;
use crate::storage::{PathStamp, load_data};
use crate::tracker::TimeData;

pub mod theme;
pub mod types;
mod search;
mod navigation;
mod entry_form;
mod marks_surface;
mod panes;
mod summary;
mod render;

pub use types::{
    ConfirmAction, Focus, InputField, InputMode, Pane, PendingConfirm, SortOrder, ViewMode,
};

pub(crate) struct App {
    pub(crate) data: TimeData,
    pub(crate) table_state: TableState,
    pub(crate) should_quit: bool,
    pub(crate) view_mode: ViewMode,
    pub(crate) selected_date: NaiveDate,
    pub(crate) input_mode: InputMode,
    pub(crate) input_field: InputField,
    pub(crate) input_description: String,
    pub(crate) input_project: String,
    pub(crate) input_tags: String,
    pub(crate) input_start_time: String,
    pub(crate) input_end_time: String,
    pub(crate) input_duration: String,
    pub(crate) search_term: String,
    /// The values picked in each pane. OR within a set, AND across the two.
    pub(crate) selected_projects: Vec<String>,
    pub(crate) selected_tags: Vec<String>,
    pub(crate) editing_entry_id: Option<u64>,
    /// What `InputMode::Confirm` is asking about: the action, entry and origin mode.
    pub(crate) pending_confirm: Option<PendingConfirm>,
    pub(crate) sort_order: SortOrder,
    /// Cursor position in the active input field — a char index, not a byte index.
    pub(crate) cursor_pos: usize,
    /// Fingerprint of the store as of the last load, so a tick can skip the read.
    pub(crate) store_stamp: Option<PathStamp>,
    /// The open agent phase marks, newest first, so a frame never reads the directory.
    pub(crate) marks: Vec<Mark>,
    /// Fingerprint of the *mark directory*, so a tick need not list it.
    pub(crate) marks_stamp: Option<PathStamp>,
    /// Whether each collapsible surface is open. All default to off, so their rows
    /// are absent from the layout plan.
    pub(crate) show_projects: bool,
    pub(crate) show_tags: bool,
    pub(crate) show_marks: bool,
    pub(crate) show_summary: bool,
    /// What `Tab` has given focus to, and where each pane's cursor rests.
    pub(crate) focus: Focus,
    pub(crate) project_cursor: usize,
    pub(crate) tag_cursor: usize,
}

impl App {
    fn new() -> Result<Self> {
        // Stamp before loading — see `App::reload`.
        let store_stamp = crate::storage::store_stamp();
        let mut app = Self {
            data: load_data()?,
            store_stamp,
            marks: Vec::new(),
            marks_stamp: None,
            table_state: TableState::default().with_selected(Some(0)),
            should_quit: false,
            view_mode: ViewMode::Day,
            selected_date: Local::now().date_naive(),
            input_mode: InputMode::Normal,
            input_field: InputField::Description,
            input_description: String::new(),
            input_project: String::new(),
            input_tags: String::new(),
            input_start_time: String::new(),
            input_end_time: String::new(),
            input_duration: String::new(),
            search_term: String::new(),
            selected_projects: Vec::new(),
            selected_tags: Vec::new(),
            editing_entry_id: None,
            pending_confirm: None,
            sort_order: SortOrder::NewestFirst,
            cursor_pos: 0,
            show_projects: false,
            show_tags: false,
            show_marks: false,
            show_summary: false,
            focus: Focus::Table,
            project_cursor: 0,
            tag_cursor: 0,
        };
        // The first tick is 250 ms away, so read now for a current first frame.
        app.sync_from_marks();
        Ok(app)
    }

    /// Apply `edit` under the store's exclusive lock, then refresh from what landed.
    /// **Every** TUI mutation goes through here: `App.data` is a startup snapshot, so
    /// saving it back would drop outside writes and reuse a stale `next_id`.
    pub(crate) fn mutate_store<T>(&mut self, edit: impl FnOnce(&mut TimeData) -> T) -> Result<T> {
        let (result, fresh) = crate::storage::with_data(|data| {
            let result = edit(data);
            Ok((result, data.clone()))
        })?;
        self.data = fresh;
        // Our own write moved the file on; stamp it so the next tick skips it.
        self.store_stamp = crate::storage::store_stamp();
        Ok(result)
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

pub fn run_tui() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut app = App::new()?;

    loop {
        terminal.draw(|f| render::ui(f, &mut app))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.input_mode {
                        InputMode::Normal => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                if app.is_searching() {
                                    app.clear_search();
                                } else if app.is_filtering() {
                                    app.clear_filters();
                                } else {
                                    app.should_quit = true;
                                }
                            }
                            // j/k move in the focused pane, else the table.
                            KeyCode::Char('j') | KeyCode::Down => {
                                if !app.pane_next() {
                                    app.next();
                                }
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                if !app.pane_previous() {
                                    app.previous();
                                }
                            }
                            // One key, disambiguated by focus: `toggle_pane_value`
                            // reporting false *is* the focus check.
                            KeyCode::Enter => {
                                if !app.toggle_pane_value() {
                                    app.open_detail();
                                }
                            }
                            KeyCode::Char('P') => app.toggle_pane(Pane::Projects),
                            KeyCode::Char('T') => app.toggle_pane(Pane::Tags),
                            KeyCode::Char('A') => app.toggle_marks(),
                            // Capital `S` only; lowercase `s` stops the entry.
                            KeyCode::Char('S') => app.toggle_summary(),
                            KeyCode::Tab => app.cycle_focus(),
                            // crossterm reports Shift-Tab as its own code.
                            KeyCode::BackTab => app.cycle_focus_back(),
                            KeyCode::Char('d') => app.request_confirm(ConfirmAction::Delete),
                            KeyCode::Char('s') => app.stop_active()?,
                            KeyCode::Char('r') => app.reload()?,
                            KeyCode::Char('a') => app.start_adding(),
                            KeyCode::Char('e') => app.start_editing(),
                            KeyCode::Char('/') => app.start_search(),
                            KeyCode::Char('1') => app.set_view_mode(ViewMode::Day),
                            KeyCode::Char('2') => app.set_view_mode(ViewMode::Week),
                            KeyCode::Char('3') => app.set_view_mode(ViewMode::All),
                            KeyCode::Char('h') | KeyCode::Left => app.previous_period(),
                            KeyCode::Char('l') | KeyCode::Right => app.next_period(),
                            KeyCode::Char('t') => app.go_to_today(),
                            KeyCode::Char('o') => app.toggle_sort_order(),
                            KeyCode::Char('?') => app.input_mode = InputMode::Help,
                            _ => {}
                        },
                        InputMode::AddingEntry => match key.code {
                            KeyCode::Esc => app.cancel_adding(),
                            KeyCode::Enter => app.submit_entry()?,
                            KeyCode::Tab => app.next_input_field(),
                            KeyCode::BackTab => app.prev_input_field(),
                            KeyCode::Backspace => app.handle_input_backspace(),
                            KeyCode::Left => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_left();
                                } else {
                                    app.move_cursor_left();
                                }
                            }
                            KeyCode::Right => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_right();
                                } else {
                                    app.move_cursor_right();
                                }
                            }
                            KeyCode::Char(c) => app.handle_input_char(c),
                            _ => {}
                        },
                        InputMode::EditingEntry => match key.code {
                            KeyCode::Esc => app.cancel_adding(),
                            KeyCode::Enter => app.submit_edit()?,
                            KeyCode::Tab => app.next_input_field(),
                            KeyCode::BackTab => app.prev_input_field(),
                            KeyCode::Backspace => app.handle_input_backspace(),
                            KeyCode::Left => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_left();
                                } else {
                                    app.move_cursor_left();
                                }
                            }
                            KeyCode::Right => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_right();
                                } else {
                                    app.move_cursor_right();
                                }
                            }
                            KeyCode::Char(c) => app.handle_input_char(c),
                            _ => {}
                        },
                        InputMode::Searching => match key.code {
                            KeyCode::Esc => app.clear_search(),
                            KeyCode::Enter => app.confirm_search(),
                            KeyCode::Backspace => app.handle_search_backspace(),
                            KeyCode::Left => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_left();
                                } else {
                                    app.move_cursor_left();
                                }
                            }
                            KeyCode::Right => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_right();
                                } else {
                                    app.move_cursor_right();
                                }
                            }
                            KeyCode::Char(c) => app.handle_search_char(c),
                            _ => {}
                        },
                        InputMode::Help => match key.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                                app.input_mode = InputMode::Normal;
                            }
                            _ => {}
                        },
                        // Modal, but not inert: the popover renders whatever
                        // `selected_entry()` returns, so j/k alone make it follow.
                        InputMode::Detail => match key.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                                app.input_mode = InputMode::Normal;
                            }
                            // The table's own, so a focused pane cannot capture j/k.
                            KeyCode::Char('j') | KeyCode::Down => app.next(),
                            KeyCode::Char('k') | KeyCode::Up => app.previous(),
                            // The form is modal too, so it replaces the popover.
                            KeyCode::Char('e') => app.start_editing(),
                            // Confirmed here as on the table: one key, one meaning.
                            KeyCode::Char('d') => app.request_confirm(ConfirmAction::Delete),
                            // `t` rather than `s`, so a slip outside this modal hits
                            // `go_to_today()`. Bound here only.
                            KeyCode::Char('t') => app.request_confirm(ConfirmAction::Trim),
                            _ => {}
                        },
                        // Which key is a yes depends on the pending action, so the
                        // whole answer lives in `answer_confirm`.
                        InputMode::Confirm => app.answer_confirm(key.code)?,
                    }
                }
            }
        }

        // The poll above is the loop's clock, key or timeout alike.
        app.sync_from_store()?;
        app.sync_from_marks();

        if app.should_quit {
            break;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;
    /// Serialises the tests that repoint `HOME` and `TT_MARK_DIR`; env is
    /// process-wide, and `marks`' own env test shares this lock.
    use crate::storage::env_guard;
    use crate::tracker::TimeEntry;
    use std::path::PathBuf;

    /// Point `HOME` and `TT_MARK_DIR` at a fresh scratch dir. **Tests must never
    /// touch the real store or marks** — live agent sessions write both.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-store-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("HOME", &dir) };
        unsafe { std::env::set_var("TT_MARK_DIR", dir.join("marks")) };
        let path = storage::get_data_path().unwrap();
        assert!(
            path.starts_with(&dir),
            "sandbox HOME not in effect: {path:?}"
        );
        let marks = crate::marks::mark_dir().expect("a mark dir");
        assert!(
            marks.starts_with(&dir),
            "sandbox TT_MARK_DIR not in effect: {marks:?}"
        );
        dir
    }

    /// The sandbox's mark directory, created on demand.
    fn mark_sandbox() -> PathBuf {
        let dir = crate::marks::mark_dir().expect("a mark dir");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a mark file the way `tt agent begin` does: name is the phase key,
    /// content a unix-seconds start. Never shells out.
    fn begin_mark(dir: &std::path::Path, key: &str, minutes_ago: i64) {
        let start = Local::now() - chrono::Duration::minutes(minutes_ago);
        std::fs::write(dir.join(key), format!("{}\n", start.timestamp())).unwrap();
    }

    fn entry(id: u64, description: &str) -> TimeEntry {
        TimeEntry {
            id,
            description: description.to_string(),
            project: None,
            tags: Vec::new(),
            start_time: Local::now(),
            end_time: None,
            idle: Vec::new(),
        }
    }

    fn seed(entries: Vec<TimeEntry>, next_id: u64) {
        storage::save_data(&TimeData {
            entries,
            next_id,
            schema_version: 1,
        })
        .unwrap();
    }

    /// A write from outside the TUI, through the same `with_data` path `tt log` uses.
    fn agent_write(description: &str) -> u64 {
        storage::with_data(|data| {
            Ok(data
                .add_entry(
                    description.to_string(),
                    Some("probe".to_string()),
                    vec!["probe".to_string()],
                    Local::now(),
                    Some(Local::now()),
                )
                .id)
        })
        .unwrap()
    }

    fn on_disk() -> TimeData {
        storage::load_data().unwrap()
    }

    fn descriptions(data: &TimeData) -> Vec<&str> {
        data.entries
            .iter()
            .map(|e| e.description.as_str())
            .collect()
    }

    /// `d` pressed and answered, via the two real calls the event loop makes. The
    /// current mode is the mode the prompt is raised from.
    fn press_d_then(app: &mut App, answer: KeyCode) {
        app.request_confirm(ConfirmAction::Delete);
        app.answer_confirm(answer).unwrap();
    }

    /// `t` pressed and answered, the same way.
    fn press_t_then(app: &mut App, answer: KeyCode) {
        app.request_confirm(ConfirmAction::Trim);
        app.answer_confirm(answer).unwrap();
    }

    fn select(app: &mut App, description: &str) {
        let idx = app
            .filtered_entries()
            .iter()
            .position(|e| e.description == description)
            .expect("entry not in view");
        app.table_state.select(Some(idx));
    }

    #[test]
    fn delete_keeps_a_concurrent_agent_write() {
        let _guard = env_guard();
        sandbox("delete");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        agent_write("probe");
        select(&mut app, "doomed");
        press_d_then(&mut app, KeyCode::Char('d'));

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["keep", "probe"]);
        assert_eq!(descriptions(&app.data), vec!["keep", "probe"]);
    }

    #[test]
    fn deleting_an_already_removed_id_is_a_no_op() {
        let _guard = env_guard();
        sandbox("delete-gone");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "doomed");
        // Someone else removed it first, then wrote an entry of their own
        storage::with_data(|data| {
            data.entries.retain(|e| e.id != 1);
            Ok(())
        })
        .unwrap();
        agent_write("probe");

        press_d_then(&mut app, KeyCode::Char('d'));

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["keep", "probe"]);
    }

    #[test]
    fn stop_active_keeps_a_concurrent_agent_write() {
        let _guard = env_guard();
        sandbox("stop");
        seed(vec![entry(0, "running")], 1);

        let mut app = App::new().unwrap();
        agent_write("probe");
        app.stop_active().unwrap();

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["running", "probe"]);
        assert!(
            data.entries[0].end_time.is_some(),
            "the active entry should have been stopped"
        );
    }

    #[test]
    fn add_keeps_a_concurrent_agent_write_and_takes_a_fresh_id() {
        let _guard = env_guard();
        sandbox("add");
        seed(vec![entry(0, "existing")], 1);

        let mut app = App::new().unwrap();
        // The agent claims id 1, which the TUI's snapshot still thinks is free
        let agent_id = agent_write("probe");
        assert_eq!(agent_id, 1);

        app.start_adding();
        app.input_description = "from the tui".to_string();
        app.input_duration = "15m".to_string();
        app.submit_entry().unwrap();

        let data = on_disk();
        assert_eq!(
            descriptions(&data),
            vec!["existing", "probe", "from the tui"]
        );
        let tui_id = data
            .entries
            .iter()
            .find(|e| e.description == "from the tui")
            .unwrap()
            .id;
        assert_ne!(tui_id, agent_id, "the TUI entry reused the agent's id");
        assert_eq!(tui_id, 2);
        let mut ids: Vec<u64> = data.entries.iter().map(|e| e.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate ids in the store");
    }

    fn selected_description(app: &App) -> String {
        let idx = app.table_state.selected().expect("nothing selected");
        app.filtered_entries()[idx].description.clone()
    }

    #[test]
    fn sync_picks_up_an_outside_write_and_keeps_the_selection() {
        let _guard = env_guard();
        sandbox("sync");
        seed(vec![entry(0, "first"), entry(1, "second")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "second");
        agent_write("probe");

        app.sync_from_store().unwrap();

        assert!(descriptions(&app.data).contains(&"probe"));
        assert_eq!(selected_description(&app), "second");
    }

    #[test]
    fn sync_is_skipped_while_a_form_is_open() {
        let _guard = env_guard();
        sandbox("sync-form");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        agent_write("probe");
        for mode in [
            InputMode::AddingEntry,
            InputMode::EditingEntry,
            InputMode::Searching,
        ] {
            app.input_mode = mode;
            app.input_description = "half typed".to_string();
            app.sync_from_store().unwrap();
            assert_eq!(descriptions(&app.data), vec!["first"]);
            assert_eq!(app.input_description, "half typed");
        }

        // …and the change is picked up once the mode is Normal again.
        app.input_mode = InputMode::Normal;
        app.sync_from_store().unwrap();
        assert!(descriptions(&app.data).contains(&"probe"));
    }

    /// The phase keys of the marks the app currently holds, newest first.
    fn mark_keys(app: &App) -> Vec<String> {
        app.marks
            .iter()
            .map(|m| match &m.issue {
                Some(issue) => format!("{}.{}.{}", m.project, issue, m.phase),
                None => format!("{}.-.{}", m.project, m.phase),
            })
            .collect()
    }

    #[test]
    fn a_mark_begun_outside_the_tui_appears_on_the_next_tick_and_cancelling_removes_it() {
        let _guard = env_guard();
        sandbox("marks-tick");
        seed(vec![entry(0, "first")], 1);
        let marks = mark_sandbox();

        // Nothing open: an empty mark directory is an empty list, not an error.
        let mut app = App::new().unwrap();
        assert!(app.marks.is_empty());

        begin_mark(&marks, "tt.14.impl", 2);
        app.sync_from_marks();
        assert_eq!(mark_keys(&app), vec!["tt.14.impl"]);

        begin_mark(&marks, "vinge.-.plan", 126);
        app.sync_from_marks();
        assert_eq!(mark_keys(&app), vec!["tt.14.impl", "vinge.-.plan"]);

        // `tt agent cancel` / `tt agent end` both remove the file.
        std::fs::remove_file(marks.join("tt.14.impl")).unwrap();
        app.sync_from_marks();
        assert_eq!(mark_keys(&app), vec!["vinge.-.plan"]);
    }

    /// An in-place rewrite inside the mark directory leaves its mtime alone, so a
    /// settled stamp means no re-read.
    #[test]
    fn an_unchanged_mark_directory_is_not_read_again() {
        let _guard = env_guard();
        sandbox("marks-noread");
        seed(vec![entry(0, "first")], 1);
        let marks = mark_sandbox();
        begin_mark(&marks, "tt.14.impl", 2);

        let mut app = App::new().unwrap();
        let first = app.marks.clone();
        assert_eq!(mark_keys(&app), vec!["tt.14.impl"]);

        // Let the mtime settle out of the current second, so the stamp is trusted.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // Same length, no file added or removed: only a re-read could show this.
        begin_mark(&marks, "tt.14.impl", 999);
        app.sync_from_marks();
        assert_eq!(app.marks, first, "the directory did not change: no re-read");

        // Creating a file *does* move the directory on, so the whole list re-reads.
        begin_mark(&marks, "loremind.64.plan", 38);
        app.sync_from_marks();
        assert_eq!(mark_keys(&app), vec!["loremind.64.plan", "tt.14.impl"]);
    }

    #[test]
    fn sync_falls_back_to_a_nearby_row_when_the_selection_is_gone() {
        let _guard = env_guard();
        sandbox("sync-gone");
        seed(vec![entry(0, "a"), entry(1, "b"), entry(2, "c")], 3);

        let mut app = App::new().unwrap();
        let last = app.filtered_entries().len() - 1;
        app.table_state.select(Some(last));
        let doomed = app.filtered_entries()[last].id;
        storage::with_data(|data| {
            data.entries.retain(|e| e.id != doomed);
            Ok(())
        })
        .unwrap();

        app.sync_from_store().unwrap();

        let len = app.filtered_entries().len();
        assert_eq!(len, 2);
        assert_eq!(app.table_state.selected(), Some(len - 1));
    }

    #[test]
    fn an_untouched_store_reports_no_change_but_an_unsettled_mtime_does() {
        let _guard = env_guard();
        sandbox("sync-quiet");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        // Let the mtime fall out of the current second, past the granularity guard.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(
            app.store_is_unchanged(),
            "a quiet store should not trigger a reload"
        );

        agent_write("probe");
        assert!(!app.store_is_unchanged(), "an outside write was missed");

        // Inside the current second even an identical stamp counts as changed.
        app.store_stamp = storage::store_stamp();
        assert!(
            !app.store_is_unchanged(),
            "an unsettled stamp should not be trusted"
        );
    }

    #[test]
    fn edit_keeps_a_concurrent_agent_write() {
        let _guard = env_guard();
        sandbox("edit");
        seed(vec![entry(0, "before")], 1);

        let mut app = App::new().unwrap();
        agent_write("probe");
        select(&mut app, "before");
        app.start_editing();
        app.input_description = "after".to_string();
        app.submit_edit().unwrap();

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["after", "probe"]);
        assert_eq!(descriptions(&app.data), vec!["after", "probe"]);
    }

    /// A whitespace-only Project means "no project", and must land as JSON `null`.
    #[test]
    fn the_form_writes_the_project_and_leaves_a_blank_one_null() {
        let _guard = env_guard();
        sandbox("project-form");
        seed(Vec::new(), 0);

        let mut app = App::new().unwrap();
        app.start_adding();
        app.input_description = "with a project".to_string();
        app.input_project = "  acme  ".to_string();
        app.input_duration = "15m".to_string();
        app.submit_entry().unwrap();

        app.start_adding();
        app.input_description = "without one".to_string();
        app.input_project = "   ".to_string();
        app.input_duration = "15m".to_string();
        app.submit_entry().unwrap();

        let data = on_disk();
        let project = |desc: &str| {
            data.entries
                .iter()
                .find(|e| e.description == desc)
                .unwrap()
                .project
                .clone()
        };
        assert_eq!(project("with a project"), Some("acme".to_string()));
        assert_eq!(project("without one"), None);

        let raw = std::fs::read_to_string(storage::get_data_path().unwrap()).unwrap();
        assert!(
            raw.contains("\"project\": null"),
            "blank project not null: {raw}"
        );
        assert!(
            !raw.contains("\"project\": \"\""),
            "blank project stored as \"\": {raw}"
        );
    }

    fn dated(
        id: u64,
        description: &str,
        project: &str,
        tags: &[&str],
        date: NaiveDate,
    ) -> TimeEntry {
        logged(id, description, project, tags, date, 60)
    }

    /// [`dated`] with a duration, for the summary's per-project totals.
    fn logged(
        id: u64,
        description: &str,
        project: &str,
        tags: &[&str],
        date: NaiveDate,
        minutes: i64,
    ) -> TimeEntry {
        let start = date
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        TimeEntry {
            id,
            description: description.to_string(),
            project: (!project.is_empty()).then(|| project.to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: start,
            end_time: Some(start + chrono::Duration::minutes(minutes)),
            idle: Vec::new(),
        }
    }

    /// A logged entry with idle stretches, as minute offsets from its own start.
    fn with_idle(id: u64, minutes: i64, gaps: &[(i64, i64)]) -> TimeEntry {
        let mut entry = logged(
            id,
            "long session",
            "tt",
            &["tt"],
            Local::now().date_naive(),
            minutes,
        );
        let start = entry.start_time;
        entry.idle = gaps
            .iter()
            .map(|(from, to)| {
                crate::tracker::IdleInterval::new(
                    start + chrono::Duration::minutes(*from),
                    start + chrono::Duration::minutes(*to),
                )
            })
            .collect();
        entry
    }

    /// Two days in the current week plus one a week back, so each scope differs.
    fn seed_panes() -> App {
        let today = Local::now().date_naive();
        let week_start = TimeData::week_start(today);
        let day_one = week_start;
        let day_two = week_start + chrono::Duration::days(1);
        let last_week = week_start - chrono::Duration::days(7);
        seed(
            vec![
                dated(0, "a", "tt", &["impl", "tt/8"], day_one),
                dated(1, "b", "tt", &["plan"], day_one),
                dated(2, "c", "loremind", &["impl", "ops"], day_one),
                dated(3, "d", "vinge", &["ops"], day_two),
                dated(4, "e", "vinge", &["impl"], last_week),
                dated(5, "f", "", &[], day_one),
            ],
            6,
        );
        let mut app = App::new().unwrap();
        app.selected_date = day_one;
        app
    }

    /// A pane's rows as `value=count`, in the order they are listed.
    fn values(app: &App, pane: Pane) -> String {
        app.pane_values(pane)
            .iter()
            .map(|(value, count)| format!("{value}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Each pane offers its scope's distinct values; no project means no row.
    #[test]
    fn pane_values_follow_the_view_scope() {
        let _guard = env_guard();
        sandbox("pane-scope");
        let mut app = seed_panes();

        app.view_mode = ViewMode::Day;
        assert_eq!(values(&app, Pane::Projects), "tt=2 loremind=1");
        assert_eq!(values(&app, Pane::Tags), "impl=2 ops=1 plan=1 tt/8=1");

        app.view_mode = ViewMode::Week;
        assert_eq!(values(&app, Pane::Projects), "tt=2 loremind=1 vinge=1");
        assert_eq!(values(&app, Pane::Tags), "impl=2 ops=2 plan=1 tt/8=1");

        app.view_mode = ViewMode::All;
        assert_eq!(values(&app, Pane::Projects), "tt=2 vinge=2 loremind=1");
        assert_eq!(values(&app, Pane::Tags), "impl=3 ops=2 plan=1 tt/8=1");
    }

    #[test]
    fn pane_values_ignore_the_active_filter_and_search() {
        let _guard = env_guard();
        sandbox("pane-prefilter");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        let before = app.pane_values(Pane::Tags);

        app.selected_tags = vec!["plan".to_string()];
        app.search_term = "nothing matches this".to_string();
        assert!(app.filtered_entries().is_empty(), "filter did not bite");
        assert_eq!(app.pane_values(Pane::Tags), before);
        assert_eq!(app.pane_values(Pane::Projects).len(), 2);
    }

    #[test]
    fn the_scroll_indicator_appears_only_when_values_do_not_fit() {
        let _guard = env_guard();
        sandbox("pane-scroll-indicator");
        let today = Local::now().date_naive();
        let tags: Vec<String> = (0..8).map(|n| format!("tag{n}")).collect();
        let entries: Vec<TimeEntry> = tags
            .iter()
            .enumerate()
            .map(|(n, tag)| dated(n as u64, "x", "tt", &[tag.as_str()], today))
            .collect();
        seed(entries, 8);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.pane_values(Pane::Tags).len(), 8);

        // Six rows, eight values: the position tracks every one, wrap included.
        for expected in 1..=8 {
            assert_eq!(
                app.pane_scroll_indicator(Pane::Tags, 6).as_deref(),
                Some(format!("{expected}/8").as_str())
            );
            app.pane_next();
        }
        assert_eq!(
            app.pane_scroll_indicator(Pane::Tags, 6).as_deref(),
            Some("1/8"),
            "the cursor did not wrap back to the first value"
        );

        assert_eq!(app.pane_scroll_indicator(Pane::Tags, 8), None);
        assert_eq!(app.pane_scroll_indicator(Pane::Tags, 12), None);
        assert_eq!(app.pane_scroll_indicator(Pane::Projects, 6), None);
    }

    #[test]
    fn the_surface_has_no_height_until_a_pane_is_opened() {
        let _guard = env_guard();
        sandbox("pane-height");
        let mut app = seed_panes();
        assert!(!app.show_projects && !app.show_tags);
        assert_eq!(app.pane_surface_height(), 0);

        app.toggle_pane(Pane::Projects);
        assert!(app.pane_surface_height() > 0);
        app.toggle_pane(Pane::Projects);
        assert_eq!(app.pane_surface_height(), 0);
    }

    /// An app with `n` open marks, newest first, in a sandboxed mark directory.
    fn seed_marks(names: &[(&str, i64)]) -> App {
        let dir = mark_sandbox();
        for (key, minutes_ago) in names {
            begin_mark(&dir, key, *minutes_ago);
        }
        App::new().unwrap()
    }

    #[test]
    fn the_marks_surface_has_no_height_until_it_is_toggled_on() {
        let _guard = env_guard();
        sandbox("marks-height");
        seed(vec![entry(0, "first")], 1);
        let mut app = seed_marks(&[]);

        assert!(!app.show_marks);
        assert_eq!(app.marks_surface_height(), 0, "hidden: no row at all");

        // Empty, but open: one row, so the box can say there is nothing.
        app.toggle_marks();
        assert_eq!(app.marks_surface_height(), 3);

        // Then two borders plus one row per mark, capped at three.
        let dir = mark_sandbox();
        for (n, expected) in [(1, 3), (2, 4), (3, 5), (4, 5), (5, 5)] {
            begin_mark(&dir, &format!("proj.{n}.impl"), n);
            app.marks_stamp = None; // force a re-read; the tick would do this
            app.sync_from_marks();
            assert_eq!(app.marks.len(), n as usize);
            assert_eq!(app.marks_surface_height(), expected, "{n} marks");
        }

        app.toggle_marks();
        assert_eq!(app.marks_surface_height(), 0, "hidden again: no row again");
    }

    #[test]
    fn the_surface_lists_the_three_newest_marks_and_counts_the_rest() {
        let _guard = env_guard();
        sandbox("marks-cap");
        seed(vec![entry(0, "first")], 1);
        let app = seed_marks(&[
            ("tt.14.impl", 2),
            ("loremind.64.plan", 38),
            ("vinge.-.plan", 126),
            ("ops.-.rota", 300),
        ]);

        let shown: Vec<String> = app.visible_marks().iter().map(Mark::label).collect();
        assert_eq!(
            shown,
            vec!["tt/14 impl", "loremind/64 plan", "vinge plan"],
            "the three newest, newest first"
        );
        // Three rows on screen, four open: existence, not position.
        assert_eq!(app.marks_count(3).as_deref(), Some("3/4"));
    }

    #[test]
    fn the_border_count_reports_how_many_marks_exist() {
        let _guard = env_guard();
        sandbox("marks-count");
        seed(vec![entry(0, "first")], 1);
        let mut app = seed_marks(&[]);
        assert_eq!(app.marks_count(3), None, "nothing open: nothing to count");

        let dir = mark_sandbox();
        for (n, expected) in [(1, "1"), (2, "2"), (3, "3")] {
            begin_mark(&dir, &format!("proj.{n}.impl"), n);
            app.marks_stamp = None;
            app.sync_from_marks();
            assert_eq!(
                app.marks_count(3).as_deref(),
                Some(expected),
                "all {n} fit: a bare total"
            );
        }
    }

    #[test]
    fn toggling_the_marks_surface_leaves_focus_and_the_table_alone() {
        let _guard = env_guard();
        sandbox("marks-focus");
        seed(vec![entry(0, "first"), entry(1, "second")], 2);
        let mut app = seed_marks(&[("tt.14.impl", 2)]);

        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        app.toggle_marks();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects), "opening it");
        app.toggle_marks();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects), "closing it");

        app.toggle_marks();
        app.focus = Focus::Table;
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table, "the ring skips the marks surface");

        app.focus = Focus::Table;
        app.open_detail();
        assert!(matches!(app.input_mode, InputMode::Detail));
    }

    #[test]
    fn tab_cycles_focus_through_the_visible_panes_only() {
        let _guard = env_guard();
        sandbox("pane-focus");
        let mut app = seed_panes();

        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);

        app.toggle_pane(Pane::Tags);
        app.focus = Focus::Table;
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);

        app.toggle_pane(Pane::Projects);
        app.focus = Focus::Table;
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);
    }

    /// Opening a pane focuses it, so `j`/`k`/`Enter` drive it with no `Tab` first.
    #[test]
    fn opening_a_pane_focuses_it() {
        let _guard = env_guard();
        sandbox("pane-open-focus");
        let mut app = seed_panes();
        assert_eq!(app.focus, Focus::Table);

        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        assert_eq!(app.focused_pane(), Some(Pane::Projects));

        app.toggle_pane(Pane::Tags);
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));

        // …in either order.
        let mut app = seed_panes();
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
    }

    /// A reopened pane resumes its cursor: it lives on `App`, not on visibility.
    #[test]
    fn a_reopened_pane_resumes_its_cursor() {
        let _guard = env_guard();
        sandbox("pane-reopen-cursor");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        app.toggle_pane(Pane::Tags);
        app.pane_next();
        app.pane_next();
        assert_eq!(app.pane_cursor(Pane::Tags), 2);

        app.toggle_pane(Pane::Tags);
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        assert_eq!(app.pane_cursor(Pane::Tags), 2);
    }

    /// `Shift-Tab` undoes `Tab` for every pane-visibility combination.
    #[test]
    fn shift_tab_cycles_focus_in_the_exact_reverse_order() {
        let _guard = env_guard();
        sandbox("pane-focus-back");

        for (projects, tags) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut app = seed_panes();
            if projects {
                app.toggle_pane(Pane::Projects);
            }
            if tags {
                app.toggle_pane(Pane::Tags);
            }
            app.focus = Focus::Table;

            let ring_len = 1 + app.visible_panes().len();
            let mut forward = Vec::new();
            for _ in 0..ring_len {
                app.cycle_focus();
                forward.push(app.focus);
            }
            assert_eq!(
                app.focus,
                Focus::Table,
                "forward did not return to the table"
            );

            let mut backward = Vec::new();
            for _ in 0..ring_len {
                app.cycle_focus_back();
                backward.push(app.focus);
            }
            backward.reverse();
            let mut expected = forward.clone();
            expected.rotate_right(1);
            assert_eq!(
                backward, expected,
                "reverse cycling is not the inverse of forward for \
                 projects={projects} tags={tags}"
            );

            for _ in 0..ring_len {
                let before = app.focus;
                app.cycle_focus();
                app.cycle_focus_back();
                assert_eq!(app.focus, before, "Shift-Tab did not undo Tab");
                app.cycle_focus();
            }
        }
    }

    #[test]
    fn shift_tab_recovers_when_the_focused_pane_was_hidden() {
        let _guard = env_guard();
        sandbox("pane-focus-back-hidden");
        let mut app = seed_panes();
        app.toggle_pane(Pane::Projects);
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));

        // Bypass `toggle_pane`'s hand-off: the only way to observe this state.
        app.show_tags = false;
        assert!(app.focused_pane().is_none());
        app.cycle_focus_back();
        assert_eq!(
            app.focus,
            Focus::Pane(Pane::Projects),
            "reverse left focus off screen"
        );

        app.show_tags = true;
        app.focus = Focus::Pane(Pane::Tags);
        app.show_tags = false;
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
    }

    #[test]
    fn hiding_the_focused_pane_falls_back_to_the_other_pane_then_the_table() {
        let _guard = env_guard();
        sandbox("pane-focus-drop");
        let mut app = seed_panes();
        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));

        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Table);
        assert!(app.focused_pane().is_none());

        app.toggle_pane(Pane::Projects);
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));

        // Closing an *unfocused* pane leaves focus alone.
        app.toggle_pane(Pane::Tags);
        app.focus = Focus::Pane(Pane::Projects);
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
    }

    /// `j`/`k` wrap in the focused pane, or report "not handled" so the table moves.
    #[test]
    fn pane_cursor_moves_only_while_a_pane_has_focus() {
        let _guard = env_guard();
        sandbox("pane-cursor");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        app.table_state.select(Some(0));

        assert!(!app.pane_next(), "no pane focused, yet j was swallowed");
        assert_eq!(app.pane_cursor(Pane::Tags), 0);

        app.toggle_pane(Pane::Tags);
        let len = app.pane_values(Pane::Tags).len();
        assert_eq!(len, 4);
        assert!(app.pane_next());
        assert_eq!(app.pane_cursor(Pane::Tags), 1);
        assert!(app.pane_previous());
        assert_eq!(app.pane_cursor(Pane::Tags), 0);
        assert!(app.pane_previous());
        assert_eq!(app.pane_cursor(Pane::Tags), len - 1);
        assert!(app.pane_next());
        assert_eq!(app.pane_cursor(Pane::Tags), 0);
        assert_eq!(app.table_state.selected(), Some(0), "the table moved too");
    }

    #[test]
    fn a_stale_pane_cursor_is_clamped_to_the_new_value_list() {
        let _guard = env_guard();
        sandbox("pane-cursor-stale");
        let mut app = seed_panes();
        app.view_mode = ViewMode::All;
        app.project_cursor = 2;
        assert_eq!(app.pane_cursor(Pane::Projects), 2);

        // Day scope has fewer projects than All
        app.view_mode = ViewMode::Day;
        assert_eq!(app.pane_values(Pane::Projects).len(), 2);
        assert_eq!(app.pane_cursor(Pane::Projects), 1);
    }

    /// The descriptions of the entries currently in view, in table order.
    fn in_view(app: &App) -> Vec<String> {
        app.filtered_entries()
            .iter()
            .map(|e| e.description.clone())
            .collect()
    }

    /// Move focus onto `pane` and put its cursor on `value`.
    fn point_at(app: &mut App, pane: Pane, value: &str) {
        if !app.pane_is_visible(pane) {
            app.toggle_pane(pane);
        }
        while app.focused_pane() != Some(pane) {
            app.cycle_focus();
        }
        let idx = app
            .pane_values(pane)
            .iter()
            .position(|(v, _)| v == value)
            .unwrap_or_else(|| panic!("{value} not offered by the pane"));
        match pane {
            Pane::Projects => app.project_cursor = idx,
            Pane::Tags => app.tag_cursor = idx,
        }
    }

    /// `Enter` in a pane filters on the value under its cursor; on the table it does not.
    #[test]
    fn enter_toggles_the_value_under_the_pane_cursor() {
        let _guard = env_guard();
        sandbox("pane-toggle");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);

        point_at(&mut app, Pane::Projects, "tt");
        assert!(app.toggle_pane_value(), "Enter was not handled by the pane");
        assert_eq!(app.selected_projects, vec!["tt".to_string()]);
        assert_eq!(in_view(&app), vec!["a", "b"]);
        assert!(app.is_filtering());
        assert!(app.pane_value_is_selected(Pane::Projects, "tt"));

        assert!(app.toggle_pane_value());
        assert!(app.selected_projects.is_empty());
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);
        assert!(!app.is_filtering());

        app.focus = Focus::Table;
        assert!(!app.toggle_pane_value());
        assert!(app.selected_projects.is_empty() && app.selected_tags.is_empty());
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);
    }

    #[test]
    fn enter_opens_the_detail_popover_only_from_the_table() {
        let _guard = env_guard();
        sandbox("detail-focus");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        app.table_state.select(Some(0));

        // This is exactly what the Normal-mode Enter arm does.
        let enter = |app: &mut App| {
            if !app.toggle_pane_value() {
                app.open_detail();
            }
        };

        point_at(&mut app, Pane::Projects, "tt");
        enter(&mut app);
        assert!(
            app.input_mode == InputMode::Normal,
            "Enter in a pane opened the popover instead of filtering"
        );
        assert_eq!(app.selected_projects, vec!["tt".to_string()]);

        app.focus = Focus::Table;
        enter(&mut app);
        assert!(
            app.input_mode == InputMode::Detail,
            "Enter on the table did not open the popover"
        );
        assert_eq!(app.table_state.selected(), Some(0));
        assert_eq!(app.selected_projects, vec!["tt".to_string()]);
        assert_eq!(app.selected_entry().map(|e| e.id), Some(0));
    }

    /// What the `Detail` arm does with j/k, `e` and `d`, via the calls it makes.
    #[test]
    fn the_detail_popover_traverses_the_list_and_acts_on_what_it_shows() {
        let _guard = env_guard();
        sandbox("detail-traverse");
        seed(vec![entry(0, "a"), entry(1, "b"), entry(2, "c")], 3);

        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));
        app.open_detail();
        let ids: Vec<u64> = app.filtered_entries().iter().map(|e| e.id).collect();
        assert_eq!(app.selected_entry().map(|e| e.id), Some(ids[0]));

        app.next();
        assert_eq!(app.selected_entry().map(|e| e.id), Some(ids[1]));
        app.previous();
        assert_eq!(app.selected_entry().map(|e| e.id), Some(ids[0]));
        app.previous();
        assert_eq!(app.selected_entry().map(|e| e.id), Some(ids[2]));
        app.next();
        assert_eq!(app.selected_entry().map(|e| e.id), Some(ids[0]));
        assert!(app.input_mode == InputMode::Detail, "traversal closed it");

        app.start_editing();
        assert!(app.input_mode == InputMode::EditingEntry);
        assert_eq!(app.editing_entry_id, Some(ids[0]));

        // `d` asks, the repeated `d` answers, and the popover stays open.
        app.input_mode = InputMode::Detail;
        press_d_then(&mut app, KeyCode::Char('d'));
        assert!(app.input_mode == InputMode::Detail);
        assert_eq!(app.filtered_entries().len(), 2);
        assert!(app.selected_entry().is_some());

        // …and the delete that empties the view closes it.
        press_d_then(&mut app, KeyCode::Char('y'));
        press_d_then(&mut app, KeyCode::Char('d'));
        assert_eq!(app.filtered_entries().len(), 0);
        assert!(app.input_mode == InputMode::Normal);
    }

    #[test]
    fn t_in_the_popover_trims_the_entry_and_stays_on_the_piece_that_kept_the_id() {
        let _guard = env_guard();
        sandbox("detail-trim");
        let today = Local::now().date_naive();
        seed(
            vec![
                with_idle(4, 180, &[(30, 45), (100, 130)]),
                dated(9, "untouched", "vinge", &["ops"], today),
            ],
            10,
        );

        let mut app = App::new().unwrap();
        select(&mut app, "long session");
        app.open_detail();
        assert!(
            app.detail_hints().contains(&("t", "trim…")),
            "the footer hid the hint on an entry that has idle"
        );
        let before = app
            .selected_entry()
            .map(|e| (e.duration(), e.idle.len()))
            .unwrap();
        let idle_total = with_idle(4, 180, &[(30, 45), (100, 130)])
            .idle
            .iter()
            .fold(chrono::Duration::zero(), |acc, gap| acc + gap.duration());

        press_t_then(&mut app, KeyCode::Char('t'));

        assert!(app.input_mode == InputMode::Detail, "the trim closed it");
        assert_eq!(
            app.selected_entry().map(|e| e.id),
            Some(4),
            "the popover slid off the piece that kept the id"
        );
        // Two holes, so three pieces, plus the entry that was never touched.
        assert_eq!(app.filtered_entries().len(), before.1 + 1 + 1);
        let pieces: Vec<&TimeEntry> = app
            .filtered_entries()
            .into_iter()
            .filter(|e| e.description == "long session")
            .collect();
        assert_eq!(pieces.len(), 3);
        let after = pieces
            .iter()
            .fold(chrono::Duration::zero(), |acc, e| acc + e.duration());
        assert_eq!(after, before.0 - idle_total);
        assert!(pieces.iter().all(|e| e.idle.is_empty()));
        assert!(
            !app.detail_hints().contains(&("t", "trim…")),
            "the footer advertises a trim that would now do nothing"
        );
        assert!(
            descriptions(&on_disk()).contains(&"untouched"),
            "the trim disturbed another entry"
        );
    }

    #[test]
    fn t_on_an_entry_with_no_idle_does_nothing_and_the_footer_omits_the_hint() {
        let _guard = env_guard();
        sandbox("detail-trim-noop");
        seed(vec![entry(0, "no idle here"), entry(1, "nor here")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "no idle here");
        app.open_detail();
        assert!(
            !app.detail_hints().contains(&("t", "trim…")),
            "the footer advertised a no-op"
        );
        let before = serde_json::to_string(&on_disk()).unwrap();

        press_t_then(&mut app, KeyCode::Char('t'));

        assert!(app.input_mode == InputMode::Detail);
        assert_eq!(selected_description(&app), "no idle here");
        assert_eq!(
            serde_json::to_string(&on_disk()).unwrap(),
            before,
            "a no-op trim rewrote the store"
        );
    }

    #[test]
    fn t_outside_the_popover_still_jumps_to_today() {
        let _guard = env_guard();
        sandbox("detail-trim-normal-t");
        seed(vec![with_idle(4, 180, &[(30, 45)])], 5);

        let mut app = App::new().unwrap();
        app.previous_period();
        assert_ne!(app.selected_date, Local::now().date_naive());

        // What the `Normal` arm's `t` calls.
        app.go_to_today();

        assert_eq!(app.selected_date, Local::now().date_naive());
        assert_eq!(on_disk().entries.len(), 1, "the table's `t` split an entry");
    }

    #[test]
    fn requesting_a_confirmation_records_the_selected_id_and_writes_nothing() {
        let _guard = env_guard();
        sandbox("confirm-request");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "doomed");
        let before = serde_json::to_string(&on_disk()).unwrap();

        app.request_confirm(ConfirmAction::Delete);

        assert!(app.input_mode == InputMode::Confirm);
        let pending = app.pending_confirm.expect("a pending confirmation");
        assert_eq!(pending.action, ConfirmAction::Delete);
        assert_eq!(pending.entry_id, 1, "the prompt pinned the selected entry");
        assert!(pending.from == InputMode::Normal, "raised from the table");
        assert_eq!(
            serde_json::to_string(&on_disk()).unwrap(),
            before,
            "asking the question wrote to the store"
        );
    }

    #[test]
    fn a_confirmed_delete_acts_on_the_captured_id_not_the_current_selection() {
        let _guard = env_guard();
        sandbox("confirm-captured");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "doomed");
        app.request_confirm(ConfirmAction::Delete);
        // The cursor moves out from under the prompt.
        app.next();
        assert_ne!(app.selected_entry().map(|e| e.id), Some(1));

        app.confirm_pending().unwrap();

        assert_eq!(descriptions(&on_disk()), vec!["keep"]);
        assert!(app.pending_confirm.is_none());
        assert!(app.input_mode == InputMode::Normal);
    }

    /// The live poll can drop the cursor onto a different entry; the id keeps it on target.
    #[test]
    fn the_poll_moving_the_cursor_under_the_prompt_does_not_move_the_target() {
        let _guard = env_guard();
        sandbox("confirm-poll-moves");
        let today = Local::now().date_naive();
        seed(
            vec![
                dated(1, "doomed", "tt", &["tt"], today),
                dated(2, "bystander", "tt", &["tt"], today),
            ],
            3,
        );

        let mut app = App::new().unwrap();
        select(&mut app, "doomed");
        app.request_confirm(ConfirmAction::Delete);

        // The pending entry leaves the day view, so the cursor falls back by position.
        storage::with_data(|data| {
            let e = data.entries.iter_mut().find(|e| e.id == 1).unwrap();
            e.start_time -= chrono::Duration::days(1);
            e.end_time = Some(e.start_time + chrono::Duration::hours(1));
            Ok(())
        })
        .unwrap();
        app.sync_from_store().unwrap();
        assert_eq!(
            app.selected_entry().map(|e| e.id),
            Some(2),
            "the fixture no longer reproduces the cursor moving under the prompt"
        );
        assert!(app.input_mode == InputMode::Confirm, "the poll closed it");

        app.confirm_pending().unwrap();

        assert_eq!(
            descriptions(&on_disk()),
            vec!["bystander"],
            "the confirm destroyed an entry the prompt never named"
        );
    }

    #[test]
    fn a_confirm_whose_target_vanished_performs_nothing() {
        let _guard = env_guard();
        sandbox("confirm-vanished");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "doomed");
        app.open_detail();
        app.request_confirm(ConfirmAction::Delete);

        // Someone else removes it while the prompt is on screen.
        storage::with_data(|data| {
            data.entries.retain(|e| e.id != 1);
            Ok(())
        })
        .unwrap();
        app.sync_from_store().unwrap();
        assert!(app.pending_confirm.is_none());
        assert!(
            app.input_mode == InputMode::Detail,
            "the popover still has a row"
        );

        // …and a confirm arriving on a prompt that is already gone is inert too.
        app.confirm_pending().unwrap();
        assert_eq!(descriptions(&on_disk()), vec!["keep"]);
    }

    #[test]
    fn cancelling_restores_the_originating_mode_and_leaves_the_store_alone() {
        let _guard = env_guard();
        sandbox("confirm-cancel");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "doomed");
        let before = serde_json::to_string(&on_disk()).unwrap();

        app.request_confirm(ConfirmAction::Delete);
        app.cancel_confirm();
        assert!(app.input_mode == InputMode::Normal);
        assert!(app.pending_confirm.is_none());
        assert_eq!(selected_description(&app), "doomed", "the cursor moved");

        // From the popover, which reopens on the same entry.
        app.open_detail();
        app.request_confirm(ConfirmAction::Delete);
        app.cancel_confirm();
        assert!(app.input_mode == InputMode::Detail);
        assert_eq!(selected_description(&app), "doomed");

        assert_eq!(
            serde_json::to_string(&on_disk()).unwrap(),
            before,
            "a cancelled confirmation wrote to the store"
        );
    }

    #[test]
    fn a_confirmed_trim_splits_the_captured_entry_and_stays_on_the_first_piece() {
        let _guard = env_guard();
        sandbox("confirm-trim");
        seed(vec![with_idle(4, 180, &[(30, 45), (100, 130)])], 5);

        let mut app = App::new().unwrap();
        select(&mut app, "long session");
        app.open_detail();
        app.request_confirm(ConfirmAction::Trim);
        assert_eq!(
            app.pending_confirm.map(|p| (p.action, p.entry_id)),
            Some((ConfirmAction::Trim, 4))
        );
        assert_eq!(on_disk().entries.len(), 1, "the prompt trimmed on its own");

        app.confirm_pending().unwrap();

        assert_eq!(
            on_disk().entries.len(),
            3,
            "two holes should give three pieces"
        );
        assert!(
            app.input_mode == InputMode::Detail,
            "the trim closed the popover"
        );
        assert_eq!(app.selected_entry().map(|e| e.id), Some(4));
    }

    #[test]
    fn only_y_or_the_originating_key_is_a_yes() {
        let _guard = env_guard();
        sandbox("confirm-keys");
        seed(vec![with_idle(4, 180, &[(30, 45)])], 5);

        let mut app = App::new().unwrap();
        select(&mut app, "long session");
        assert!(!app.confirms_pending('y'), "no prompt, no yes");

        app.request_confirm(ConfirmAction::Delete);
        assert!(app.confirms_pending('d'));
        assert!(app.confirms_pending('y'));
        assert!(!app.confirms_pending('t'), "`t` confirmed a delete");
        assert!(!app.confirms_pending('n'));
        app.cancel_confirm();

        app.open_detail();
        app.request_confirm(ConfirmAction::Trim);
        assert!(app.confirms_pending('t'));
        assert!(app.confirms_pending('y'));
        assert!(!app.confirms_pending('d'), "`d` confirmed a trim");
    }

    /// One rendered frame as lines of text, so assertions read the real screen.
    fn frame_lines(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render::ui(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn the_delete_prompt_names_the_entry_it_would_destroy() {
        let _guard = env_guard();
        sandbox("confirm-render-delete");
        let today = Local::now().date_naive();
        seed(
            vec![dated(12, "pane cursor markers", "tt", &["tt"], today)],
            13,
        );

        let mut app = App::new().unwrap();
        select(&mut app, "pane cursor markers");
        app.request_confirm(ConfirmAction::Delete);
        let screen = frame_lines(&mut app, 100, 30).join("\n");

        assert!(
            screen.contains("Delete entry #12?"),
            "the title did not ask about the captured entry:\n{screen}"
        );
        assert!(
            screen.contains("pane cursor markers (tt)"),
            "the prompt did not name the entry:\n{screen}"
        );
        let duration = app.data.get_entry(12).unwrap().format_duration();
        assert!(
            screen.contains(&duration),
            "the prompt did not state the duration {duration}:\n{screen}"
        );
        assert!(screen.contains("d / y yes"), "hint row:\n{screen}");
        assert!(
            screen.contains("n / esc / enter cancel"),
            "hint row:\n{screen}"
        );
        assert!(!screen.contains("t / y"), "a delete prompt offered `t`");
    }

    /// The trim prompt states its outcome, from the same helper the write uses.
    #[test]
    fn the_trim_prompt_states_the_pieces_and_what_is_removed() {
        let _guard = env_guard();
        sandbox("confirm-render-trim");
        seed(vec![with_idle(14, 110, &[(25, 45), (85, 100)])], 15);

        let mut app = App::new().unwrap();
        select(&mut app, "long session");
        app.open_detail();
        app.request_confirm(ConfirmAction::Trim);
        let screen = frame_lines(&mut app, 100, 30).join("\n");

        assert!(
            screen.contains("Trim entry #14?"),
            "the title did not ask about a trim:\n{screen}"
        );
        // Two holes, so three pieces: 0-25, 45-85, 100-110.
        assert!(
            screen.contains("3 pieces: 0h 25m, 0h 40m, 0h 10m"),
            "the prompt did not state the pieces:\n{screen}"
        );
        assert!(
            screen.contains("0h 35m removed"),
            "the prompt did not state what it removes:\n{screen}"
        );
        assert!(screen.contains("t / y yes"), "hint row:\n{screen}");
        assert!(
            !screen.contains("split"),
            "the user-facing verb is trim, never split:\n{screen}"
        );
    }

    #[test]
    fn the_key_hints_say_the_destructive_keys_ask_first() {
        let _guard = env_guard();
        sandbox("confirm-render-hints");
        seed(vec![with_idle(4, 180, &[(30, 45)])], 5);

        let mut app = App::new().unwrap();
        select(&mut app, "long session");

        // The footer legend, which clips at 80 columns, so it gets a bare `…`.
        let footer = frame_lines(&mut app, 200, 30).join("\n");
        assert!(footer.contains("d: del…"), "footer legend:\n{footer}");

        app.input_mode = InputMode::Help;
        let help = frame_lines(&mut app, 100, 40).join("\n");
        assert!(
            help.contains("delete selected entry (asks first)"),
            "help popup:\n{help}"
        );

        app.input_mode = InputMode::Detail;
        let popover = frame_lines(&mut app, 100, 40).join("\n");
        assert!(popover.contains("d delete…"), "popover hints:\n{popover}");
        assert!(popover.contains("t trim…"), "popover hints:\n{popover}");
    }

    #[test]
    fn each_answer_key_does_exactly_what_the_hint_row_says() {
        let _guard = env_guard();
        sandbox("confirm-answer-keys");

        // Yes, by the originating key and by `y`.
        for yes in [KeyCode::Char('d'), KeyCode::Char('y')] {
            seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);
            let mut app = App::new().unwrap();
            select(&mut app, "doomed");
            press_d_then(&mut app, yes);
            assert_eq!(
                descriptions(&on_disk()),
                vec!["keep"],
                "{yes:?} was not taken as a yes"
            );
            assert!(app.input_mode == InputMode::Normal);
        }

        // No, by every route out, `Enter` among them.
        for no in [KeyCode::Char('n'), KeyCode::Esc, KeyCode::Enter] {
            seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);
            let mut app = App::new().unwrap();
            select(&mut app, "doomed");
            app.open_detail();
            press_d_then(&mut app, no);
            assert_eq!(
                descriptions(&on_disk()),
                vec!["keep", "doomed"],
                "{no:?} destroyed something"
            );
            assert!(
                app.input_mode == InputMode::Detail,
                "{no:?} left the popover"
            );
            assert!(app.pending_confirm.is_none());
        }

        // Neither the other destructive key nor a stray press dismisses the prompt.
        for inert in [KeyCode::Char('t'), KeyCode::Char('j'), KeyCode::Char('q')] {
            seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);
            let mut app = App::new().unwrap();
            select(&mut app, "doomed");
            press_d_then(&mut app, inert);
            assert_eq!(
                descriptions(&on_disk()),
                vec!["keep", "doomed"],
                "{inert:?} confirmed a delete"
            );
            assert!(
                app.input_mode == InputMode::Confirm,
                "{inert:?} closed the prompt"
            );
            assert!(app.pending_confirm.is_some());
        }
    }

    #[test]
    fn the_trim_prompt_takes_t_and_y_and_ignores_d() {
        let _guard = env_guard();
        sandbox("confirm-trim-keys");

        for yes in [KeyCode::Char('t'), KeyCode::Char('y')] {
            seed(vec![with_idle(4, 180, &[(30, 45), (100, 130)])], 5);
            let mut app = App::new().unwrap();
            select(&mut app, "long session");
            app.open_detail();
            press_t_then(&mut app, yes);
            assert_eq!(on_disk().entries.len(), 3, "{yes:?} was not taken as a yes");
            assert!(app.input_mode == InputMode::Detail);
            assert_eq!(app.selected_entry().map(|e| e.id), Some(4));
        }

        seed(vec![with_idle(4, 180, &[(30, 45), (100, 130)])], 5);
        let mut app = App::new().unwrap();
        select(&mut app, "long session");
        app.open_detail();
        press_t_then(&mut app, KeyCode::Char('d'));
        assert_eq!(on_disk().entries.len(), 1, "`d` confirmed a trim");
        assert!(app.input_mode == InputMode::Confirm);
    }

    /// `t` outside the popover is navigation, and `s` acts without a prompt.
    #[test]
    fn the_keys_that_were_not_wired_still_mean_what_they_did() {
        let _guard = env_guard();
        sandbox("confirm-unwired-keys");
        seed(
            vec![with_idle(4, 180, &[(30, 45)]), entry(9, "running")],
            10,
        );

        let mut app = App::new().unwrap();
        app.previous_period();

        // What the `Normal` arm's `t` calls.
        app.go_to_today();
        assert_eq!(app.selected_date, Local::now().date_naive());
        assert!(
            app.pending_confirm.is_none(),
            "the table's `t` raised a prompt"
        );
        assert_eq!(on_disk().entries.len(), 2, "the table's `t` split an entry");

        // …and what its `s` calls, which acts at once and asks nothing.
        app.stop_active().unwrap();
        assert!(app.pending_confirm.is_none(), "`s` raised a prompt");
        assert!(app.input_mode == InputMode::Normal);
    }

    #[test]
    fn a_trim_with_nothing_to_trim_raises_no_prompt() {
        let _guard = env_guard();
        sandbox("confirm-trim-noop");
        seed(vec![entry(0, "no idle here")], 1);

        let mut app = App::new().unwrap();
        select(&mut app, "no idle here");
        app.open_detail();

        app.request_confirm(ConfirmAction::Trim);

        assert!(app.pending_confirm.is_none());
        assert!(app.input_mode == InputMode::Detail);
    }

    /// `Detail` is *not* in `sync_from_store`'s guarded set; the poll re-anchors on the
    /// selected id. `selected_entry` is positional, so `trim_entry` re-selects by id.
    #[test]
    fn an_outside_write_reaches_the_open_detail_popover_without_moving_it() {
        let _guard = env_guard();
        sandbox("detail-sync");
        seed(vec![entry(0, "first"), entry(1, "second")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "second");
        app.open_detail();
        let shown = app.selected_entry().map(|e| e.id);
        agent_write("probe");

        app.sync_from_store().unwrap();

        assert!(descriptions(&app.data).contains(&"probe"));
        assert!(app.input_mode == InputMode::Detail);
        assert_eq!(
            app.selected_entry().map(|e| e.id),
            shown,
            "the popover changed entry under the reader"
        );
        assert_eq!(selected_description(&app), "second");
    }

    #[test]
    fn the_detail_popover_stays_shut_with_nothing_selected() {
        let _guard = env_guard();
        sandbox("detail-empty");
        seed(vec![], 0);
        let mut app = App::new().unwrap();
        assert!(app.selected_entry().is_none());

        app.open_detail();
        assert!(app.input_mode == InputMode::Normal);
    }

    #[test]
    fn selections_or_within_a_pane_and_and_across_panes() {
        let _guard = env_guard();
        sandbox("pane-filter-semantics");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;

        point_at(&mut app, Pane::Projects, "tt");
        app.toggle_pane_value();
        assert_eq!(in_view(&app), vec!["a", "b"]);

        // A second project widens the set — the two are OR'd.
        point_at(&mut app, Pane::Projects, "loremind");
        app.toggle_pane_value();
        assert_eq!(in_view(&app), vec!["a", "b", "c"]);

        // A tag narrows within them — the panes are AND'd.
        point_at(&mut app, Pane::Tags, "impl");
        app.toggle_pane_value();
        assert_eq!(in_view(&app), vec!["a", "c"]);

        point_at(&mut app, Pane::Tags, "plan");
        app.toggle_pane_value();
        assert_eq!(in_view(&app), vec!["a", "b", "c"]);

        app.clear_filters();
        assert!(!app.is_filtering());
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);
    }

    /// The footer's total is the filtered one, so it describes the rows on screen.
    #[test]
    fn the_filtered_total_tracks_the_selection() {
        let _guard = env_guard();
        sandbox("pane-filter-total");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        // Four one-hour entries in scope.
        assert_eq!(app.filtered_total().num_hours(), 4);

        point_at(&mut app, Pane::Projects, "tt");
        app.toggle_pane_value();
        assert_eq!(app.filtered_total().num_hours(), 2);
        assert!(app.is_filtering());
    }

    /// Unequal per-project times across three scopes, two unprojected entries and a
    /// tie the name breaks. Day: tt 90m/2, loremind 90m, none 30m — 210m. Week adds
    /// vinge 120m and none 45m — 375m. All adds loremind 60m — 435m.
    fn seed_summary() -> App {
        let today = Local::now().date_naive();
        let week_start = TimeData::week_start(today);
        let day_one = week_start;
        let day_two = week_start + chrono::Duration::days(1);
        let last_week = week_start - chrono::Duration::days(7);
        seed(
            vec![
                logged(0, "a", "tt", &["impl"], day_one, 60),
                logged(1, "b", "tt", &["plan"], day_one, 30),
                logged(2, "c", "loremind", &["impl"], day_one, 90),
                logged(3, "d", "", &[], day_one, 30),
                logged(4, "e", "vinge", &["ops"], day_two, 120),
                logged(5, "f", "  ", &["ops"], day_two, 45),
                logged(6, "g", "loremind", &["impl"], last_week, 60),
            ],
            7,
        );
        let mut app = App::new().unwrap();
        app.selected_date = day_one;
        app
    }

    /// The three view scopes with a name to report against; `ViewMode` is not `Debug`.
    fn scopes() -> [(ViewMode, &'static str); 3] {
        [
            (ViewMode::Day, "day"),
            (ViewMode::Week, "week"),
            (ViewMode::All, "all"),
        ]
    }

    /// The summary's rows as `project=minutes/entries/share%`, in order.
    fn summary(app: &App) -> String {
        app.project_summary()
            .iter()
            .map(|row| {
                format!(
                    "{}={}m/{}/{}%",
                    row.project,
                    row.total.num_minutes(),
                    row.entries,
                    row.share
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Totals per scope, largest first, ties by name, absence collapsed into one row.
    #[test]
    fn project_summary_totals_and_counts_follow_the_view_scope() {
        let _guard = env_guard();
        sandbox("summary-scope");
        let mut app = seed_summary();

        app.view_mode = ViewMode::Day;
        assert_eq!(
            summary(&app),
            "loremind=90m/1/43% tt=90m/2/43% (no project)=30m/1/14%"
        );

        app.view_mode = ViewMode::Week;
        assert_eq!(
            summary(&app),
            "vinge=120m/1/32% loremind=90m/1/24% tt=90m/2/24% (no project)=75m/2/20%"
        );

        app.view_mode = ViewMode::All;
        assert_eq!(
            summary(&app),
            "loremind=150m/2/34% vinge=120m/1/28% tt=90m/2/21% (no project)=75m/2/17%"
        );

        for (mode, name) in scopes() {
            app.view_mode = mode;
            assert_eq!(
                app.project_summary()
                    .iter()
                    .filter(|row| row.project == super::summary::NO_PROJECT)
                    .count(),
                1,
                "{name} did not collapse absence into one row"
            );
        }
    }

    /// The rows sum to the scope total, and their shares to within a point of 100.
    #[test]
    fn project_summary_rows_account_for_the_whole_scope() {
        let _guard = env_guard();
        sandbox("summary-sums");
        let mut app = seed_summary();

        for (mode, name) in scopes() {
            app.view_mode = mode;
            let rows = app.project_summary();
            let scope_total: i64 = app
                .scope_entries()
                .iter()
                .map(|e| e.duration().num_seconds())
                .sum();
            let summed: i64 = rows.iter().map(|r| r.total.num_seconds()).sum();
            assert_eq!(summed, scope_total, "{name} rows do not sum to the scope");
            let entries: usize = rows.iter().map(|r| r.entries).sum();
            assert_eq!(entries, app.scope_entries().len(), "{name} entry counts");

            let shares: u32 = rows.iter().map(|r| u32::from(r.share)).sum();
            assert!(
                (99..=101).contains(&shares),
                "{name} shares sum to {shares}, not ~100"
            );
        }
    }

    #[test]
    fn project_summary_ignores_the_active_filter_and_search() {
        let _guard = env_guard();
        sandbox("summary-prefilter");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        let before = app.project_summary();
        let in_scope = app.scope_entries().len();

        app.selected_projects = vec!["tt".to_string()];
        assert!(
            app.filtered_entries().len() < in_scope,
            "filter did not bite"
        );
        assert_eq!(app.project_summary(), before);

        app.selected_projects.clear();
        app.selected_tags = vec!["ops".to_string()];
        assert!(
            app.filtered_entries().len() < in_scope,
            "filter did not bite"
        );
        assert_eq!(app.project_summary(), before);

        app.search_term = "nothing matches this".to_string();
        assert!(app.filtered_entries().is_empty());
        assert_eq!(app.project_summary(), before);
    }

    #[test]
    fn an_empty_or_zero_length_scope_summarises_without_dividing_by_zero() {
        let _guard = env_guard();
        sandbox("summary-empty");
        seed(Vec::new(), 0);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        assert!(app.project_summary().is_empty());
        app.view_mode = ViewMode::All;
        assert!(app.project_summary().is_empty());

        // A populated store still has empty scopes: a day nobody worked.
        let today = Local::now().date_naive();
        seed(vec![logged(0, "a", "tt", &["impl"], today, 60)], 1);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.selected_date = today - chrono::Duration::days(400);
        assert!(app.project_summary().is_empty());

        // Zero-length entries: rows exist, and the shares are 0%.
        seed(
            vec![
                logged(0, "a", "tt", &[], today, 0),
                logged(1, "b", "", &[], today, 0),
            ],
            2,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        // Nothing separates them by total, so the name tie-break decides.
        assert_eq!(summary(&app), "(no project)=0m/1/0% tt=0m/1/0%");
    }

    /// Hidden, the surface has no height, so `ui` leaves its row out of the plan.
    #[test]
    fn the_summary_surface_has_no_height_until_it_is_toggled_on() {
        let _guard = env_guard();
        sandbox("summary-height");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Day;

        assert!(!app.show_summary);
        assert_eq!(app.summary_surface_height(), 0, "hidden: no row at all");

        // Two borders plus one row per project: the day has three.
        app.toggle_summary();
        assert_eq!(app.summary_surface_height(), 5);
        // Re-scoping re-sizes it: the week has four projects, all entries too.
        app.view_mode = ViewMode::Week;
        assert_eq!(app.summary_surface_height(), 6);

        // An empty scope still gets one row, so the box can say it is empty.
        app.view_mode = ViewMode::Day;
        app.selected_date = Local::now().date_naive() - chrono::Duration::days(400);
        assert!(app.project_summary().is_empty());
        assert_eq!(app.summary_surface_height(), 3);

        app.toggle_summary();
        assert_eq!(app.summary_surface_height(), 0, "hidden again: no row");
    }

    #[test]
    fn toggling_the_summary_surface_leaves_focus_and_the_table_alone() {
        let _guard = env_guard();
        sandbox("summary-focus");
        let mut app = seed_summary();
        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        app.table_state.select(Some(1));

        app.toggle_summary();
        assert!(app.show_summary);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        assert_eq!(app.table_state.selected(), Some(1));

        app.toggle_summary();
        assert!(!app.show_summary);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        assert_eq!(app.table_state.selected(), Some(1));
    }

    #[test]
    fn the_title_marker_names_the_scope_and_always_says_all_projects() {
        let _guard = env_guard();
        sandbox("summary-marker");
        let mut app = seed_summary();
        app.toggle_summary();

        for (mode, word) in scopes() {
            app.set_view_mode(mode);
            assert_eq!(app.summary_marker(6), format!("{word} · all projects"));
        }

        // A filter changes the emphasis, never the words.
        app.set_view_mode(ViewMode::Day);
        assert!(!app.total_is_filtered());
        let unfiltered = app.summary_marker(6);
        app.selected_projects = vec!["tt".to_string()];
        assert!(app.total_is_filtered(), "the footer total is now narrowed");
        assert_eq!(app.summary_marker(6), unfiltered);
    }

    /// Overflow says `shown/total` off the frame's real height, and nothing while all fit.
    #[test]
    fn more_projects_than_fit_are_counted_on_the_title() {
        let _guard = env_guard();
        sandbox("summary-overflow");
        let today = Local::now().date_naive();
        let entries: Vec<TimeEntry> = (0..9)
            .map(|n| logged(n, "x", &format!("p{n}"), &[], today, 30 + n as i64))
            .collect();
        seed(entries, 9);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.toggle_summary();

        // Capped at six rows, so nine projects overflow: `6/9`, on the one title.
        assert_eq!(app.summary_surface_height(), 8);
        assert_eq!(app.summary_count(6).as_deref(), Some("6/9"));
        assert_eq!(app.summary_marker(6), "day · all projects · 6/9");
        assert_eq!(app.visible_project_summary(6).len(), 6);

        // A shorter box counts what *it* left out, not what the cap would have.
        assert_eq!(app.summary_marker(2), "day · all projects · 2/9");
        assert_eq!(app.visible_project_summary(2).len(), 2);

        assert_eq!(app.summary_count(9), None);
        assert_eq!(app.summary_marker(9), "day · all projects");
    }

    #[test]
    fn the_rows_sum_to_the_footer_total_until_a_filter_narrows_it() {
        let _guard = env_guard();
        sandbox("summary-vs-footer");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;

        let rows = app.project_summary();
        let summed: chrono::Duration = rows
            .iter()
            .fold(chrono::Duration::zero(), |acc, row| acc + row.total);
        // Unfiltered, the footer prints the scope total for the week.
        let week_start = TimeData::week_start(app.selected_date);
        assert!(!app.total_is_filtered());
        assert_eq!(summed, app.data.total_for_week(week_start));

        app.selected_projects = vec!["tt".to_string()];
        assert!(app.total_is_filtered(), "the marker is now emphasised");
        assert!(
            app.filtered_total() < summed,
            "the footer total should have dropped below the summary's"
        );
        assert_eq!(app.project_summary(), rows, "the summary must not move");
    }

    #[test]
    fn search_matches_every_field_a_row_shows() {
        let _guard = env_guard();
        sandbox("search-any-field");
        let today = Local::now().date_naive();
        let start = today
            .and_hms_opt(14, 30, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        seed(
            vec![
                TimeEntry {
                    id: 42,
                    description: "wrote the migration".to_string(),
                    project: Some("loremind".to_string()),
                    tags: vec!["impl".to_string()],
                    start_time: start,
                    end_time: Some(start + chrono::Duration::hours(2)),
                    idle: Vec::new(),
                },
                dated(7, "unrelated", "vinge", &["ops"], today),
            ],
            43,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        assert_eq!(in_view(&app).len(), 2);

        for needle in [
            "migration", // description
            "LOREMIND",  // project, case-insensitively
            "impl",      // a tag
            "42",        // the id
            "14:",       // a fragment of the start time
            "16:30",     // the end time
            "2h 0m",     // the formatted duration
            &today.format("%Y-%m-%d").to_string(),
        ] {
            app.search_term = needle.to_string();
            let view = in_view(&app);
            assert!(
                view.contains(&"wrote the migration".to_string()),
                "search {needle:?} missed the entry: {view:?}"
            );
        }

        // The date matches both entries; every other needle above is unique to one.
        app.search_term = "loremind".to_string();
        assert_eq!(in_view(&app), vec!["wrote the migration"]);
        app.search_term = "no such thing".to_string();
        assert!(in_view(&app).is_empty());
    }

    /// `e` pre-fills the Project field, and clearing it drops the project.
    #[test]
    fn editing_round_trips_the_project_field() {
        let _guard = env_guard();
        sandbox("project-edit");
        let mut seeded = entry(0, "has a project");
        seeded.project = Some("acme".to_string());
        seed(vec![seeded], 1);

        let mut app = App::new().unwrap();
        select(&mut app, "has a project");
        app.start_editing();
        assert_eq!(app.input_project, "acme");

        app.input_project = "beta".to_string();
        app.submit_edit().unwrap();
        assert_eq!(on_disk().entries[0].project, Some("beta".to_string()));

        select(&mut app, "has a project");
        app.start_editing();
        app.input_project.clear();
        app.submit_edit().unwrap();
        assert_eq!(on_disk().entries[0].project, None);
    }
}
