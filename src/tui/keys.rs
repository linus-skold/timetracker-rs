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
