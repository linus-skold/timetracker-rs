//! The whole key map, in one place and reachable without a terminal:
//! `run_tui` only reads events, this decides what they mean.

use super::App;
use super::npx_available;
use super::{ConfirmAction, InputMode, OnboardingStep, Pane, ViewMode};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Dispatches one key press against the current input mode.
pub(crate) fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match app.input_mode {
        InputMode::Normal => normal(app, key)?,
        InputMode::Onboarding => onboarding(app, key)?,
        InputMode::AddingEntry => form(app, key, App::submit_entry)?,
        InputMode::EditingEntry => form(app, key, App::submit_edit)?,
        InputMode::Searching => searching(app, key),
        InputMode::Help => help(app, key),
        InputMode::Detail => detail(app, key),
        // Which key is a yes depends on the pending action, so the
        // whole answer lives in `answer_confirm`.
        InputMode::Confirm => app.answer_confirm(key.code)?,
    }
    Ok(())
}

fn normal(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
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
        // One key, disambiguated by focus: `cycle_pane_value`
        // reporting false *is* the focus check.
        KeyCode::Enter => {
            if !app.cycle_pane_value(true) {
                app.open_detail();
            }
        }
        // Reverse cycle; without pane focus it does nothing.
        KeyCode::Char('-') => {
            app.cycle_pane_value(false);
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
        KeyCode::Char('4') => app.set_view_mode(ViewMode::Overview),
        KeyCode::Char('h') | KeyCode::Left => app.previous_period(),
        KeyCode::Char('l') | KeyCode::Right => app.next_period(),
        KeyCode::Char('t') => app.go_to_today(),
        KeyCode::Char('o') => app.toggle_sort_order(),
        KeyCode::Char('?') => {
            app.help_scroll = 0;
            app.input_mode = InputMode::Help;
        }
        _ => {}
    }
    Ok(())
}

fn onboarding(app: &mut App, key: KeyEvent) -> Result<()> {
    match app.onboarding_step {
        OnboardingStep::Layout => match key.code {
            KeyCode::Char('j') | KeyCode::Down => app.onboarding_move(1),
            KeyCode::Char('k') | KeyCode::Up => app.onboarding_move(-1),
            KeyCode::Char(' ') | KeyCode::Enter => app.onboarding_toggle(),
            KeyCode::Char('s') => app.onboarding_apply_layout(),
            KeyCode::Esc => app.onboarding_skip()?,
            _ => {}
        },
        OnboardingStep::Skill => match key.code {
            KeyCode::Char('y') => {
                if npx_available() {
                    app.onboarding_skill_error = None;
                    app.request_skill_install = true;
                } else {
                    app.onboarding_skill_error = Some(
                        "npx not found on PATH — install Node.js, or run \
                                             this later yourself; see AGENTS.md."
                            .to_string(),
                    );
                }
            }
            KeyCode::Char('n') | KeyCode::Enter => app.onboarding_finish()?,
            KeyCode::Esc => app.onboarding_skip()?,
            _ => {}
        },
    }
    Ok(())
}

/// The entry form, in both its modes. They differ only in what `Enter` submits.
fn form(app: &mut App, key: KeyEvent, submit: fn(&mut App) -> Result<()>) -> Result<()> {
    match key.code {
        KeyCode::Esc => app.cancel_adding(),
        KeyCode::Enter => submit(app)?,
        KeyCode::Tab => app.next_input_field(),
        KeyCode::BackTab => app.prev_input_field(),
        KeyCode::Backspace => app.handle_input_backspace(),
        KeyCode::Char(c) => app.handle_input_char(c),
        _ => {
            cursor_key(app, key);
        }
    }
    Ok(())
}

fn searching(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.clear_search(),
        KeyCode::Enter => app.confirm_search(),
        KeyCode::Backspace => app.handle_search_backspace(),
        KeyCode::Char(c) => app.handle_search_char(c),
        _ => {
            cursor_key(app, key);
        }
    }
}

fn help(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            app.help_scroll = 0;
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Char('j') | KeyCode::Down => app.help_scroll += 1,
        KeyCode::Char('k') | KeyCode::Up => app.help_scroll = app.help_scroll.saturating_sub(1),
        _ => {}
    }
}

// Modal, but not inert: the popover renders whatever
// `selected_entry()` returns, so j/k alone make it follow.
fn detail(app: &mut App, key: KeyEvent) {
    match key.code {
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
    }
}

/// Cursor movement shared by both form modes and the search bar; ctrl
/// makes it a word jump. Reports whether the key was one of its own.
fn cursor_key(app: &mut App, key: KeyEvent) -> bool {
    let word = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Left => {
            if word {
                app.move_cursor_word_left();
            } else {
                app.move_cursor_left();
            }
            true
        }
        KeyCode::Right => {
            if word {
                app.move_cursor_word_right();
            } else {
                app.move_cursor_right();
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;
    use crate::storage::env_guard;
    use crate::storage::env_sandbox as sandbox;
    use crate::tracker::{TimeData, TimeEntry};
    use crate::tui::types::{Focus, InputField, SortOrder};
    use chrono::Local;

    fn press(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    fn press_ctrl(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::CONTROL)).unwrap();
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
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

    fn entry(id: u64, description: &str) -> TimeEntry {
        TimeEntry {
            id,
            description: description.to_string(),
            project: None,
            tags: Vec::new(),
            start_time: Local::now() - chrono::Duration::hours(2),
            end_time: Some(Local::now() - chrono::Duration::hours(1)),
            idle: Vec::new(),
            data: None,
        }
    }

    /// An app in `AddingEntry` with `text` typed into the description field.
    fn adding_with(text: &str) -> App {
        let mut app = App::new().unwrap();
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.input_mode, InputMode::AddingEntry);
        type_str(&mut app, text);
        app
    }

    #[test]
    fn typing_and_backspace_edit_the_focused_field() {
        let _guard = env_guard();
        sandbox("keys-typing");
        let mut app = adding_with("hello");

        assert_eq!(app.input_description.value(), "hello");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.input_description.value(), "hell");
    }

    #[test]
    fn tab_and_backtab_walk_the_form_fields() {
        let _guard = env_guard();
        sandbox("keys-tab");
        let mut app = adding_with("hello");

        press(&mut app, KeyCode::Tab);
        assert_eq!(app.input_field, InputField::Project);
        type_str(&mut app, "acme");
        assert_eq!(app.input_project.value(), "acme");

        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.input_field, InputField::Description);
        // The description is untouched by the detour.
        assert_eq!(app.input_description.value(), "hello");
    }

    #[test]
    fn left_and_right_move_the_form_cursor_one_char() {
        let _guard = env_guard();
        sandbox("keys-cursor");
        let mut app = adding_with("abc");

        press(&mut app, KeyCode::Left);
        assert_eq!(app.input_description.before_cursor(), "ab");
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.input_description.before_cursor(), "ab");
    }

    #[test]
    fn ctrl_left_and_right_jump_words_in_the_form() {
        let _guard = env_guard();
        sandbox("keys-word-form");
        let mut app = adding_with("hello world");

        press_ctrl(&mut app, KeyCode::Left);
        assert_eq!(app.input_description.before_cursor(), "hello ");
        press_ctrl(&mut app, KeyCode::Left);
        assert_eq!(app.input_description.before_cursor(), "");
        press_ctrl(&mut app, KeyCode::Right);
        assert_eq!(app.input_description.before_cursor(), "hello ");
    }

    #[test]
    fn ctrl_left_and_right_jump_words_while_editing() {
        let _guard = env_guard();
        sandbox("keys-word-edit");
        seed(vec![entry(0, "hello world")], 1);
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.input_mode, InputMode::EditingEntry);
        press_ctrl(&mut app, KeyCode::Left);
        assert_eq!(app.input_description.before_cursor(), "hello ");
    }

    #[test]
    fn ctrl_left_and_right_jump_words_in_the_search_bar() {
        let _guard = env_guard();
        sandbox("keys-word-search");
        let mut app = App::new().unwrap();

        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.input_mode, InputMode::Searching);
        type_str(&mut app, "hello world");
        assert_eq!(app.search_term.value(), "hello world");

        press_ctrl(&mut app, KeyCode::Left);
        assert_eq!(app.search_term.before_cursor(), "hello ");
        press(&mut app, KeyCode::Left);
        assert_eq!(app.search_term.before_cursor(), "hello");
        press_ctrl(&mut app, KeyCode::Right);
        assert_eq!(app.search_term.before_cursor(), "hello ");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.search_term.value(), "helloworld");
    }

    #[test]
    fn enter_in_adding_mode_submits_a_new_entry() {
        let _guard = env_guard();
        sandbox("keys-submit-add");
        seed(Vec::new(), 0);
        let mut app = adding_with("written by a keypress");

        // Description -> Project -> Tags -> Duration.
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.input_field, InputField::Duration);
        type_str(&mut app, "1h");
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.input_mode, InputMode::Normal);
        let stored = storage::load_data().unwrap();
        assert_eq!(stored.entries.len(), 1);
        assert_eq!(stored.entries[0].description, "written by a keypress");
    }

    #[test]
    fn enter_in_editing_mode_updates_the_selected_entry() {
        let _guard = env_guard();
        sandbox("keys-submit-edit");
        seed(vec![entry(0, "before")], 1);
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        press(&mut app, KeyCode::Char('e'));
        type_str(&mut app, "-after");
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.input_mode, InputMode::Normal);
        let stored = storage::load_data().unwrap();
        // Edited in place, not added alongside.
        assert_eq!(stored.entries.len(), 1);
        assert_eq!(stored.entries[0].description, "before-after");
    }

    #[test]
    fn esc_leaves_each_mode_the_way_it_came() {
        let _guard = env_guard();
        sandbox("keys-esc");
        seed(vec![entry(0, "kept")], 1);
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        press(&mut app, KeyCode::Char('a'));
        type_str(&mut app, "discarded");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.input_description.is_empty());

        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.editing_entry_id, None);

        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "kept");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.search_term.is_empty());

        // Nothing left to clear, so Esc quits.
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit);

        // Unchanged on disk throughout.
        assert_eq!(storage::load_data().unwrap().entries.len(), 1);
    }

    #[test]
    fn help_and_detail_keys_are_bound() {
        let _guard = env_guard();
        sandbox("keys-modals");
        seed(vec![entry(0, "kept")], 1);
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.input_mode, InputMode::Help);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.help_scroll, 1);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.help_scroll, 0);
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.input_mode, InputMode::Normal);

        app.input_mode = InputMode::Detail;
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.input_mode, InputMode::EditingEntry);
        press(&mut app, KeyCode::Esc);

        app.input_mode = InputMode::Detail;
        press(&mut app, KeyCode::Char('q'));
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    /// Every key the Normal-mode map claims, asserted to still land on its
    /// action rather than the arm's `_ => {}`.
    #[test]
    fn every_normal_binding_reaches_its_action() {
        let _guard = env_guard();
        sandbox("keys-normal-map");
        seed(vec![entry(0, "kept")], 1);

        /// A key, the action it should reach, and how to see that it did.
        type Binding = (KeyCode, &'static str, fn(&App) -> bool);

        let checks: Vec<Binding> = vec![
            (KeyCode::Char('1'), "day view", |a| {
                a.view_mode == ViewMode::Day
            }),
            (KeyCode::Char('2'), "week view", |a| {
                a.view_mode == ViewMode::Week
            }),
            (KeyCode::Char('3'), "all view", |a| {
                a.view_mode == ViewMode::All
            }),
            (KeyCode::Char('4'), "overview", |a| {
                a.view_mode == ViewMode::Overview
            }),
            (KeyCode::Char('a'), "add form", |a| {
                a.input_mode == InputMode::AddingEntry
            }),
            (KeyCode::Char('e'), "edit form", |a| {
                a.input_mode == InputMode::EditingEntry
            }),
            (KeyCode::Char('/'), "search", |a| {
                a.input_mode == InputMode::Searching
            }),
            (KeyCode::Char('?'), "help", |a| {
                a.input_mode == InputMode::Help
            }),
            (KeyCode::Char('d'), "delete confirm", |a| {
                a.input_mode == InputMode::Confirm
            }),
            (KeyCode::Enter, "detail", |a| {
                a.input_mode == InputMode::Detail
            }),
            (KeyCode::Char('P'), "projects pane", |a| a.show_projects),
            (KeyCode::Char('T'), "tags pane", |a| a.show_tags),
            (KeyCode::Char('A'), "marks pane", |a| a.show_marks),
            (KeyCode::Char('S'), "summary pane", |a| a.show_summary),
            (KeyCode::Char('o'), "sort order", |a| {
                a.sort_order != SortOrder::NewestFirst
            }),
            (KeyCode::Char('q'), "quit", |a| a.should_quit),
            (KeyCode::Esc, "quit", |a| a.should_quit),
            (KeyCode::Char('h'), "previous period", |a| {
                a.selected_date < Local::now().date_naive()
            }),
            (KeyCode::Char('l'), "next period", |a| {
                a.selected_date > Local::now().date_naive()
            }),
        ];

        for (code, what, effect) in checks {
            let mut app = App::new().unwrap();
            app.table_state.select(Some(0));
            press(&mut app, code);
            assert!(effect(&app), "{code:?} should reach {what}");
        }

        // Tab and BackTab need a pane with something in it to focus.
        let mut with_project = entry(2, "has a project");
        with_project.project = Some("acme".to_string());
        seed(vec![with_project], 3);
        let mut app = App::new().unwrap();
        press(&mut app, KeyCode::Char('P'));
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Table, "Tab should cycle focus");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Table, "BackTab should cycle focus");

        // `t` returns from wherever `h` left the cursor.
        let mut app = App::new().unwrap();
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.selected_date, Local::now().date_naive());

        // `s` stops the running entry through the same key path.
        let mut open = entry(1, "running");
        open.end_time = None;
        seed(vec![open], 2);
        let mut app = App::new().unwrap();
        press(&mut app, KeyCode::Char('s'));
        assert!(storage::load_data().unwrap().entries[0].end_time.is_some());

        // `r` reloads from the store without a keypress of its own being lost.
        let mut app = App::new().unwrap();
        seed(vec![entry(5, "written elsewhere")], 6);
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.data.entries[0].description, "written elsewhere");
    }
}
