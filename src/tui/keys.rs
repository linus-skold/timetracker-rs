//! The whole key map, in one place and reachable without a terminal:
//! `run_tui` only reads events, this decides what they mean.

use super::App;
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

// Collapsing these `if`s into match guards is what clippy suggests, but the
// conditions here call into `app` and change it — a guard that both dispatches
// and acts, and silently falls through to the next arm when it is false, hides
// the fallback these arms exist to spell out.
#[allow(
    clippy::collapsible_match,
    reason = "the conditions have side effects; a match guard would hide them"
)]
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
        // Summary has no rows: j/k must not move the table.
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.pane_next() && !app.summary_is_focused() && !app.scroll_heat(true) {
                app.next();
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if !app.pane_previous() && !app.summary_is_focused() && !app.scroll_heat(false) {
                app.previous();
            }
        }
        // One key, disambiguated by focus: `cycle_pane_value`
        // reporting false *is* the focus check.
        KeyCode::Enter => {
            if !app.cycle_pane_value(true) {
                app.activate_row();
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
        // Capital `M` only, and unconditional: it owns the content pane rather
        // than a focusable surface, so no focus state takes it away.
        KeyCode::Char('M') => app.toggle_heat_view(),
        // Lowercase, and unconditional too: `m` pairs with `M`, so a focus
        // check on one and not the other would break the pair.
        KeyCode::Char('m') => app.toggle_summary_heat(),
        KeyCode::Char('v') => {
            if app.summary_is_focused() {
                app.toggle_summary_split();
            }
        }
        // Only the Summary owns `f`; every other focus leaves it inert.
        KeyCode::Char('f') => {
            if app.summary_is_focused() {
                app.toggle_summary_follows_filters();
            }
        }
        KeyCode::Tab => app.cycle_focus(),
        // crossterm reports Shift-Tab as its own code.
        KeyCode::BackTab => app.cycle_focus_back(),
        KeyCode::Char('d') => app.request_confirm(ConfirmAction::Delete),
        KeyCode::Char('s') => app.stop_active()?,
        KeyCode::Char('r') => app.reload()?,
        KeyCode::Char('a') => app.start_adding(),
        KeyCode::Char('e') => app.start_editing(),
        KeyCode::Char('g') => app.toggle_group_at_cursor(),
        KeyCode::Char('/') => app.start_search(),
        KeyCode::Char('1') => app.set_view_mode(ViewMode::Day),
        KeyCode::Char('2') => app.set_view_mode(ViewMode::Week),
        KeyCode::Char('3') => app.set_view_mode(ViewMode::Month),
        KeyCode::Char('4') => app.set_view_mode(ViewMode::Year),
        KeyCode::Char('5') => app.set_view_mode(ViewMode::All),
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
            // The skill ships inside this binary, so there is nothing to check
            // for first — the install cannot fail on a missing tool.
            KeyCode::Char('y') => {
                app.request_skill_install = true;
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
        // The form holds no list of its own, so the arrows walk the fields.
        KeyCode::Tab | KeyCode::Down => app.next_input_field(),
        KeyCode::BackTab | KeyCode::Up => app.prev_input_field(),
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
    fn down_and_up_walk_the_form_fields() {
        let _guard = env_guard();
        sandbox("keys-arrows");
        let mut app = adding_with("hello");

        press(&mut app, KeyCode::Down);
        assert_eq!(app.input_field, InputField::Project);

        press(&mut app, KeyCode::Up);
        assert_eq!(app.input_field, InputField::Description);
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

    /// An entry `hours_ago`, tagged, so it groups with its siblings.
    fn tagged(id: u64, description: &str, tags: &[&str], hours_ago: i64) -> TimeEntry {
        let start = Local::now() - chrono::Duration::hours(hours_ago);
        TimeEntry {
            id,
            description: description.to_string(),
            project: Some("tt".to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: start,
            end_time: Some(start + chrono::Duration::minutes(30)),
            idle: Vec::new(),
            data: None,
        }
    }

    #[test]
    fn j_and_k_step_over_a_collapsed_group_and_through_its_members() {
        let _guard = env_guard();
        sandbox("keys-group-steps");
        seed(
            vec![
                tagged(0, "round one", &["tt/174"], 4),
                tagged(1, "round two", &["tt/174"], 3),
                tagged(2, "loose", &[], 5),
            ],
            3,
        );
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        // Collapsed: the header and the loose entry, and `j` wraps over two.
        assert_eq!(app.selectable_len(), 2);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.table_state.selected(), Some(1));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.table_state.selected(), Some(0));

        app.expanded_issues.insert("tt/174".to_string());
        assert_eq!(app.selectable_len(), 4);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.selected_entry().map(|e| e.description.clone()),
            Some("round two".to_string()),
            "`j` did not step onto the first member"
        );
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.table_state.selected(), Some(0));
        assert!(app.selected_entry().is_none(), "`k` left the header");
    }

    /// The Summary has no rows, so `j`/`k` must move nothing while it has focus.
    #[test]
    fn j_and_k_are_inert_while_the_summary_has_focus() {
        let _guard = env_guard();
        sandbox("keys-summary-inert");
        // Three rows, so two presses in one direction cannot wrap back to the start.
        seed(
            vec![
                tagged(0, "first", &["impl"], 4),
                tagged(1, "second", &["ops"], 3),
                tagged(2, "third", &["plan"], 2),
            ],
            3,
        );
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        press(&mut app, KeyCode::Char('P'));
        press(&mut app, KeyCode::Char('T'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.pane_cursor(Pane::Tags),
            1,
            "the Tags cursor moved first"
        );

        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.focus, Focus::Summary);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.table_state.selected(), Some(0), "`j` moved the table");
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Up);
        assert_eq!(app.table_state.selected(), Some(0), "`k` moved the table");
        assert_eq!(app.pane_cursor(Pane::Tags), 1);
        assert_eq!(app.pane_cursor(Pane::Projects), 0);

        // The table moves again as soon as focus leaves the surface.
        app.focus = Focus::Table;
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.table_state.selected(), Some(1));
    }

    /// `v` belongs to the Summary, so every other focus state must ignore it.
    #[test]
    fn v_splits_the_summary_only_while_it_has_focus() {
        let _guard = env_guard();
        sandbox("keys-summary-split");
        let mut with_project = entry(0, "has a project");
        with_project.project = Some("acme".to_string());
        seed(vec![with_project], 1);
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        // Hidden: the surface is not there to split.
        assert!(!app.show_summary);
        press(&mut app, KeyCode::Char('v'));
        assert!(!app.summary_split, "`v` split a hidden surface");

        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.focus, Focus::Summary);
        press(&mut app, KeyCode::Char('v'));
        assert!(app.summary_split, "`v` did not reach the split");
        press(&mut app, KeyCode::Char('v'));
        assert!(!app.summary_split, "`v` did not flip back");

        // Visible, but focus rests elsewhere.
        app.summary_split = true;
        app.focus = Focus::Table;
        press(&mut app, KeyCode::Char('v'));
        assert!(app.summary_split, "`v` fired with the table focused");

        press(&mut app, KeyCode::Char('P'));
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        press(&mut app, KeyCode::Char('v'));
        assert!(app.summary_split, "`v` fired with a pane focused");

        // Hiding the surface takes the key away again, and leaves the bool alone.
        app.focus = Focus::Summary;
        press(&mut app, KeyCode::Char('S'));
        assert!(!app.show_summary);
        press(&mut app, KeyCode::Char('v'));
        assert!(app.summary_split, "`v` fired on a hidden surface");
    }

    /// `f` belongs to the Summary, so every other focus state must ignore it.
    #[test]
    fn f_follows_the_filters_only_while_the_summary_has_focus() {
        let _guard = env_guard();
        sandbox("keys-summary-follow");
        let mut with_project = entry(0, "has a project");
        with_project.project = Some("acme".to_string());
        seed(vec![with_project], 1);
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));

        // Hidden: the surface is not there to follow anything.
        assert!(!app.show_summary);
        press(&mut app, KeyCode::Char('f'));
        assert!(
            !app.summary_follows_filters,
            "`f` fired on a hidden surface"
        );

        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.focus, Focus::Summary);
        press(&mut app, KeyCode::Char('f'));
        assert!(app.summary_follows_filters, "`f` did not reach the mode");
        press(&mut app, KeyCode::Char('f'));
        assert!(!app.summary_follows_filters, "`f` did not flip back");

        // Visible, but focus rests elsewhere.
        app.summary_follows_filters = true;
        app.focus = Focus::Table;
        press(&mut app, KeyCode::Char('f'));
        assert!(
            app.summary_follows_filters,
            "`f` fired with the table focused"
        );

        press(&mut app, KeyCode::Char('P'));
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        press(&mut app, KeyCode::Char('f'));
        assert!(app.summary_follows_filters, "`f` fired with a pane focused");

        // Hiding the surface takes the key away again, and leaves the bool alone.
        app.focus = Focus::Summary;
        press(&mut app, KeyCode::Char('S'));
        assert!(!app.show_summary);
        press(&mut app, KeyCode::Char('f'));
        assert!(app.summary_follows_filters, "`f` fired on a hidden surface");
    }

    /// `M` owns the content pane, so no focus state may take it away, and the
    /// lowercase `h` beside it must still step the period back.
    #[test]
    fn shift_m_flips_the_heat_view_from_any_focus() {
        let _guard = env_guard();
        sandbox("keys-heat-view");
        let mut with_project = entry(0, "has a project");
        with_project.project = Some("acme".to_string());
        seed(vec![with_project], 1);
        let mut app = App::new().unwrap();

        press(&mut app, KeyCode::Char('M'));
        assert!(app.heat_view, "`M` did not reach the flag");
        press(&mut app, KeyCode::Char('M'));
        assert!(!app.heat_view, "`M` did not flip back");

        for focus in [Focus::Summary, Focus::Pane(Pane::Projects)] {
            app.show_summary = true;
            app.focus = focus;
            press(&mut app, KeyCode::Char('M'));
            assert!(app.heat_view, "`M` was gated on {focus:?}");
            press(&mut app, KeyCode::Char('M'));
        }

        let today = app.selected_date;
        press(&mut app, KeyCode::Char('h'));
        assert!(app.selected_date < today, "`h` stopped stepping back");
    }

    /// `m` pairs with `M`, so it works wherever `M` does — focus and all.
    #[test]
    fn m_flips_the_summary_heat_from_any_focus() {
        let _guard = env_guard();
        sandbox("keys-summary-heat");
        let mut with_project = entry(0, "has a project");
        with_project.project = Some("acme".to_string());
        seed(vec![with_project], 1);
        let mut app = App::new().unwrap();

        assert!(!app.summary_heat, "the strips are opt-in");
        press(&mut app, KeyCode::Char('m'));
        assert!(app.summary_heat, "`m` was gated on the hidden Summary");
        press(&mut app, KeyCode::Char('m'));
        assert!(!app.summary_heat, "`m` did not flip back");

        for focus in [Focus::Summary, Focus::Pane(Pane::Projects)] {
            app.show_summary = true;
            app.focus = focus;
            press(&mut app, KeyCode::Char('m'));
            assert!(app.summary_heat, "`m` was gated on {focus:?}");
            press(&mut app, KeyCode::Char('m'));
        }
        assert!(!app.heat_view, "`m` reached the content pane's own flag");
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
            (KeyCode::Char('3'), "month view", |a| {
                a.view_mode == ViewMode::Month
            }),
            (KeyCode::Char('4'), "year view", |a| {
                a.view_mode == ViewMode::Year
            }),
            (KeyCode::Char('5'), "all view", |a| {
                a.view_mode == ViewMode::All
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
            (KeyCode::Char('M'), "heat view", |a| a.heat_view),
            (KeyCode::Char('m'), "summary heat", |a| a.summary_heat),
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

        // `v` needs the Summary focused, so `S` opens it first.
        let mut app = App::new().unwrap();
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('v'));
        assert!(app.summary_split, "`v` should reach the summary split");
        press(&mut app, KeyCode::Char('f'));
        assert!(
            app.summary_follows_filters,
            "`f` should reach the follow mode"
        );

        // `t` returns from wherever `h` left the cursor.
        let mut app = App::new().unwrap();
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.selected_date, Local::now().date_naive());

        // `g` needs a group under the cursor to have anything to toggle.
        seed(
            vec![
                tagged(0, "round one", &["tt/174"], 4),
                tagged(1, "round two", &["tt/174"], 3),
            ],
            2,
        );
        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));
        press(&mut app, KeyCode::Char('g'));
        assert!(
            app.expanded_issues.contains("tt/174"),
            "`g` should reach the group toggle"
        );
        press(&mut app, KeyCode::Char('g'));
        assert!(app.expanded_issues.is_empty(), "`g` should collapse again");

        // `Enter` reaches the same toggle while the cursor is on a header.
        press(&mut app, KeyCode::Enter);
        assert!(
            app.expanded_issues.contains("tt/174"),
            "Enter should toggle the group under the cursor"
        );
        assert_eq!(app.input_mode, InputMode::Normal, "a header has no detail");

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

    /// A Day heat with more projects than rows scrolls them; the table
    /// selection stays where it was.
    #[test]
    fn j_and_k_scroll_the_day_heat_rows() {
        let _guard = env_guard();
        sandbox("keys-heat-scroll");
        let today = Local::now().date_naive();
        seed(
            (0..4)
                .map(|id| {
                    let start = today
                        .and_hms_opt(9 + id as u32, 0, 0)
                        .unwrap()
                        .and_local_timezone(Local)
                        .unwrap();
                    TimeEntry {
                        id,
                        description: "seed".to_string(),
                        project: Some(format!("p{id}")),
                        tags: Vec::new(),
                        start_time: start,
                        end_time: Some(start + chrono::Duration::minutes(30)),
                        idle: Vec::new(),
                        data: None,
                    }
                })
                .collect(),
            4,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.heat_view = true;
        app.heat_scroll_max = 3;
        let selected = app.table_state.selected();

        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.heat_scroll, 0, "the top is a floor");

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.heat_scroll, 1);
        assert_eq!(
            app.table_state.selected(),
            selected,
            "scrolling moved the table selection"
        );

        for _ in 0..10 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.heat_scroll, 3, "the last row is the bottom");

        app.set_view_mode(ViewMode::Week);
        assert_eq!(app.heat_scroll, 0, "a view change kept the offset");
    }

    /// Only the views that can overflow claim the keys.
    #[test]
    fn j_keeps_moving_the_table_in_a_week_heat() {
        let _guard = env_guard();
        sandbox("keys-heat-no-scroll");
        seed(vec![entry(0, "a"), entry(1, "b")], 2);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Week;
        app.heat_view = true;
        app.table_state.select(Some(0));

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.heat_scroll, 0, "the week heat cannot scroll");
        assert_eq!(
            app.table_state.selected(),
            Some(1),
            "the table stopped moving under a heat that cannot scroll"
        );
    }

    /// The list keeps `j` for the table, heat mode or not.
    #[test]
    fn a_day_list_leaves_the_scroll_alone() {
        let _guard = env_guard();
        sandbox("keys-heat-list");
        seed(vec![entry(0, "a"), entry(1, "b")], 2);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::All;
        app.table_state.select(Some(0));

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.heat_scroll, 0);
        assert_eq!(app.table_state.selected(), Some(1));
    }

    /// The offset stops where the box does.
    #[test]
    fn the_day_heat_scroll_stops_at_the_limit_the_draw_set() {
        let _guard = env_guard();
        sandbox("keys-heat-scroll-room");
        let today = Local::now().date_naive();
        seed(
            (0..8)
                .map(|id| {
                    let start = today
                        .and_hms_opt(8 + id as u32, 0, 0)
                        .unwrap()
                        .and_local_timezone(Local)
                        .unwrap();
                    TimeEntry {
                        id,
                        description: "seed".to_string(),
                        project: Some(format!("p{id}")),
                        tags: Vec::new(),
                        start_time: start,
                        end_time: Some(start + chrono::Duration::minutes(30)),
                        idle: Vec::new(),
                        data: None,
                    }
                })
                .collect(),
            8,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.heat_view = true;
        // Eight project rows over the seven the last draw had room for.
        app.heat_scroll_max = 1;

        for _ in 0..6 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.heat_scroll, 1, "the offset outran the box");

        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.heat_scroll, 0, "one press did not reach the top");
    }

    /// A filter leaves other projects on the grid, so the offset goes home.
    #[test]
    fn a_pane_filter_resets_the_heat_scroll() {
        let _guard = env_guard();
        sandbox("keys-heat-filter-reset");
        seed(vec![entry(0, "a"), entry(1, "b")], 2);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.heat_view = true;
        app.heat_scroll_max = 3;
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.heat_scroll, 1);

        app.clear_filters();
        assert_eq!(app.heat_scroll, 0, "a filter change kept the offset");

        press(&mut app, KeyCode::Char('j'));
        app.handle_search_char('a');
        assert_eq!(app.heat_scroll, 0, "a search kept the offset");
    }
}
