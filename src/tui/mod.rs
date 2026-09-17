use crate::activity::Session;
use crate::audit::Unaccounted;
use crate::marks::Mark;
use crate::storage::{PathStamp, load_data};
use crate::tracker::TimeData;
use anyhow::Result;
use cache::{FilterKey, RowKey, ScopeKey};
use chrono::{Local, NaiveDate};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::TableState};
use std::cell::RefCell;
use std::io::{self, Stdout};
use text_input::TextInput;

mod cache;
mod entry_form;
mod keys;
mod marks_surface;
mod navigation;
mod onboarding;
mod panes;
mod render;
mod rows;
mod search;
mod summary;
mod text_input;
pub mod theme;
pub mod types;

pub use types::{
    ConfirmAction, Focus, InputField, InputMode, OnboardingStep, Pane, PendingConfirm, SortOrder,
    ViewMode,
};

/// The view every session opens on, and what `heat_view` defaults against.
const START_VIEW: ViewMode = ViewMode::Day;

pub(crate) struct App {
    /// The store snapshot. **Never assign this directly** — go through
    /// [`App::set_data`], which bumps `data_revision`.
    pub(crate) data: TimeData,
    /// Bumped on every replacement of `data`, so the cache keys can stand in for
    /// its contents without comparing them.
    pub(crate) data_revision: u64,
    /// `filtered_entries` as indices into `data.entries` — indices, not
    /// references, so the cache does not borrow `data` — with the key they hold
    /// for. `RefCell` because nearly every reader is a `&self` method.
    filtered_cache: RefCell<Option<(FilterKey, Vec<usize>)>>,
    /// `pane_values` for `[Projects, Tags]`, with the key they hold for.
    pane_cache: RefCell<Option<(ScopeKey, panes::PaneValues)>>,
    /// `rows` — the grouped row model — with the key it holds for.
    rows_cache: RefCell<Option<(RowKey, Vec<rows::VisibleRow>)>>,
    /// The item tags whose group header is expanded. Keyed on the tag alone, so
    /// in Week view an issue expands in every day it appears in.
    pub(crate) expanded_issues: std::collections::HashSet<String>,
    pub(crate) table_state: TableState,
    pub(crate) should_quit: bool,
    pub(crate) view_mode: ViewMode,
    pub(crate) selected_date: NaiveDate,
    pub(crate) input_mode: InputMode,
    /// Top row of the help popup; clamped by the renderer, so it may run ahead.
    pub(crate) help_scroll: usize,
    pub(crate) input_field: InputField,
    pub(crate) input_description: TextInput,
    pub(crate) input_project: TextInput,
    pub(crate) input_tags: TextInput,
    pub(crate) input_start_time: TextInput,
    pub(crate) input_end_time: TextInput,
    pub(crate) input_duration: TextInput,
    /// The Data field's raw text: compact JSON, or empty for no data.
    pub(crate) input_data: TextInput,
    /// Why the last save was refused, shown in the form's help row and cleared
    /// by the next keystroke. Only invalid JSON raises one today.
    pub(crate) form_error: Option<String>,
    pub(crate) search_term: TextInput,
    /// Each pane's tri-state filter. OR within a pane's includes, AND across the two.
    pub(crate) project_filter: panes::PaneFilter,
    pub(crate) tag_filter: panes::PaneFilter,
    pub(crate) editing_entry_id: Option<u64>,
    /// What `InputMode::Confirm` is asking about: the action, entry and origin mode.
    pub(crate) pending_confirm: Option<PendingConfirm>,
    pub(crate) sort_order: SortOrder,
    /// Fingerprint of the store as of the last load, so a tick can skip the read.
    pub(crate) store_stamp: Option<PathStamp>,
    /// The open agent phase marks, newest first, so a frame never reads the directory.
    pub(crate) marks: Vec<Mark>,
    /// Fingerprint of the *mark directory*, so a tick need not list it.
    pub(crate) marks_stamp: Option<PathStamp>,
    /// The hook-only activity ledger's sessions, cached the same way `marks` is.
    pub(crate) activity_sessions: Vec<Session>,
    /// Fingerprint of the *activity directory*, so a tick need not list it.
    pub(crate) activity_stamp: Option<PathStamp>,
    /// Each mark in `marks` paired with its liveness, so a frame never reads the
    /// beats directory; empty while the Agents surface is hidden.
    pub(crate) leases: Vec<crate::marks::Lease>,
    /// When liveness was last read; `None` re-reads on the next call.
    pub(crate) liveness_at: Option<std::time::Instant>,
    /// The thresholds as of that read, so a frame judges staleness without the config.
    pub(crate) liveness_thresholds: crate::marks::Thresholds,
    /// Activity windows with no covering mark or logged entry — recomputed
    /// each tick from `marks`, `activity_sessions` and `data`, never read
    /// from disk itself.
    pub(crate) unaccounted: Vec<Unaccounted>,
    /// Whether each collapsible surface is open. All default to off, so their rows
    /// are absent from the layout plan.
    pub(crate) show_projects: bool,
    pub(crate) show_tags: bool,
    pub(crate) show_marks: bool,
    pub(crate) show_summary: bool,
    /// Whether the Summary splits each row into human and agent time.
    pub(crate) summary_split: bool,
    /// Whether the Summary folds `filtered_entries()` instead of the scope.
    pub(crate) summary_follows_filters: bool,
    /// Whether the content area draws a heatmap instead of the entry list.
    /// One flag for every view, so switching view keeps the representation.
    pub(crate) heat_view: bool,
    /// Whether the Summary's project rows carry their per-project heat strips.
    pub(crate) summary_heat: bool,
    /// Rows or bands the heat grid is scrolled past. Session state: a view
    /// change, a period step and `M` all reset it, and it is never persisted.
    pub(crate) heat_scroll: usize,
    /// How far `heat_scroll` may go, as the last frame worked it out. The
    /// renderer owns the clamp; `j`/`k` only read the limit back.
    pub(crate) heat_scroll_max: usize,
    /// What `Tab` has given focus to, and where each pane's cursor rests.
    pub(crate) focus: Focus,
    pub(crate) project_cursor: usize,
    pub(crate) tag_cursor: usize,
    /// Which screen of `InputMode::Onboarding` is showing.
    pub(crate) onboarding_step: OnboardingStep,
    /// The onboarding popup's checklist cursor and checked state, in
    /// `LayoutSurface::ALL` order. Unused once `InputMode::Onboarding` is left.
    pub(crate) onboarding_cursor: usize,
    pub(crate) onboarding_checked: [bool; 4],
    /// One-shot: the run loop is what can suspend the terminal to run a
    /// child process, so onboarding just requests it here.
    pub(crate) request_skill_install: bool,
    /// A newer version than this build, if `main` found one before the TUI
    /// took the terminal over. Shown as a banner, never blocking.
    pub(crate) update_notice: Option<String>,
}

impl App {
    /// Used directly only by tests — never onboards. A real run goes through
    /// `for_interactive_run` instead.
    #[cfg_attr(not(test), allow(dead_code))]
    fn new() -> Result<Self> {
        Self::from_config(crate::config::load())
    }

    /// The blessed constructor for a real run: any future production entry
    /// point should build its `App` through here, so onboarding isn't
    /// something each call site has to remember to bolt on.
    fn for_interactive_run(update_notice: Option<String>) -> Result<Self> {
        let config = crate::config::load();
        let mut app = Self::from_config(config)?;
        if crate::config::should_onboard(&config.general) {
            app.input_mode = InputMode::Onboarding;
        }
        app.update_notice = update_notice;
        Ok(app)
    }

    /// The env-free half of `new`, so callers can load the config once and
    /// reuse it (e.g. for `should_onboard`) instead of reading it twice.
    fn from_config(config: &crate::config::Config) -> Result<Self> {
        // Stamp before loading — see `App::reload`.
        let store_stamp = crate::storage::store_stamp();
        let layout = &config.layout;
        let mut app = Self {
            data: load_data()?,
            data_revision: 0,
            filtered_cache: RefCell::new(None),
            pane_cache: RefCell::new(None),
            rows_cache: RefCell::new(None),
            expanded_issues: std::collections::HashSet::new(),
            store_stamp,
            marks: Vec::new(),
            marks_stamp: None,
            activity_sessions: Vec::new(),
            activity_stamp: None,
            leases: Vec::new(),
            liveness_at: None,
            liveness_thresholds: crate::audit::thresholds(),
            unaccounted: Vec::new(),
            table_state: TableState::default().with_selected(Some(0)),
            should_quit: false,
            view_mode: START_VIEW,
            selected_date: Local::now().date_naive(),
            input_mode: InputMode::Normal,
            help_scroll: 0,
            input_field: InputField::Description,
            input_description: TextInput::default(),
            input_project: TextInput::default(),
            input_tags: TextInput::default(),
            input_start_time: TextInput::default(),
            input_end_time: TextInput::default(),
            input_duration: TextInput::default(),
            input_data: TextInput::default(),
            form_error: None,
            search_term: TextInput::default(),
            project_filter: panes::PaneFilter::default(),
            tag_filter: panes::PaneFilter::default(),
            editing_entry_id: None,
            pending_confirm: None,
            sort_order: SortOrder::NewestFirst,
            show_projects: layout.show_projects.unwrap_or(false),
            show_tags: layout.show_tags.unwrap_or(false),
            show_marks: layout.show_agents.unwrap_or(false),
            show_summary: layout.show_summary.unwrap_or(false),
            summary_split: layout.summary_split.unwrap_or(false),
            summary_follows_filters: layout.summary_follows_filters.unwrap_or(false),
            // Year is the only view whose own representation is the heatmap.
            heat_view: layout.heat_view.unwrap_or(START_VIEW == ViewMode::Year),
            summary_heat: layout.summary_heat.unwrap_or(false),
            heat_scroll: 0,
            heat_scroll_max: 0,
            focus: Focus::Table,
            project_cursor: 0,
            tag_cursor: 0,
            onboarding_step: OnboardingStep::Layout,
            onboarding_cursor: 0,
            // Seeded from what's already saved, so re-onboarding shows real
            // settings; an unsaved field falls back to a recommended default.
            onboarding_checked: [
                layout.show_projects.unwrap_or(true),
                layout.show_agents.unwrap_or(false),
                layout.show_summary.unwrap_or(false),
                layout.show_tags.unwrap_or(true),
            ],
            request_skill_install: false,
            update_notice: None,
        };
        // The first tick is 250 ms away, so read now for a current first frame.
        app.sync_from_marks();
        app.sync_from_activity();
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
        self.set_data(fresh);
        // Our own write moved the file on; stamp it so the next tick skips it.
        self.store_stamp = crate::storage::store_stamp();
        Ok(result)
    }

    /// How the TUI stands right now — open surfaces and representations — in
    /// config shape. **The single writer:** a field left out of this literal is
    /// erased on the next [`persist_layout`](Self::persist_layout).
    pub(crate) fn layout_config(&self) -> crate::config::LayoutConfig {
        crate::config::LayoutConfig {
            show_projects: Some(self.show_projects),
            show_agents: Some(self.show_marks),
            show_summary: Some(self.show_summary),
            show_tags: Some(self.show_tags),
            summary_split: Some(self.summary_split),
            summary_follows_filters: Some(self.summary_follows_filters),
            heat_view: Some(self.heat_view),
            summary_heat: Some(self.summary_heat),
        }
    }

    /// Write the open surfaces back to the config file, so the next run starts
    /// the way this one was left. Best-effort: a failed write must not take the
    /// session down, and the terminal is in raw mode so there is nowhere to
    /// report it — the toggle still holds for the rest of the session.
    pub(crate) fn persist_layout(&self) {
        let _ = crate::config::save_layout(&self.layout_config());
    }

    /// The one place `data` is replaced, by `mutate_store` and by `reload`.
    /// Bumping `data_revision` here is what makes the derived-view caches notice
    /// the new store, so a bare `self.data = …` elsewhere would go unseen.
    pub(crate) fn set_data(&mut self, data: TimeData) {
        self.data = data;
        self.data_revision = self.data_revision.wrapping_add(1);
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

/// Hands the real terminal to `run` (an interactive child command), then
/// restores our screen and forces a redraw over its leftover output.
fn with_suspended_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    run: impl FnOnce() -> Result<()>,
) -> Result<()> {
    restore_terminal(terminal)?;
    let result = run();
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    terminal.clear()?;
    result
}

pub fn run_tui(update_notice: Option<String>) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut app = App::for_interactive_run(update_notice)?;

    loop {
        terminal.draw(|f| render::ui(f, &mut app))?;

        if event::poll(std::time::Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            keys::handle_key(&mut app, key)?;
        }

        if app.request_skill_install {
            app.request_skill_install = false;
            with_suspended_terminal(&mut terminal, || {
                // Best-effort: a failed install must not take onboarding down.
                if let Err(error) = crate::skill::install(crate::skill::Request {
                    dir: None,
                    agents: Vec::new(),
                    all: false,
                    hooks: true,
                }) {
                    println!("Couldn't install the skill: {error}");
                }
                println!("Press Enter to return to tt...");
                let mut discard = String::new();
                io::stdin().read_line(&mut discard).ok();
                Ok(())
            })?;
            app.onboarding_finish()?;
        }

        // The poll above is the loop's clock, key or timeout alike.
        app.sync_from_store()?;
        app.sync_from_marks();
        app.sync_from_activity();

        if app.should_quit {
            break;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::panes::Polarity;
    use super::*;
    use crate::storage;
    /// Serialises the tests that repoint `HOME` and `TT_MARK_DIR`; env is
    /// process-wide, and `marks`' own env test shares this lock.
    use crate::storage::env_guard;
    use crate::storage::env_sandbox as sandbox;
    use crate::tracker::TimeEntry;
    use chrono::Datelike;
    use crossterm::event::KeyCode;
    use std::path::PathBuf;

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

    fn beat_mark(dir: &std::path::Path, key: &str) {
        let beats = dir.join("beats");
        std::fs::create_dir_all(&beats).unwrap();
        std::fs::write(beats.join(key), format!("{}\n", Local::now().timestamp())).unwrap();
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
            data: None,
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
        agent_write_at(description, Local::now())
    }

    /// [`agent_write`] with a fixed start, for a test whose sort order must not
    /// depend on the wall clock.
    fn agent_write_at(description: &str, start: chrono::DateTime<Local>) -> u64 {
        storage::with_data(|data| {
            Ok(data
                .add_entry(
                    description.to_string(),
                    Some("probe".to_string()),
                    vec!["probe".to_string()],
                    start,
                    Some(start),
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

    /// Three rounds on one issue, plus one hand-written entry: two selectable
    /// rows, the group header first.
    fn seed_grouped() -> App {
        let today = Local::now().date_naive();
        seed(
            vec![
                dated(0, "round one", "tt", &["tt/174", "impl"], today),
                dated(1, "round two", "tt", &["tt/174", "impl"], today),
                dated(2, "round three", "tt", &["tt/174", "impl"], today),
                dated(3, "hand written", "tt", &[], today),
            ],
            4,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.table_state.select(Some(0));
        app
    }

    #[test]
    fn a_group_header_is_no_entry_so_e_d_and_t_do_nothing_on_it() {
        let _guard = env_guard();
        sandbox("group-header-inert");
        let mut app = seed_grouped();

        assert!(app.selected_entry().is_none(), "a header is not an entry");
        app.start_editing();
        assert_eq!(app.input_mode, InputMode::Normal, "`e` opened the form");
        press_d_then(&mut app, KeyCode::Char('y'));
        assert_eq!(app.input_mode, InputMode::Normal, "`d` raised a prompt");
        press_t_then(&mut app, KeyCode::Char('y'));
        assert_eq!(app.input_mode, InputMode::Normal, "`t` raised a prompt");
        assert_eq!(on_disk().entries.len(), 4, "the store was touched");
    }

    /// `Enter` is the second way in and out of a group, so the header need not
    /// be learnt as the one row where `Enter` does nothing.
    #[test]
    fn enter_toggles_a_group_header_and_still_opens_an_entry() {
        let _guard = env_guard();
        sandbox("group-enter-toggle");
        let mut app = seed_grouped();

        // This is exactly what the Normal-mode Enter arm does.
        let enter = |app: &mut App| {
            if !app.cycle_pane_value(true) {
                app.activate_row();
            }
        };

        enter(&mut app);
        assert_eq!(app.input_mode, InputMode::Normal, "a header has no detail");
        assert!(
            app.expanded_issues.contains("tt/174"),
            "Enter did not expand the group"
        );
        assert_eq!(app.table_state.selected(), Some(0), "the cursor moved");

        enter(&mut app);
        assert!(
            app.expanded_issues.is_empty(),
            "Enter did not collapse the group"
        );

        app.select_by_id(3);
        enter(&mut app);
        assert_eq!(
            app.input_mode,
            InputMode::Detail,
            "Enter stopped opening an entry"
        );
    }

    #[test]
    fn select_by_id_lands_on_the_header_of_the_group_holding_the_id() {
        let _guard = env_guard();
        sandbox("group-select-by-id");
        let mut app = seed_grouped();

        app.select_by_id(2);
        assert_eq!(app.table_state.selected(), Some(0), "not on the header");
        app.select_by_id(3);
        assert_eq!(app.table_state.selected(), Some(1), "not on the entry");

        // Expanded, the member has a row of its own: header, then the three.
        app.expanded_issues.insert("tt/174".to_string());
        app.select_by_id(2);
        assert_eq!(app.table_state.selected(), Some(3));
    }

    #[test]
    fn an_outside_write_keeps_the_cursor_on_the_group_header() {
        let _guard = env_guard();
        sandbox("group-sync-anchor");
        let mut app = seed_grouped();
        // The fixtures start at 09:00; a later probe sorts above the group and
        // shifts it down, whatever the clock says.
        let later = app
            .selected_date
            .and_hms_opt(17, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        agent_write_at("probe", later);

        app.sync_from_store().unwrap();

        assert_eq!(app.table_state.selected(), Some(1));
        assert!(matches!(
            app.selected_row(),
            Some(rows::VisibleRow::GroupHeader(_))
        ));

        // Expanded, the header's members each have a row of their own, and the
        // cursor must still land on the header rather than on the first member.
        app.expanded_issues.insert("tt/174".to_string());
        agent_write_at("second probe", later + chrono::Duration::minutes(1));
        app.sync_from_store().unwrap();

        assert_eq!(app.table_state.selected(), Some(2));
        assert!(
            matches!(app.selected_row(), Some(rows::VisibleRow::GroupHeader(_))),
            "the anchor slid onto a member"
        );
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
        app.input_description.set_from("from the tui");
        app.input_duration.set_from("15m");
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
            app.input_description.set_from("half typed");
            app.sync_from_store().unwrap();
            assert_eq!(descriptions(&app.data), vec!["first"]);
            assert_eq!(app.input_description.value(), "half typed");
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
        app.input_description.set_from("after");
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
        app.input_description.set_from("with a project");
        app.input_project.set_from("  acme  ");
        app.input_duration.set_from("15m");
        app.submit_entry().unwrap();

        app.start_adding();
        app.input_description.set_from("without one");
        app.input_project.set_from("   ");
        app.input_duration.set_from("15m");
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
            data: None,
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

        app.tag_filter.cycle("plan", true);
        app.search_term.set_from("nothing matches this");
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

        let mut app = app;
        app.toggle_marks();
        app.sync_from_activity();
        let shown: Vec<String> = app
            .visible_leases()
            .iter()
            .map(|lease| lease.mark.label())
            .collect();
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

    /// The sandbox's activity directory, created on demand.
    fn activity_sandbox() -> PathBuf {
        let dir = crate::activity::activity_dir().expect("an activity dir");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a session file the way a hook would, backdated `hours_ago`, still
    /// open (no `end=` line). Never shells out.
    fn write_session(dir: &std::path::Path, session_id: &str, project: &str, hours_ago: i64) {
        let start = (Local::now() - chrono::Duration::hours(hours_ago)).timestamp();
        std::fs::write(
            dir.join(session_id),
            format!("start={start}\nproject={project}\n"),
        )
        .unwrap();
    }

    #[test]
    fn an_unaccounted_session_adds_a_header_and_row_to_the_surface_height() {
        let _guard = env_guard();
        sandbox("unaccounted-height");
        seed(vec![entry(0, "first")], 1);
        let mut app = seed_marks(&[]);
        app.toggle_marks();
        assert_eq!(
            app.marks_surface_height(),
            3,
            "empty: just the marks section"
        );

        let dir = activity_sandbox();
        // Well past the default 120-minute floor, and no mark or entry covers it.
        write_session(&dir, "sess-1", "smoke", 3);
        app.activity_stamp = None; // force a re-read; the tick would do this
        app.sync_from_activity();

        assert_eq!(app.unaccounted.len(), 1);
        assert_eq!(
            app.marks_surface_height(),
            5,
            "+1 header, +1 row for the one unaccounted window"
        );
    }

    #[test]
    fn a_session_under_the_floor_is_never_flagged_in_the_tui_either() {
        let _guard = env_guard();
        sandbox("unaccounted-floor");
        seed(vec![entry(0, "first")], 1);
        let mut app = seed_marks(&[]);

        let dir = activity_sandbox();
        write_session(&dir, "sess-1", "smoke", 1); // under the 120-minute floor
        app.activity_stamp = None;
        app.sync_from_activity();

        assert!(app.unaccounted.is_empty());
        app.toggle_marks();
        assert_eq!(
            app.marks_surface_height(),
            3,
            "no unaccounted section when nothing is flagged"
        );
    }

    #[test]
    fn a_covering_mark_clears_the_unaccounted_flag_in_the_tui() {
        let _guard = env_guard();
        sandbox("unaccounted-covered");
        seed(vec![entry(0, "first")], 1);
        let mark_dir = mark_sandbox();
        begin_mark(&mark_dir, "smoke.-.impl", 4 * 60); // started before the window
        beat_mark(&mark_dir, "smoke.-.impl"); // and still alive, so its lease holds

        let activity_dir = activity_sandbox();
        write_session(&activity_dir, "sess-1", "smoke", 3);

        let mut app = App::new().unwrap();
        app.activity_stamp = None;
        app.sync_from_activity();

        assert!(
            app.unaccounted.is_empty(),
            "an overlapping open mark for the same project must cover it"
        );
    }

    #[test]
    fn the_surface_caps_visible_unaccounted_windows_and_counts_the_rest() {
        let _guard = env_guard();
        sandbox("unaccounted-cap");
        seed(vec![entry(0, "first")], 1);
        let mut app = seed_marks(&[]);
        app.toggle_marks();

        let dir = activity_sandbox();
        for (n, project) in [(1, "a"), (2, "b"), (3, "c"), (4, "d")] {
            write_session(&dir, &format!("sess-{n}"), project, 3);
        }
        app.activity_stamp = None;
        app.sync_from_activity();

        assert_eq!(app.unaccounted.len(), 4);
        assert_eq!(app.visible_unaccounted().len(), 3, "capped at three shown");
        assert_eq!(app.unaccounted_count().as_deref(), Some("3/4"));
    }

    #[test]
    fn a_hidden_agents_surface_reads_no_liveness_and_reconciles_nothing() {
        let _guard = env_guard();
        sandbox("liveness-hidden");
        seed(vec![entry(0, "first")], 1);
        let mark_dir = mark_sandbox();
        begin_mark(&mark_dir, "smoke.-.impl", 4 * 60);
        let dir = activity_sandbox();
        write_session(&dir, "sess-1", "smoke", 3);

        let mut app = App::new().unwrap();
        assert!(!app.show_marks, "hidden by default");
        app.activity_stamp = None;
        app.sync_from_activity();

        assert!(app.leases.is_empty(), "no beats read while hidden");
        assert!(app.unaccounted.is_empty(), "and nothing reconciled");
        assert!(app.liveness_at.is_none(), "the interval never started");
    }

    /// Toggling a surface writes it back, so the next run opens the same way.
    #[test]
    fn toggling_a_surface_persists_the_layout() {
        let _guard = env_guard();
        sandbox("layout-persist");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        assert!(!app.show_tags && !app.show_marks, "hidden by default");
        app.toggle_pane(Pane::Tags);
        app.toggle_marks();
        app.toggle_summary();

        let saved = saved_config();
        let layout = &saved["layout"];
        assert_eq!(layout["show_tags"].as_bool(), Some(true));
        assert_eq!(layout["show_agents"].as_bool(), Some(true));
        assert_eq!(layout["show_summary"].as_bool(), Some(true));
        assert_eq!(layout["show_projects"].as_bool(), Some(false));
        // A toggle is not an onboarding answer.
        assert!(saved.get("general").is_none(), "onboarding left untouched");
    }

    /// Reads the config file the TUI wrote, off disk rather than through
    /// `config::load`, whose resolved value is cached for the process.
    fn saved_config() -> toml::Value {
        let path = std::env::var("TT_CONFIG_FILE").unwrap();
        let text = std::fs::read_to_string(path).expect("a written config file");
        toml::from_str(&text).expect("valid TOML")
    }

    fn saved_layout() -> toml::Value {
        saved_config()["layout"].clone()
    }

    #[test]
    fn toggling_a_summary_mode_persists_it() {
        let _guard = env_guard();
        sandbox("summary-mode-persist");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        app.toggle_summary_split();
        app.toggle_summary_follows_filters();

        let layout = saved_layout();
        assert_eq!(layout["summary_split"].as_bool(), Some(true));
        assert_eq!(layout["summary_follows_filters"].as_bool(), Some(true));
    }

    #[test]
    fn answering_onboarding_keeps_both_summary_modes() {
        let _guard = env_guard();
        sandbox("summary-mode-onboarding-finish");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        app.summary_split = true;
        app.summary_follows_filters = true;
        app.onboarding_finish().unwrap();

        let layout = saved_layout();
        assert_eq!(layout["summary_split"].as_bool(), Some(true));
        assert_eq!(layout["summary_follows_filters"].as_bool(), Some(true));
    }

    #[test]
    fn skipping_onboarding_keeps_both_summary_modes() {
        let _guard = env_guard();
        sandbox("summary-mode-onboarding-skip");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        app.summary_split = true;
        app.summary_follows_filters = true;
        app.onboarding_skip().unwrap();

        let layout = saved_layout();
        assert_eq!(layout["summary_split"].as_bool(), Some(true));
        assert_eq!(layout["summary_follows_filters"].as_bool(), Some(true));
    }

    #[test]
    fn the_layout_keys_seed_each_summary_mode() {
        let _guard = env_guard();
        let dir = sandbox("summary-mode-seed");
        seed(vec![entry(0, "first")], 1);
        std::fs::write(
            dir.join("config.toml"),
            "[layout]\nsummary_split = true\nsummary_follows_filters = true\n",
        )
        .unwrap();

        let app = App::new().unwrap();
        assert!(app.summary_split);
        assert!(app.summary_follows_filters);
    }

    /// The flag survives a write made for another key: the literal in
    /// `layout_config` is the only writer, so a field left out is erased.
    #[test]
    fn toggling_the_heat_view_persists_it_and_survives_another_write() {
        let _guard = env_guard();
        sandbox("heat-view-persist");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        assert!(!app.heat_view, "a Day session opens as a list");
        app.toggle_heat_view();
        assert_eq!(saved_layout()["heat_view"].as_bool(), Some(true));

        app.toggle_summary_split();
        assert_eq!(
            saved_layout()["heat_view"].as_bool(),
            Some(true),
            "another layout write erased the heat view"
        );
    }

    #[test]
    fn toggling_the_summary_heat_persists_it_and_survives_another_write() {
        let _guard = env_guard();
        sandbox("summary-heat-persist");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        assert!(!app.summary_heat, "the strips are opt-in");
        app.toggle_summary_heat();
        assert_eq!(saved_layout()["summary_heat"].as_bool(), Some(true));

        app.toggle_summary_split();
        assert_eq!(
            saved_layout()["summary_heat"].as_bool(),
            Some(true),
            "another layout write erased the summary heat"
        );
    }

    #[test]
    fn the_layout_key_seeds_the_heat_view() {
        let _guard = env_guard();
        let dir = sandbox("heat-view-seed");
        seed(vec![entry(0, "first")], 1);
        std::fs::write(dir.join("config.toml"), "[layout]\nheat_view = true\n").unwrap();

        assert!(App::new().unwrap().heat_view);
    }

    #[test]
    fn liveness_is_read_at_most_once_per_interval_however_many_events_arrive() {
        let _guard = env_guard();
        sandbox("liveness-throttle");
        seed(vec![entry(0, "first")], 1);
        let mark_dir = mark_sandbox();
        // Well clear of the expiry boundary: this test is about the throttle.
        begin_mark(&mark_dir, "smoke.-.impl", 5 * 60);
        let dir = activity_sandbox();

        let mut app = App::new().unwrap();
        app.toggle_marks();
        app.activity_stamp = None;
        app.sync_from_activity();
        assert!(app.unaccounted.is_empty(), "no session to flag yet");

        // However many keypresses drive the loop, the next second's calls keep the result.
        write_session(&dir, "sess-1", "smoke", 3);
        for _ in 0..5 {
            app.activity_stamp = None;
            app.sync_from_activity();
        }
        assert!(app.unaccounted.is_empty(), "throttled, not recomputed");

        app.liveness_at = None; // the interval elapsing
        app.sync_from_activity();
        assert_eq!(app.unaccounted.len(), 1);
        assert_eq!(app.leases.len(), 1, "the leases are kept for the surface");
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

    /// The Summary sits last in the ring, below the panes, and only while it is open.
    #[test]
    fn tab_reaches_the_summary_only_while_it_is_open() {
        let _guard = env_guard();
        sandbox("summary-focus-ring");
        let mut app = seed_panes();

        app.focus = Focus::Table;
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table, "closed: the ring skips it");

        app.toggle_summary();
        app.focus = Focus::Table;
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Summary);
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);

        app.toggle_pane(Pane::Projects);
        app.toggle_pane(Pane::Tags);
        app.focus = Focus::Table;
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Summary, "after every visible pane");
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);

        app.toggle_summary();
        app.focus = Focus::Table;
        app.cycle_focus();
        app.cycle_focus();
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table, "hidden again: two panes only");
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

    /// `Shift-Tab` undoes `Tab` for every surface-visibility combination.
    #[test]
    fn shift_tab_cycles_focus_in_the_exact_reverse_order() {
        let _guard = env_guard();
        sandbox("pane-focus-back");

        for (projects, tags, summary) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (false, false, true),
            (true, false, true),
            (false, true, true),
            (true, true, true),
        ] {
            let mut app = seed_panes();
            if projects {
                app.toggle_pane(Pane::Projects);
            }
            if tags {
                app.toggle_pane(Pane::Tags);
            }
            if summary {
                app.toggle_summary();
            }
            app.focus = Focus::Table;

            let ring_len = 1 + app.visible_panes().len() + usize::from(app.show_summary);
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
                 projects={projects} tags={tags} summary={summary}"
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

    /// `Enter` in a pane cycles the value under its cursor; on the table it does not.
    #[test]
    fn enter_toggles_the_value_under_the_pane_cursor() {
        let _guard = env_guard();
        sandbox("pane-toggle");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);

        // Three Enters walk off → include → exclude → off.
        point_at(&mut app, Pane::Projects, "tt");
        assert!(
            app.cycle_pane_value(true),
            "Enter was not handled by the pane"
        );
        assert_eq!(
            app.pane_value_state(Pane::Projects, "tt"),
            Some(Polarity::Include)
        );
        assert_eq!(in_view(&app), vec!["a", "b"]);
        assert!(app.is_filtering());

        assert!(app.cycle_pane_value(true));
        assert_eq!(
            app.pane_value_state(Pane::Projects, "tt"),
            Some(Polarity::Exclude)
        );
        // Pure negation: `f`, with no project, survives the exclusion.
        assert_eq!(in_view(&app), vec!["c", "f"]);
        assert!(app.is_filtering());

        assert!(app.cycle_pane_value(true));
        assert_eq!(app.pane_value_state(Pane::Projects, "tt"), None);
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);
        assert!(!app.is_filtering());

        app.focus = Focus::Table;
        assert!(!app.cycle_pane_value(true));
        assert!(!app.is_filtering());
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);
    }

    /// `-` walks the cycle the other way: off → exclude → include → off.
    #[test]
    fn minus_cycles_the_pane_value_backwards() {
        let _guard = env_guard();
        sandbox("pane-cycle-back");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;

        point_at(&mut app, Pane::Projects, "tt");
        assert!(app.cycle_pane_value(false), "- was not handled by the pane");
        assert_eq!(
            app.pane_value_state(Pane::Projects, "tt"),
            Some(Polarity::Exclude)
        );
        assert_eq!(in_view(&app), vec!["c", "f"]);

        assert!(app.cycle_pane_value(false));
        assert_eq!(
            app.pane_value_state(Pane::Projects, "tt"),
            Some(Polarity::Include)
        );
        assert_eq!(in_view(&app), vec!["a", "b"]);

        assert!(app.cycle_pane_value(false));
        assert_eq!(app.pane_value_state(Pane::Projects, "tt"), None);
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);

        app.focus = Focus::Table;
        assert!(!app.cycle_pane_value(false));
        assert!(!app.is_filtering());
        assert_eq!(in_view(&app), vec!["a", "b", "c", "f"]);
    }

    /// Excluding a tag hides its entries and keeps the untagged ones.
    #[test]
    fn excluding_a_tag_allows_untagged_entries() {
        let _guard = env_guard();
        sandbox("pane-exclude-tag");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;

        app.tag_filter.cycle("impl", false);
        assert_eq!(in_view(&app), vec!["b", "f"]);

        // An include plus an exclude in the same pane narrows correctly.
        app.tag_filter.cycle("ops", true);
        assert_eq!(in_view(&app), Vec::<String>::new());
        app.tag_filter.cycle("plan", true);
        assert_eq!(in_view(&app), vec!["b"]);
    }

    /// An excluded value renders as `-value` in its pane and `-` in the title.
    #[test]
    fn the_excluded_state_is_rendered_in_the_pane_and_the_title() {
        let _guard = env_guard();
        sandbox("pane-exclude-render");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        app.toggle_pane(Pane::Projects);
        app.toggle_pane(Pane::Tags);
        app.project_filter.cycle("tt", false);
        app.tag_filter.cycle("impl", true);

        let screen = frame_lines(&mut app, 100, 30).join("\n");
        assert!(screen.contains("-tt"), "no `-tt` pane row:\n{screen}");
        assert!(screen.contains("•impl"), "no `•impl` pane row:\n{screen}");
        assert!(
            screen.contains("Entries [filtered: -(tt) #impl]"),
            "the title does not show the exclusion:\n{screen}"
        );
    }

    #[test]
    fn the_help_popup_teaches_both_cycle_keys() {
        let _guard = env_guard();
        sandbox("help-cycle-keys");
        let mut app = seed_panes();
        app.input_mode = InputMode::Help;

        let screen = frame_lines(&mut app, 100, 45).join("\n");
        assert!(screen.contains("pane value: include / exclude / off"));
        assert!(screen.contains("cycle the pane value back"));
    }

    #[test]
    fn the_help_popup_aligns_descriptions_across_sections() {
        let _guard = env_guard();
        sandbox("help-aligned");
        let mut app = seed_panes();
        app.input_mode = InputMode::Help;

        let lines = frame_lines(&mut app, 100, 45);
        let column = |needle: &str| {
            lines
                .iter()
                .find_map(|l| l.find(needle).map(|byte| l[..byte].chars().count()))
                .unwrap_or_else(|| panic!("{needle} missing:\n{}", lines.join("\n")))
        };
        let first = column("previous period");
        for needle in [
            "stop active entry",
            "focus the same ring in reverse",
            "quit",
        ] {
            assert_eq!(column(needle), first, "{needle}");
        }
        let screen = lines.join("\n");
        assert!(
            screen.contains("trim idle from the entry (asks first)"),
            "{screen}"
        );
    }

    /// Every key sits under the surface it acts on, so a reader looking for a
    /// pane's keys finds them under the pane's own name.
    #[test]
    fn the_help_popup_groups_every_key_under_its_surface() {
        let _guard = env_guard();
        sandbox("help-sections");
        let mut app = seed_panes();
        app.input_mode = InputMode::Help;

        let screen = frame_lines(&mut app, 100, 60);
        let heading = |name: &str| {
            let needle = format!("\u{2502}  {name}");
            screen
                .iter()
                .position(|line| line.contains(&needle))
                .unwrap_or_else(|| panic!("no {name} heading:\n{}", screen.join("\n")))
        };
        let order: Vec<usize> = [
            "Navigation",
            "Entries",
            "Projects & Tags",
            "Agents",
            "Summary",
        ]
        .iter()
        .map(|name| heading(name))
        .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "sections out of order: {order:?}"
        );
        assert!(
            !screen.iter().any(|line| line.contains("\u{2502}  Other")),
            "the Other bucket survived:\n{}",
            screen.join("\n")
        );
    }

    #[test]
    fn a_short_help_popup_scrolls_and_clamps() {
        let _guard = env_guard();
        sandbox("help-scroll");
        let mut app = seed_panes();
        app.input_mode = InputMode::Help;

        // The last row of the last section: only ever on the last page.
        const LAST_ROW: &str = "summary heat strips";
        let top = frame_lines(&mut app, 100, 20).join("\n");
        assert!(top.contains("▾ more"), "{top}");
        assert!(top.contains("j/k scroll"), "{top}");
        assert!(!top.contains(LAST_ROW), "{top}");

        app.help_scroll = 1000;
        let bottom = frame_lines(&mut app, 100, 20).join("\n");
        assert!(bottom.contains(LAST_ROW), "{bottom}");
        assert!(!bottom.contains("▾ more"), "{bottom}");
        assert!(app.help_scroll < 1000, "render clamps the offset");

        app.input_mode = InputMode::Help;
        let tall = frame_lines(&mut app, 100, 47).join("\n");
        assert!(
            !tall.contains("▾ more") && !tall.contains("j/k scroll"),
            "{tall}"
        );
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
            if !app.cycle_pane_value(true) {
                app.open_detail();
            }
        };

        point_at(&mut app, Pane::Projects, "tt");
        enter(&mut app);
        assert!(
            app.input_mode == InputMode::Normal,
            "Enter in a pane opened the popover instead of filtering"
        );
        assert_eq!(
            app.pane_value_state(Pane::Projects, "tt"),
            Some(Polarity::Include)
        );

        app.focus = Focus::Table;
        enter(&mut app);
        assert!(
            app.input_mode == InputMode::Detail,
            "Enter on the table did not open the popover"
        );
        assert_eq!(app.table_state.selected(), Some(0));
        assert_eq!(
            app.pane_value_state(Pane::Projects, "tt"),
            Some(Polarity::Include)
        );
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
    fn the_year_view_pages_by_year_instead_of_by_day() {
        let _guard = env_guard();
        sandbox("year-view-paging");
        seed(vec![], 0);

        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Year;
        let start = app.selected_date;

        app.next_period();
        assert_eq!(app.selected_date.year(), start.year() + 1);
        assert_eq!(app.selected_date.month(), start.month());
        assert_eq!(app.selected_date.day(), start.day());

        app.previous_period();
        app.previous_period();
        assert_eq!(app.selected_date.year(), start.year() - 1);
    }

    #[test]
    fn the_year_view_leap_day_falls_back_to_feb_28_in_a_non_leap_year() {
        let _guard = env_guard();
        sandbox("year-view-paging-leap");
        seed(vec![], 0);

        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Year;
        app.selected_date = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();

        app.next_period();

        assert_eq!(
            app.selected_date,
            NaiveDate::from_ymd_opt(2025, 2, 28).unwrap()
        );
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

    /// The background colour of the drawn row carrying `needle`.
    fn row_bg(app: &mut App, needle: &str) -> ratatui::style::Color {
        let width = 120;
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(width, 30)).unwrap();
        terminal.draw(|f| render::ui(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = (0..30)
            .find(|y| {
                (0..width)
                    .map(|x| buffer[(x, *y)].symbol())
                    .collect::<String>()
                    .contains(needle)
            })
            .unwrap_or_else(|| panic!("no drawn row carries {needle}"));
        // Column 3 is inside the table and left of every cursor marker.
        buffer[(3, row)].bg
    }

    /// One rendered frame plus the cursor it asked for.
    fn frame_cursor(app: &mut App, width: u16, height: u16) -> (u16, u16) {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render::ui(f, app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        (pos.x, pos.y)
    }

    /// The legend names every surface key, and `KEYS_WIDTH` still fits it: the
    /// zone never clips, so an undercount eats the end of the legend.
    #[test]
    fn the_footer_legend_names_the_summary_key_without_clipping() {
        let _guard = env_guard();
        sandbox("footer-legend");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        let screen = frame_lines(&mut app, 140, 30);
        let footer = screen
            .iter()
            .rev()
            .find(|line| line.contains("?: help"))
            .unwrap_or_else(|| panic!("no footer:\n{}", screen.join("\n")));
        assert!(
            footer.contains(" | P/T/A/S | Tab | ?: help"),
            "footer legend clipped or missing `S`: {footer}"
        );
        assert!(footer.contains("s: stop"), "the hints zone was clipped");
    }

    /// The foreground of the first cell of `needle` on the footer's key row.
    fn footer_key_fg(app: &mut App, needle: &str) -> ratatui::style::Color {
        const WIDTH: u16 = 140;
        const HEIGHT: u16 = 30;
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(WIDTH, HEIGHT)).unwrap();
        terminal.draw(|f| render::ui(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        for y in 0..HEIGHT {
            let row: String = (0..WIDTH).map(|x| buffer[(x, y)].symbol()).collect();
            if !row.contains("?: help") {
                continue;
            }
            let byte = row
                .find(needle)
                .unwrap_or_else(|| panic!("the footer does not name {needle}: {row}"));
            let column = row[..byte].chars().count() as u16;
            return buffer[(column, y)].fg;
        }
        panic!("no footer row carries the keys");
    }

    /// `Tab` is live whenever the ring reaches past the table, and the Summary
    /// alone is enough to do that.
    #[test]
    fn the_footer_tab_key_is_live_while_the_summary_alone_is_open() {
        let _guard = env_guard();
        sandbox("footer-tab-key");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        assert_eq!(
            footer_key_fg(&mut app, "Tab"),
            theme::inactive(),
            "nothing open: Tab moves nothing"
        );

        app.toggle_summary();
        assert_eq!(
            footer_key_fg(&mut app, "Tab"),
            theme::accent(),
            "the Summary alone is a ring member"
        );

        app.toggle_summary();
        app.toggle_pane(Pane::Projects);
        assert_eq!(
            footer_key_fg(&mut app, "Tab"),
            theme::accent(),
            "a pane open"
        );
    }

    /// The help popup's ring rows name the Summary, not the panes alone.
    #[test]
    fn the_help_popup_names_the_summary_in_the_focus_ring() {
        let _guard = env_guard();
        sandbox("help-ring-rows");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        app.input_mode = InputMode::Help;
        let screen = frame_lines(&mut app, 100, 60);
        let row = |key: &str| {
            screen
                .iter()
                .find(|line| line.contains(key))
                .unwrap_or_else(|| panic!("no {key} row:\n{}", screen.join("\n")))
                .clone()
        };
        assert!(row("Shift-Tab").contains("the same ring in reverse"));
        let split_row = row("human / agent split");
        assert!(
            split_row
                .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{2502}')
                .starts_with("v "),
            "the split row's key column is not `v`: {split_row}"
        );
        assert!(
            split_row.contains("Summary focused"),
            "the split row does not say the key is focus-gated: {split_row}"
        );
        let follow_row = row("follow the filters");
        assert!(
            follow_row
                .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{2502}')
                .starts_with("f "),
            "the follow row's key column is not `f`: {follow_row}"
        );
        assert!(
            follow_row.contains("Summary focused"),
            "the follow row does not say the key is focus-gated: {follow_row}"
        );
        assert!(
            screen
                .iter()
                .any(|line| line.contains("Tab") && line.contains("summary")),
            "no Tab row names the summary:\n{}",
            screen.join("\n")
        );
    }

    #[test]
    fn the_entry_form_stacks_its_fields_three_rows_apart() {
        let _guard = env_guard();
        sandbox("entry-form-layout");
        seed(vec![], 1);

        let mut app = App::new().unwrap();
        app.start_adding();
        let screen = frame_lines(&mut app, 100, 60);
        let row_of = |needle: &str| {
            screen
                .iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} is missing:\n{}", screen.join("\n")))
        };

        let rows: Vec<usize> = [
            " Description ",
            " Project (optional",
            " Tags (space-separated",
            " Duration (optional",
            " Start Time (",
            " End Time (optional",
            " Data (optional",
            " Add Log Entry ", // the help row's own block title
        ]
        .iter()
        .map(|label| row_of(label))
        .collect();

        for pair in rows.windows(2) {
            assert_eq!(pair[1] - pair[0], 3, "form rows drifted: {rows:?}");
        }
        assert_eq!(
            row_of(": switch field") - rows[rows.len() - 1],
            1,
            "the help text left its block"
        );
    }

    /// A terminal too short for the whole form scrolls it rather than
    /// squeezing the boxes: every drawn field keeps its three rows.
    #[test]
    fn a_short_terminal_scrolls_the_entry_form_to_the_active_field() {
        let _guard = env_guard();
        sandbox("entry-form-short");
        seed(vec![], 1);

        let mut app = App::new().unwrap();
        app.start_adding();
        let screen = frame_lines(&mut app, 100, 20).join("\n");
        assert!(screen.contains(" Description "), "top of form:\n{screen}");
        assert!(!screen.contains(" Data (optional"), "form did not clip");

        // Tabbing past the last visible field brings it into view, and the
        // first field goes.
        app.input_field = InputField::Data;
        let scrolled = frame_lines(&mut app, 100, 20).join("\n");
        assert!(
            scrolled.contains(" Data (optional"),
            "scrolled:\n{scrolled}"
        );
        assert!(
            !scrolled.contains(" Description "),
            "top field scrolled off"
        );
        assert!(scrolled.contains(": switch field"), "help row kept");
        assert!(scrolled.contains("(7/7)"), "position marker:\n{scrolled}");
    }

    #[test]
    fn the_form_cursor_lands_on_the_active_field() {
        let _guard = env_guard();
        sandbox("entry-form-cursor");
        seed(vec![], 1);

        let mut app = App::new().unwrap();
        app.start_adding();
        // A multi-byte value: the cursor is placed by display width, not bytes.
        app.input_description.set_from("héllo");
        app.input_project.set_from("acme");

        let first = frame_cursor(&mut app, 100, 60);
        app.next_input_field();
        let second = frame_cursor(&mut app, 100, 60);

        assert_eq!(second.1 - first.1, 3, "cursor did not follow the Tab order");
        // "héllo" is five columns wide, "acme" four — both share the chunk's x.
        assert_eq!(first.0 - second.0, 1, "cursor column ignored display width");
    }

    /// #73: what the Data field holds is parsed and stored on save.
    #[test]
    fn the_form_saves_the_data_field_as_json() {
        let _guard = env_guard();
        sandbox("form-data-save");
        seed(vec![], 0);

        let mut app = App::new().unwrap();
        app.start_adding();
        app.input_description.set_from("with data");
        app.input_duration.set_from("30m");
        app.input_data.set_from(r#"{"pr": 42}"#);
        app.submit_entry().unwrap();

        assert_eq!(app.input_mode, InputMode::Normal, "the form stayed open");
        let stored = storage::load_data().unwrap();
        assert_eq!(stored.entries[0].data, Some(serde_json::json!({"pr": 42})));
    }

    /// #72: invalid JSON refuses the save, says so, and writes nothing.
    #[test]
    fn invalid_json_refuses_the_save_and_reports_it() {
        let _guard = env_guard();
        sandbox("form-data-invalid");
        seed(vec![], 0);

        let mut app = App::new().unwrap();
        app.start_adding();
        app.input_description.set_from("with data");
        app.input_duration.set_from("30m");
        app.input_data.set_from("{not json");
        app.submit_entry().unwrap();

        assert_eq!(
            app.input_mode,
            InputMode::AddingEntry,
            "the form closed over a value it never saved"
        );
        assert!(app.form_error.is_some(), "no reason was given");
        assert!(
            storage::load_data().unwrap().entries.is_empty(),
            "a refused save still wrote an entry"
        );
        // The message reaches the screen, in the help row.
        let screen = frame_lines(&mut app, 120, 40).join("\n");
        assert!(screen.contains("invalid JSON"), "not shown:\n{screen}");

        // Typing clears it, and a valid value then saves.
        app.input_field = InputField::Data;
        app.handle_input_backspace();
        assert_eq!(app.form_error, None, "the error outlived the edit");
        app.input_data.set_from(r#"{"ok": true}"#);
        app.submit_entry().unwrap();
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(
            storage::load_data().unwrap().entries[0].data,
            Some(serde_json::json!({"ok": true}))
        );
    }

    /// Editing round-trips the stored JSON through the field, and blanking it
    /// clears the entry's data rather than leaving the old value behind.
    #[test]
    fn editing_round_trips_the_data_field_and_a_blank_clears_it() {
        let _guard = env_guard();
        sandbox("form-data-edit");
        let mut with_data = entry(0, "has data");
        with_data.end_time = Some(with_data.start_time + chrono::Duration::hours(1));
        with_data.data = Some(serde_json::json!({"pr": 42}));
        seed(vec![with_data], 1);

        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));
        app.start_editing();
        assert_eq!(app.input_data.value(), r#"{"pr":42}"#);

        app.input_data.set_from(r#"{"pr":43}"#);
        app.submit_edit().unwrap();
        assert_eq!(
            storage::load_data().unwrap().entries[0].data,
            Some(serde_json::json!({"pr": 43}))
        );

        app.table_state.select(Some(0));
        app.start_editing();
        app.input_data.clear();
        app.submit_edit().unwrap();
        assert_eq!(
            storage::load_data().unwrap().entries[0].data,
            None,
            "a blank field left the old data in place"
        );
    }

    /// #70: the popover lists the custom JSON as key/value rows under a Data
    /// header, flattening nesting rather than printing raw JSON.
    #[test]
    fn the_detail_popover_renders_custom_data_as_key_value_rows() {
        let _guard = env_guard();
        sandbox("detail-data");
        let mut with_data = entry(0, "has data");
        with_data.data = Some(serde_json::json!({
            "pr": 42,
            "review": {"by": "linus"},
        }));
        seed(vec![with_data], 1);

        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));
        app.open_detail();
        let screen = frame_lines(&mut app, 100, 40).join("\n");

        assert!(screen.contains("Data"), "no Data header:\n{screen}");
        assert!(screen.contains("pr"), "no key row:\n{screen}");
        assert!(screen.contains("42"), "no value:\n{screen}");
        assert!(
            screen.contains("review.by") && screen.contains("linus"),
            "nesting was not flattened:\n{screen}"
        );
        assert!(!screen.contains('{'), "raw JSON leaked in:\n{screen}");
    }

    /// An entry without data is rendered exactly as it was before the field existed.
    #[test]
    fn the_detail_popover_has_no_data_section_without_data() {
        let _guard = env_guard();
        sandbox("detail-no-data");
        seed(vec![entry(0, "plain")], 1);

        let mut app = App::new().unwrap();
        app.table_state.select(Some(0));
        app.open_detail();
        let screen = frame_lines(&mut app, 100, 40).join("\n");

        assert!(screen.contains("Description"), "the popover did draw");
        assert!(!screen.contains("Data"), "an empty Data header:\n{screen}");
    }

    #[test]
    fn a_running_member_keeps_the_group_and_day_totals_moving() {
        let _guard = env_guard();
        sandbox("group-live-total");
        let today = Local::now().date_naive();

        // The running member is placed so its whole-minute count ticks over
        // 500 ms from now, and the two reads below straddle that tick rather
        // than an arbitrary one. Slipping past it early fails the first read,
        // which is the safe direction.
        let mut running = logged(0, "still going", "tt", &["tt/174"], today, 60);
        running.start_time =
            Local::now() - chrono::Duration::minutes(30) + chrono::Duration::milliseconds(500);
        running.end_time = None;
        seed(
            vec![
                running,
                logged(1, "done", "tt", &["tt/174"], today, 60),
                logged(2, "loose", "tt", &[], today, 10),
            ],
            3,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Week;

        let line = |lines: &[String], needle: &str| {
            lines
                .iter()
                .find(|l| l.contains(needle))
                .cloned()
                .unwrap_or_else(|| panic!("no line carries {needle}"))
        };
        let weekday = today.format("%A").to_string();

        let before = frame_lines(&mut app, 120, 30);
        assert!(
            line(&before, "entries").contains("1h 29m"),
            "group total at rest:\n{}",
            line(&before, "entries")
        );
        assert!(
            line(&before, &weekday).contains("1h 39m"),
            "day total at rest:\n{}",
            line(&before, &weekday)
        );

        std::thread::sleep(std::time::Duration::from_millis(1000));

        let after = frame_lines(&mut app, 120, 30);
        assert!(
            line(&after, "entries").contains("1h 30m"),
            "the group total froze with the row cache:\n{}",
            line(&after, "entries")
        );
        assert!(
            line(&after, &weekday).contains("1h 40m"),
            "the day total froze with the row cache:\n{}",
            line(&after, &weekday)
        );
    }

    #[test]
    fn the_entries_table_draws_a_group_as_one_row_until_it_is_expanded() {
        let _guard = env_guard();
        sandbox("group-render");
        let mut app = seed_grouped();

        let screen = frame_lines(&mut app, 100, 30).join("\n");
        assert!(screen.contains("#tt/174"), "no issue tag:\n{screen}");
        assert!(
            screen.contains("▸ 3 entries - tt/174"),
            "no collapsed row:\n{screen}"
        );
        assert!(screen.contains("3h 0m"), "no summed duration:\n{screen}");
        assert!(!screen.contains("round"), "a member leaked:\n{screen}");
        assert!(screen.contains("hand written"), "no plain row:\n{screen}");

        app.expanded_issues.insert("tt/174".to_string());
        let screen = frame_lines(&mut app, 100, 30).join("\n");
        assert!(
            screen.contains("▾ 3 entries - tt/174"),
            "no expanded row:\n{screen}"
        );
        for member in ["round one", "round two", "round three"] {
            assert!(screen.contains(member), "{member} is missing:\n{screen}");
        }
    }

    /// A member is told apart from a top-level row two ways at once, so neither
    /// a narrow terminal nor a colour-blind palette leaves it ambiguous. The
    /// connector sits in the column the header's chevron sits in.
    #[test]
    fn an_expanded_member_row_is_tinted_and_hangs_off_a_tree_connector() {
        let _guard = env_guard();
        sandbox("group-member-style");
        let mut app = seed_grouped();
        app.expanded_issues.insert("tt/174".to_string());
        // Off the group header, whose own colour the cursor would override.
        app.select_by_id(3);

        let screen = frame_lines(&mut app, 120, 30).join("\n");
        assert!(
            screen.contains("10:00 \u{251c}\u{2500}\u{2500} round one"),
            "no branch connector on a member:\n{screen}"
        );
        assert!(
            screen.contains("10:00 \u{2514}\u{2500}\u{2500} round three"),
            "the last member does not close the tree:\n{screen}"
        );
        assert!(
            screen.contains("10:00 hand written"),
            "a top-level row moved:\n{screen}"
        );

        assert_eq!(row_bg(&mut app, "round one"), theme::member_bg());
        assert_eq!(
            row_bg(&mut app, "▾ 3 entries - tt/174"),
            theme::group_header_bg()
        );
        assert_ne!(
            row_bg(&mut app, "hand written"),
            theme::member_bg(),
            "a top-level row was tinted"
        );
    }

    /// 80 columns is the width the hint zone clips at, and the total and the
    /// label are both variable-width, so the widest of each is what to test.
    #[test]
    fn the_footer_keeps_the_scope_hints_at_80_columns() {
        let _guard = env_guard();
        sandbox("footer-80-cols");
        let today = Local::now().date_naive();
        seed(
            vec![
                logged(0, "long one", "tt", &["tt/174"], today, 12 * 60),
                logged(1, "long two", "tt", &["tt/174"], today, 30),
            ],
            2,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;

        // By position, not by content: a clipped legend may carry neither the
        // label nor the hint the assertions are looking for.
        let footer = |app: &mut App| {
            let lines = frame_lines(app, 80, 30);
            lines[lines.len() - 2].clone()
        };

        let plain = footer(&mut app);
        assert!(
            !plain.contains("Total"),
            "the total lives in the Summary: {plain}"
        );
        assert!(plain.starts_with("│ t: today"), "{plain}");
        assert!(plain.ends_with("?: help│"), "{plain}");

        // A filter changes the Summary marker, never the footer.
        app.tag_filter.cycle("tt/174", true);
        let filtered = footer(&mut app);
        assert_eq!(filtered, plain);
    }

    #[test]
    fn the_group_key_is_listed_in_the_help_overlay_but_not_the_footer() {
        let _guard = env_guard();
        sandbox("group-key-hints");
        let mut app = seed_grouped();

        let footer = frame_lines(&mut app, 200, 30).join("\n");
        assert!(!footer.contains("g: toggle"), "footer legend:\n{footer}");
        assert!(footer.contains("s: stop"), "footer legend:\n{footer}");

        app.input_mode = InputMode::Help;
        let help = frame_lines(&mut app, 100, 40).join("\n");
        assert!(
            help.contains("toggle the issue group"),
            "help popup:\n{help}"
        );
    }

    #[test]
    fn the_agents_panel_marks_a_stale_mark_and_leaves_a_fresh_one_alone() {
        let _guard = env_guard();
        sandbox("stale-render");
        seed(vec![entry(0, "first")], 1);
        let dir = mark_sandbox();
        // Fresh: beaten just now. Stale: opened well past the unvouched grace.
        begin_mark(&dir, "fresh.-.impl", 4 * 60);
        beat_mark(&dir, "fresh.-.impl");
        begin_mark(&dir, "stale.-.impl", 5 * 60);

        let mut app = App::new().unwrap();
        app.toggle_marks();
        app.sync_from_activity();

        let screen = frame_lines(&mut app, 100, 30);
        let row = |label: &str| {
            screen
                .iter()
                .find(|line| line.contains(label))
                .unwrap_or_else(|| panic!("no row for {label}:\n{}", screen.join("\n")))
                .clone()
        };
        assert!(row("stale impl").contains("[stale]"));
        assert!(row("stale impl").contains("last seen never"));
        let fresh = row("fresh impl");
        assert!(!fresh.contains("[stale]"), "{fresh}");
        assert!(fresh.contains("last seen "), "{fresh}");
    }

    #[test]
    fn the_agents_panel_renders_the_unaccounted_section_when_flagged() {
        let _guard = env_guard();
        sandbox("unaccounted-render");
        seed(vec![entry(0, "first")], 1);
        let mut app = seed_marks(&[("tt.14.impl", 2)]);
        app.toggle_marks();

        let dir = activity_sandbox();
        write_session(&dir, "sess-1", "smoke-project", 3);
        app.activity_stamp = None;
        app.sync_from_activity();

        let screen = frame_lines(&mut app, 100, 30).join("\n");
        assert!(
            screen.contains("unaccounted activity"),
            "the header should appear once something is flagged:\n{screen}"
        );
        assert!(
            screen.contains("smoke-project"),
            "the flagged project should be named:\n{screen}"
        );
        // The marks section is untouched by the addition.
        assert!(screen.contains("tt/14"));
    }

    #[test]
    fn the_agents_panel_has_no_unaccounted_section_when_nothing_is_flagged() {
        let _guard = env_guard();
        sandbox("unaccounted-render-empty");
        seed(vec![entry(0, "first")], 1);
        let mut app = seed_marks(&[("tt.14.impl", 2)]);
        app.toggle_marks();

        let screen = frame_lines(&mut app, 100, 30).join("\n");
        assert!(
            !screen.contains("unaccounted"),
            "a clean session must render exactly as it did before:\n{screen}"
        );
    }

    #[test]
    fn the_status_bar_shows_an_update_notice_when_one_is_set() {
        let _guard = env_guard();
        sandbox("update-notice-render");
        seed(vec![], 0);

        let mut app = App::new().unwrap();
        app.update_notice = Some("9.9.9".to_string());

        let screen = frame_lines(&mut app, 100, 24).join("\n");
        assert!(
            screen.contains("9.9.9") && screen.contains("tt update"),
            "the status bar should name the available version and the CTA:\n{screen}"
        );
    }

    #[test]
    fn no_update_notice_leaves_the_status_bar_unchanged() {
        let _guard = env_guard();
        sandbox("update-notice-absent");
        seed(vec![], 0);

        let mut app = App::new().unwrap();
        assert_eq!(app.update_notice, None);

        let screen = frame_lines(&mut app, 100, 24).join("\n");
        assert!(!screen.contains("tt update"));
    }

    #[test]
    fn the_year_view_renders_a_grid_with_labels_and_legend() {
        let _guard = env_guard();
        sandbox("year-view-render");
        seed(
            vec![
                logged(
                    1,
                    "a",
                    "tt",
                    &["impl"],
                    NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
                    30,
                ),
                logged(
                    2,
                    "b",
                    "tt",
                    &["impl"],
                    NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
                    300,
                ),
                logged(
                    3,
                    "c",
                    "tt",
                    &["impl"],
                    NaiveDate::from_ymd_opt(2026, 12, 20).unwrap(),
                    120,
                ),
            ],
            4,
        );

        let mut app = App::new().unwrap();
        app.heat_view = true;
        app.view_mode = ViewMode::Year;
        app.selected_date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();

        let screen = frame_lines(&mut app, 140, 24).join("\n");

        assert!(
            screen.contains("Yearly View"),
            "tab title shows the new view"
        );
        assert!(
            screen.contains("Year 2026"),
            "date_info names the shown year"
        );
        assert!(
            screen.contains("Mon") && screen.contains("Sun"),
            "all seven weekday labels render:\n{screen}"
        );
        assert!(
            screen.contains("Jan") && screen.contains("Dec"),
            "month labels span the whole year:\n{screen}"
        );
        assert!(
            screen.contains("Less") && screen.contains("More"),
            "the heat legend renders:\n{screen}"
        );
        assert!(
            screen.contains("active day"),
            "the block title reports the active-day count:\n{screen}"
        );
    }

    /// 2026 opens on a Thursday and closes on a Thursday, so its grid is 53
    /// Monday-opened weeks wide.
    const WEEKS_IN_2026: usize = 53;

    /// The first inner column of the year grid: the block border and the
    /// five-column year gutter.
    const GRID_LEFT: u16 = 6;

    fn year_view_2026() -> App {
        seed(
            vec![logged(
                1,
                "a",
                "tt",
                &["impl"],
                NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
                300,
            )],
            2,
        );
        let mut app = App::new().unwrap();
        app.heat_view = true;
        app.view_mode = ViewMode::Year;
        app.selected_date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        app
    }

    /// A wider window gives each week a fatter cell; it never invents weeks.
    #[test]
    fn a_wider_year_grid_widens_its_cells_rather_than_adding_weeks() {
        let _guard = env_guard();
        sandbox("year-view-width");
        let mut app = year_view_2026();

        // The Monday band opens in 2025, so its first cell is left unpainted
        // and the gap before the painted ones is exactly one cell wide.
        let band = |rows: Vec<Vec<(u16, ratatui::style::Color)>>| {
            let painted = rows[0].len();
            let cell_width = (rows[0][0].0 - GRID_LEFT) as usize;
            (cell_width, painted / cell_width)
        };
        let (narrow_width, narrow_weeks) = band(heat_block_cells(&mut app, 140, 30).1);
        let (wide_width, wide_weeks) = band(heat_block_cells(&mut app, 280, 30).1);

        assert_eq!(narrow_width, 2, "138 inner columns over 53 weeks");
        assert_eq!(wide_width, 5, "twice the width did not widen the cells");
        assert_eq!(
            narrow_weeks, wide_weeks,
            "the wider grid grew weeks instead of cells"
        );
        assert_eq!(
            narrow_weeks + 1,
            WEEKS_IN_2026,
            "the grid lost weeks it had room for"
        );
    }

    /// A narrow window narrows the cells; the year keeps every week of its own.
    #[test]
    fn a_narrow_year_grid_keeps_every_week_of_the_year() {
        let _guard = env_guard();
        sandbox("year-view-narrow");
        let mut app = year_view_2026();

        // 58 inner columns less the gutter leave one each for 53 weeks; the
        // Monday band opens in 2025, so one of its cells stays unpainted.
        let (_, rows) = heat_block_cells(&mut app, 60, 30);
        assert_eq!(rows[0].len(), WEEKS_IN_2026 - 1);
    }

    /// The grid takes the height it is given, and names each weekday once.
    #[test]
    fn a_taller_year_grid_grows_its_weekday_bands() {
        let _guard = env_guard();
        sandbox("year-view-height");
        let mut app = year_view_2026();

        let (_, short) = heat_block_cells(&mut app, 140, 20);
        let (_, tall) = heat_block_cells(&mut app, 140, 40);
        assert!(
            tall.len() > short.len(),
            "the bands did not grow: {} rows at both heights",
            short.len()
        );
        for rows in [&short, &tall] {
            assert_eq!(rows.len() % 7, 0, "the bands are uneven: {}", rows.len());
        }

        let screen = frame_lines(&mut app, 140, 40);
        assert_eq!(
            screen
                .iter()
                .filter(|line| line.starts_with("\u{2502} Mon "))
                .count(),
            1,
            "the weekday label repeats down its band:\n{}",
            screen.join("\n")
        );
    }

    /// The Month block's tick line and its heat cells, one inner row per
    /// vector, as `(x, colour)`. A cell is a blank symbol, so only the
    /// background says where it is. The legend rides a border row, so any row
    /// carrying a box corner or rule is left out.
    fn heat_block_cells(
        app: &mut App,
        width: u16,
        height: u16,
    ) -> (String, Vec<Vec<(u16, ratatui::style::Color)>>) {
        // One sample per step of the ramp: empty, then each level in turn.
        let palette: Vec<ratatui::style::Color> = [0, 1, 3, 5, 9]
            .into_iter()
            .map(super::theme::heat_color)
            .collect();
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render::ui(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row_text = |y: u16| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        let rows: Vec<(u16, Vec<(u16, ratatui::style::Color)>)> = (0..height)
            .filter(|y| !row_text(*y).contains(['\u{2500}', '\u{2514}', '\u{2518}']))
            .map(|y| {
                let cells: Vec<(u16, ratatui::style::Color)> = (0..width)
                    .map(|x| (x, buffer[(x, y)].bg))
                    .filter(|(_, bg)| palette.contains(bg))
                    .collect();
                (y, cells)
            })
            .filter(|(_, cells)| !cells.is_empty())
            .collect();
        let ticks = rows
            .first()
            .map(|(y, _)| row_text(y.saturating_sub(1)).trim_end().to_string())
            .unwrap_or_default();
        (ticks, rows.into_iter().map(|(_, cells)| cells).collect())
    }

    /// The tabs read in period order, shortest first, with `All` last.
    #[test]
    fn the_tabs_row_lists_the_five_views_in_period_order() {
        let _guard = env_guard();
        sandbox("tabs-period-order");
        seed(vec![], 0);

        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Month;
        app.selected_date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();

        let screen = frame_lines(&mut app, 140, 24);
        let tabs = screen
            .iter()
            .find(|line| line.contains("[1] Day"))
            .unwrap_or_else(|| panic!("no tabs row:\n{}", screen.join("\n")))
            .clone();

        let order: Vec<usize> = ["[1] Day", "[2] Week", "[3] Month", "[4] Year", "[5] All"]
            .iter()
            .map(|tab| {
                tabs.find(tab)
                    .unwrap_or_else(|| panic!("no `{tab}` on the tabs row: {tabs}"))
            })
            .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "the tabs are out of period order: {tabs}"
        );

        let title = screen
            .iter()
            .find(|line| line.contains("Monthly View"))
            .unwrap_or_else(|| panic!("no Month title:\n{}", screen.join("\n")));
        assert!(
            title.contains("June 2026"),
            "the Month tab does not name its month: {title}"
        );
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
        let prompt: String = screen
            .lines()
            .filter(|line| !line.contains("v: split"))
            .collect();
        assert!(
            !prompt.contains("split"),
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
        app.cycle_pane_value(true);
        assert_eq!(in_view(&app), vec!["a", "b"]);

        // A second project widens the set — the two are OR'd.
        point_at(&mut app, Pane::Projects, "loremind");
        app.cycle_pane_value(true);
        assert_eq!(in_view(&app), vec!["a", "b", "c"]);

        // A tag narrows within them — the panes are AND'd.
        point_at(&mut app, Pane::Tags, "impl");
        app.cycle_pane_value(true);
        assert_eq!(in_view(&app), vec!["a", "c"]);

        point_at(&mut app, Pane::Tags, "plan");
        app.cycle_pane_value(true);
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
        app.cycle_pane_value(true);
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

    /// One project logged by a person, by `#agent` and by `#auto`, plus a second
    /// project that is all human. Day: tt 180m (60 human, 120 agent), solo 30m.
    fn seed_agent_summary() -> App {
        let today = Local::now().date_naive();
        seed(
            vec![
                logged(0, "hand written", "tt", &["impl"], today, 60),
                logged(1, "agent run", "tt", &["Agent"], today, 90),
                logged(2, "auto run", "tt", &["auto"], today, 30),
                logged(3, "all human", "solo", &[], today, 30),
            ],
            4,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app
    }

    /// Agent time is split out per row, and the two halves always rebuild the total.
    #[test]
    fn project_summary_splits_human_and_agent_time() {
        let _guard = env_guard();
        sandbox("summary-split-fold");
        let app = seed_agent_summary();

        let rows = app.project_summary();
        let minutes = |name: &str| {
            let row = rows.iter().find(|r| r.project == name).unwrap();
            (
                row.total.num_minutes(),
                row.human.num_minutes(),
                row.agent.num_minutes(),
            )
        };
        assert_eq!(minutes("tt"), (180, 60, 120));
        assert_eq!(minutes("solo"), (30, 30, 0));
    }

    /// A running entry has no end time, so it counts as human time up to now.
    #[test]
    fn a_running_entry_counts_as_human_time() {
        let _guard = env_guard();
        sandbox("summary-split-running");
        let today = Local::now().date_naive();
        let mut running = logged(0, "still going", "tt", &["impl"], today, 0);
        running.start_time = Local::now() - chrono::Duration::minutes(30);
        running.end_time = None;
        seed(vec![running], 1);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;

        let row = &app.project_summary()[0];
        assert_eq!(row.agent, chrono::Duration::zero());
        assert_eq!(row.human, row.total);
        assert!(row.total.num_minutes() >= 29, "the open span is counted");
    }

    /// The split can never disagree with the total the surface already shows.
    #[test]
    fn every_summary_row_rebuilds_its_total_from_the_split() {
        let _guard = env_guard();
        sandbox("summary-split-sums");
        let mut app = seed_summary();

        for (mode, name) in scopes() {
            app.view_mode = mode;
            for row in app.project_summary() {
                assert_eq!(
                    row.human + row.agent,
                    row.total,
                    "{name}: {} does not rebuild its total",
                    row.project
                );
            }
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
    fn project_summary_ignores_the_filter_and_search_in_scope_only_mode() {
        let _guard = env_guard();
        sandbox("summary-prefilter");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        let before = app.project_summary();
        let in_scope = app.scope_entries().len();

        app.project_filter.cycle("tt", true);
        assert!(
            app.filtered_entries().len() < in_scope,
            "filter did not bite"
        );
        assert_eq!(app.project_summary(), before);

        app.project_filter.clear();
        app.tag_filter.cycle("ops", true);
        assert!(
            app.filtered_entries().len() < in_scope,
            "filter did not bite"
        );
        assert_eq!(app.project_summary(), before);

        app.search_term.set_from("nothing matches this");
        assert!(app.filtered_entries().is_empty());
        assert_eq!(app.project_summary(), before);
    }

    #[test]
    fn project_summary_follows_the_filter_and_search_in_follow_mode() {
        let _guard = env_guard();
        sandbox("summary-follows-filter");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        let scope_only = app.project_summary();
        app.toggle_summary_follows_filters();
        assert_eq!(
            app.project_summary(),
            scope_only,
            "an unset filter moves it"
        );

        let summed = |app: &App| {
            app.project_summary()
                .iter()
                .fold(chrono::Duration::zero(), |acc, row| acc + row.total)
        };

        app.project_filter.cycle("tt", true);
        assert_eq!(summary(&app), "tt=90m/2/100%");
        assert_eq!(summed(&app), app.filtered_total());

        app.project_filter.clear();
        app.tag_filter.cycle("ops", true);
        assert_eq!(summary(&app), "vinge=120m/1/73% (no project)=45m/1/27%");
        assert_eq!(summed(&app), app.filtered_total());

        app.tag_filter.clear();
        app.search_term.set_from("nothing matches this");
        assert!(app.project_summary().is_empty());
        assert_eq!(app.filtered_total(), chrono::Duration::zero());

        // The mode off again restores the scope-only rows byte for byte.
        app.search_term.clear();
        app.project_filter.cycle("tt", true);
        app.toggle_summary_follows_filters();
        assert_eq!(app.project_summary(), scope_only);
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

    /// Every bucket list rebuilds the total printed on its own row, in every
    /// view, so a strip can never disagree with the row it sits on.
    #[test]
    fn project_buckets_sum_to_the_row_total_in_every_view() {
        let _guard = env_guard();
        sandbox("summary-buckets-sum");
        let mut app = seed_summary();

        for (mode, name) in scopes().into_iter().chain([(ViewMode::Year, "year")]) {
            app.view_mode = mode;
            let rows = app.project_summary();
            let grid = app.project_buckets(&rows);
            assert_eq!(
                grid.rows.len(),
                rows.len(),
                "{name}: a row lost its buckets"
            );
            for (row, cells) in rows.iter().zip(&grid.rows) {
                let summed = cells.iter().fold(chrono::Duration::zero(), |a, c| a + *c);
                assert_eq!(summed, row.total, "{name}: {} lost time", row.project);
            }
        }
    }

    /// An entry that runs past midnight stays whole in the hour it started.
    #[test]
    fn an_entry_lands_wholly_in_the_bucket_holding_its_start() {
        let _guard = env_guard();
        sandbox("summary-buckets-start");
        let today = Local::now().date_naive();
        let mut late = logged(1, "over midnight", "tt", &[], today, 60);
        late.start_time = today
            .and_hms_opt(23, 30, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        late.end_time = Some(late.start_time + chrono::Duration::minutes(60));
        seed(vec![logged(0, "morning", "tt", &[], today, 30), late], 2);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;

        let rows = app.project_summary();
        let grid = app.project_buckets(&rows);
        assert_eq!(grid.grain, super::summary::Grain::Hour);
        let cells = &grid.rows[0];
        assert_eq!(cells.len(), 24, "the whole day, midnight to midnight");
        assert_eq!(cells[9].num_minutes(), 30, "the 09:00 hour");
        assert_eq!(cells[23].num_minutes(), 60, "the whole hour-long entry");
        assert_eq!(
            cells[10..23]
                .iter()
                .fold(chrono::Duration::zero(), |a, c| a + *c),
            chrono::Duration::zero(),
            "time leaked into the hours between"
        );
        assert_eq!(
            cells[0].num_minutes(),
            0,
            "the entry ran past midnight into no bucket of its own"
        );
    }

    /// Bucket lists are index for index with the rows they were given.
    #[test]
    fn project_buckets_line_up_with_the_rows_they_were_given() {
        let _guard = env_guard();
        sandbox("summary-buckets-order");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;

        let rows = app.project_summary();
        let grid = app.project_buckets(&rows);
        assert_eq!(grid.grain, super::summary::Grain::Day);
        let minutes = |name: &str| {
            let index = rows.iter().position(|row| row.project == name).unwrap();
            grid.rows[index]
                .iter()
                .map(|cell| cell.num_minutes())
                .collect::<Vec<_>>()
        };
        // vinge is logged on day two alone, tt on day one alone.
        assert_eq!(minutes("vinge"), vec![0, 120, 0, 0, 0, 0, 0]);
        assert_eq!(minutes("tt"), vec![90, 0, 0, 0, 0, 0, 0]);
    }

    /// The drawn Summary box, between its top and bottom borders.
    fn summary_box(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let screen = frame_lines(app, width, height);
        let top = screen
            .iter()
            .position(|line| line.contains("Summary (S)"))
            .unwrap_or_else(|| panic!("no Summary box:\n{}", screen.join("\n")));
        screen[top + 1..]
            .iter()
            .take_while(|line| !line.contains('\u{2518}'))
            .cloned()
            .collect()
    }

    /// Collapsed, the box is `Total: …` alone; `S` opens the breakdown over a
    /// rule, and the total row sums what it shows.
    #[test]
    fn the_summary_total_row_is_the_collapsed_box_and_the_expanded_foot() {
        let _guard = env_guard();
        sandbox("summary-total-row");
        let today = Local::now().date_naive();
        seed(
            vec![
                logged(0, "a", "tt", &[], today, 60),
                logged(1, "b", "tt", &[], today, 30),
                logged(2, "c", "vinge", &["agent"], today, 45),
            ],
            3,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;

        let collapsed = summary_box(&mut app, 100, 40);
        assert_eq!(collapsed.len(), 1, "{collapsed:#?}");
        assert!(
            collapsed[0].starts_with("│ Total: 2h 15m"),
            "{}",
            collapsed[0]
        );
        assert!(
            !collapsed[0].contains(" 3 "),
            "no count while collapsed: {}",
            collapsed[0]
        );

        app.toggle_summary();
        let expanded = summary_box(&mut app, 100, 40);
        // header, two projects, rule, total
        assert_eq!(expanded.len(), 5, "{expanded:#?}");
        assert!(
            expanded[3].contains("\u{2500}\u{2500}"),
            "no rule: {}",
            expanded[3]
        );
        assert!(
            !expanded[3].contains("total"),
            "the rule carries no text: {}",
            expanded[3]
        );
        let foot = &expanded[4];
        assert!(foot.contains(" total"), "{foot}");
        assert!(foot.contains("2h 15m"), "{foot}");
        assert!(foot.contains(" 3 "), "the count sums too: {foot}");
    }

    /// The header names the columns in both modes, and `v` adds the two halves
    /// between `total` and `count`.
    #[test]
    fn the_summary_heads_its_columns_in_both_modes() {
        let _guard = env_guard();
        sandbox("summary-header");
        let mut app = seed_agent_summary();
        app.toggle_summary();

        let unsplit = summary_box(&mut app, 100, 40);
        assert!(
            unsplit[0].starts_with("\u{2502} project    total count share"),
            "unsplit header: {}",
            unsplit[0]
        );
        assert!(
            unsplit[1].starts_with("\u{2502} tt         3h 0m     3   86%"),
            "unsplit row: {}",
            unsplit[1]
        );

        app.toggle_summary_split();
        let split = summary_box(&mut app, 100, 40);
        assert!(
            split[0].starts_with("\u{2502} project    total    human    agent count share"),
            "split header: {}",
            split[0]
        );
        assert!(
            split[1].starts_with("\u{2502} tt         3h 0m    1h 0m    2h 0m     3   86%"),
            "split row: {}",
            split[1]
        );
        assert!(
            split[2].starts_with("\u{2502} solo      0h 30m   0h 30m    0h 0m     1   14%"),
            "split row two: {}",
            split[2]
        );
    }

    /// The time columns hold `100h 30m` and still keep a gap between them.
    #[test]
    fn three_digit_hours_keep_a_gap_between_the_time_columns() {
        let _guard = env_guard();
        sandbox("summary-wide-hours");
        let today = Local::now().date_naive();
        seed(
            vec![
                logged(0, "a long stint", "tt", &[], today, 6030),
                logged(1, "a long agent run", "tt", &["agent"], today, 6030),
            ],
            2,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.toggle_summary();
        app.toggle_summary_split();

        let drawn = summary_box(&mut app, 100, 40);
        assert!(
            drawn[1].starts_with("\u{2502} tt       201h 0m 100h 30m 100h 30m     2  100%"),
            "the time columns ran together: {}",
            drawn[1]
        );
    }

    /// The label column follows the longest visible name and never sits on a floor.
    #[test]
    fn the_summary_label_column_follows_the_longest_project_name() {
        let _guard = env_guard();
        sandbox("summary-label-width");
        let today = Local::now().date_naive();
        seed(
            vec![
                logged(0, "a", "a-very-long-project-name", &[], today, 60),
                logged(1, "b", "tt", &[], today, 30),
            ],
            2,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.toggle_summary();

        let wide = summary_box(&mut app, 100, 40);
        assert!(
            wide[0].starts_with("\u{2502} project                     total"),
            "long names did not widen the label column: {}",
            wide[0]
        );
        assert!(
            wide[1].starts_with("\u{2502} a-very-long-project-name    1h 0m"),
            "the row does not use the widened column: {}",
            wide[1]
        );

        // Short names pull the column in to `project`.
        seed(vec![logged(0, "a", "tt", &[], today, 60)], 1);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.toggle_summary();
        let narrow = summary_box(&mut app, 100, 40);
        assert!(
            narrow[0].starts_with("\u{2502} project    total"),
            "short names did not pull the column in: {}",
            narrow[0]
        );
    }

    /// A box too narrow for every column clips the right edge, and `share` goes
    /// first. No column is dropped and nothing wraps.
    #[test]
    fn a_narrow_summary_clips_its_right_edge_rather_than_wrapping() {
        let _guard = env_guard();
        sandbox("summary-narrow");
        let mut app = seed_agent_summary();
        app.toggle_summary();
        app.toggle_summary_split();

        let narrow = summary_box(&mut app, 40, 40);
        assert!(
            narrow[0].starts_with("\u{2502} project    total    human"),
            "the narrow header lost a left column: {}",
            narrow[0]
        );
        assert!(
            !narrow[0].contains("share"),
            "share survived: {}",
            narrow[0]
        );
        assert!(!narrow[1].contains('%'), "the share column survived");
        // One line per project still, so nothing wrapped onto a second row.
        assert_eq!(narrow.len(), 5, "{narrow:#?}");
    }

    /// The five views, with a name to report against; `ViewMode` is not `Debug`.
    fn views() -> [(ViewMode, &'static str); 5] {
        [
            (ViewMode::Day, "day"),
            (ViewMode::Week, "week"),
            (ViewMode::Month, "month"),
            (ViewMode::Year, "year"),
            (ViewMode::All, "all"),
        ]
    }

    /// One flag decides the whole content area, in every view: the list and the
    /// heat never share it.
    #[test]
    fn every_view_draws_its_list_or_its_heat_by_the_flag() {
        let _guard = env_guard();
        sandbox("content-dispatch");
        let today = Local::now().date_naive();
        seed(vec![logged(0, "a", "tt", &[], today, 60)], 1);
        let mut app = App::new().unwrap();
        app.selected_date = today;

        for (view, name) in views() {
            app.view_mode = view;

            app.heat_view = false;
            let list = frame_lines(&mut app, 120, 30).join("\n");
            assert!(
                list.contains("Description"),
                "{name} list: no table:\n{list}"
            );
            assert!(
                !list.contains("tracked over"),
                "{name} list: a heat block came with it:\n{list}"
            );

            app.heat_view = true;
            let heat = frame_lines(&mut app, 120, 30).join("\n");
            assert!(
                heat.contains("tracked over"),
                "{name} heat: no heat block:\n{heat}"
            );
            assert!(
                !heat.contains("Description"),
                "{name} heat: the table came with it:\n{heat}"
            );
        }

        // The Year view keeps its own two-dimensional grid rather than a row.
        app.view_mode = ViewMode::Year;
        let year = frame_lines(&mut app, 120, 30).join("\n");
        assert!(
            year.contains("Mon") && year.contains("Sun"),
            "the year heat lost its weekday bands:\n{year}"
        );
    }

    /// In heat mode the heat row already carries the per-day shape, so the
    /// side panel that repeats it is dropped.
    #[test]
    fn the_week_view_keeps_its_daily_totals_only_as_a_list() {
        let _guard = env_guard();
        sandbox("content-dispatch-week");
        let today = Local::now().date_naive();
        seed(vec![logged(0, "a", "tt", &[], today, 60)], 1);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Week;

        assert!(
            frame_lines(&mut app, 120, 30)
                .join("\n")
                .contains("Daily Totals")
        );
        app.heat_view = true;
        assert!(
            !frame_lines(&mut app, 120, 30)
                .join("\n")
                .contains("Daily Totals"),
            "the side panel survived into heat mode"
        );
    }

    /// The grid's columns as minutes, summed down its rows, so a test reads
    /// the fold directly.
    fn heat_minutes(app: &App) -> Vec<i64> {
        let grid = app.view_heat_grid(80, 20);
        let band = &grid.bands[0];
        (0..band.columns())
            .map(|column| {
                band.cells
                    .iter()
                    .filter_map(|row| row[column])
                    .fold(chrono::Duration::zero(), |acc, held| acc + held)
                    .num_minutes()
            })
            .collect()
    }

    /// The day's own 24 hours, with an entry in the hour it ran.
    #[test]
    fn a_day_view_folds_its_own_twenty_four_hours() {
        let _guard = env_guard();
        sandbox("heat-grid-day-fold");
        let today = Local::now().date_naive();
        seed(vec![logged(0, "a", "tt", &[], today, 45)], 1);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;

        let grid = app.view_heat_grid(80, 20);
        assert_eq!(grid.bands[0].cell_span, chrono::Duration::hours(1));
        let minutes = heat_minutes(&app);
        assert_eq!(minutes.len(), 24);
        // `logged` starts at 09:00.
        assert_eq!(minutes[9], 45);
        assert_eq!(minutes.iter().sum::<i64>(), 45, "time leaked into an hour");
    }

    #[test]
    fn a_month_view_folds_one_bucket_per_day_of_its_month() {
        let _guard = env_guard();
        sandbox("heat-buckets-month");
        let february = NaiveDate::from_ymd_opt(2026, 2, 10).unwrap();
        seed(vec![logged(0, "a", "tt", &[], february, 60)], 1);
        let mut app = App::new().unwrap();
        app.selected_date = february;
        app.view_mode = ViewMode::Month;

        let minutes = heat_minutes(&app);
        assert_eq!(minutes.len(), 28, "February 2026 has 28 days");
        assert_eq!(minutes[9], 60, "the 10th is column 9");
    }

    /// The content heat folds what the table walks, so a pane filter moves it —
    /// unlike the Summary's strips, which the scope alone drives.
    #[test]
    fn a_project_filter_changes_the_content_heat_totals() {
        let _guard = env_guard();
        sandbox("heat-buckets-filtered");
        let today = Local::now().date_naive();
        seed(
            vec![
                logged(0, "a", "tt", &[], today, 60),
                logged(1, "b", "other", &[], today, 30),
            ],
            2,
        );
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        assert_eq!(heat_minutes(&app).iter().sum::<i64>(), 90);

        app.project_filter.cycle("tt", true);
        assert!(!app.summary_follows_filters, "the scope-only default holds");
        assert_eq!(
            heat_minutes(&app).iter().sum::<i64>(),
            60,
            "the heat folded the scope instead of the filtered entries"
        );
    }

    /// The heat-strip cells inside the drawn Summary box, as `(x, y, colour)`.
    /// `frame_lines` collects symbols alone, so a strip of blank coloured cells
    /// is invisible to it; this reads the background the cell really carries.
    fn summary_heat_cells(
        app: &mut App,
        width: u16,
        height: u16,
    ) -> Vec<(u16, u16, ratatui::style::Color)> {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render::ui(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let palette: Vec<_> = [(0, 1), (1, 4), (1, 2), (3, 4), (1, 1)]
            .into_iter()
            .map(|(part, max)| super::theme::heat_shade(part, max))
            .collect();
        let row_text = |y: u16| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        let top = (0..height)
            .find(|y| row_text(*y).contains("Summary (S)"))
            .expect("no Summary box");
        let bottom = (top + 1..height)
            .find(|y| row_text(*y).contains('\u{2518}'))
            .expect("no Summary foot");
        (top + 1..bottom)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|(x, y)| palette.contains(&buffer[(*x, *y)].bg))
            .map(|(x, y)| (x, y, buffer[(x, y)].bg))
            .collect()
    }

    /// Four projects, each busy on its own weekday. Each strip must ride its
    /// own project's row: one strip per row, the busy cell moving one place
    /// right as the rows go down, and no cell shared between two rows.
    #[test]
    fn every_project_gets_its_own_strip_on_its_own_row() {
        let _guard = env_guard();
        sandbox("summary-strip-alignment");
        let week = TimeData::week_start(Local::now().date_naive());
        // The rows tie on time, so the name orders them: the names are chosen
        // to sort in the same order as the weekdays they are busy on.
        let names = ["aa", "bb", "cc", "dd"];
        let entries: Vec<TimeEntry> = names
            .iter()
            .enumerate()
            .map(|(day, project)| {
                logged(
                    day as u64,
                    "x",
                    project,
                    &[],
                    week + chrono::Duration::days(day as i64),
                    60,
                )
            })
            .collect();
        seed(entries, 4);
        let mut app = App::new().unwrap();
        app.selected_date = week;
        app.view_mode = ViewMode::Week;
        app.toggle_summary();
        app.summary_heat = true;

        for width in [60u16, 90, 120, 200] {
            let cells = summary_heat_cells(&mut app, width, 40);
            let mut lines: Vec<(u16, Vec<u16>)> = Vec::new();
            for (x, y, _) in cells {
                match lines.iter_mut().find(|(row, _)| *row == y) {
                    Some((_, columns)) => columns.push(x),
                    None => lines.push((y, vec![x])),
                }
            }
            lines.sort_by_key(|(y, _)| *y);
            assert_eq!(
                lines.len(),
                names.len(),
                "width {width}: {} rows carry a strip, not {}",
                lines.len(),
                names.len()
            );
            // Each row sits one line under the last, as its table row does.
            for pair in lines.windows(2) {
                assert_eq!(
                    pair[1].0,
                    pair[0].0 + 1,
                    "width {width}: the strips left their own rows"
                );
            }
            let mut last_start = None;
            let mut cell_width = None;
            for (index, (_, columns)) in lines.iter().enumerate() {
                let mut columns = columns.clone();
                columns.sort_unstable();
                let start = columns[0];
                let run = *columns.last().unwrap() - start + 1;
                assert_eq!(
                    run as usize,
                    columns.len(),
                    "width {width}: {} holds a broken run of cells",
                    names[index]
                );
                assert_eq!(
                    *cell_width.get_or_insert(columns.len()),
                    columns.len(),
                    "width {width}: {} draws a cell of its own size",
                    names[index]
                );
                if let Some(last) = last_start {
                    assert!(
                        start > last,
                        "width {width}: {} did not start right of the row above",
                        names[index]
                    );
                }
                last_start = Some(start);
            }
        }
    }

    /// The strip rides the project's own row, right of `share`, and the ticks
    /// head it in the same columns. Only worked buckets carry colour.
    #[test]
    fn the_expanded_summary_draws_a_heat_strip_right_of_the_share_column() {
        let _guard = env_guard();
        sandbox("summary-strip-draw");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        app.toggle_summary();
        app.summary_heat = true;

        let box_lines = summary_box(&mut app, 120, 40);
        // header, four projects, rule, total: the strips cost no line.
        assert_eq!(box_lines.len(), 7, "{box_lines:#?}");
        let share_end = box_lines[1]
            .chars()
            .position(|c| c == '%')
            .expect("no share on the row") as u16;
        // Two blanks, the rule of the strip's own column, two blanks.
        assert!(
            box_lines[0].contains("share  \u{2502}  Mon"),
            "the ticks do not head the strip past its separator: {}",
            box_lines[0]
        );
        assert!(
            box_lines[1].contains("\u{2502}"),
            "the project row lost the separator: {}",
            box_lines[1]
        );

        let screen = frame_lines(&mut app, 120, 40);
        let top = screen
            .iter()
            .position(|line| line.contains("Summary (S)"))
            .unwrap() as u16;
        let cells = summary_heat_cells(&mut app, 120, 40);
        assert!(!cells.is_empty(), "no strip:\n{}", box_lines.join("\n"));
        assert!(
            cells.iter().all(|(x, _, _)| *x > share_end),
            "a strip cell sits on the share column"
        );
        // The rows run from the line under the header to the rule above the
        // total, so nothing is painted on the header, the rule or the total.
        assert!(
            cells.iter().all(|(_, y, _)| *y > top + 1 && *y <= top + 5),
            "a heat cell fell outside the project rows"
        );
        // vinge's Tuesday is the largest bucket, so it alone is hottest.
        let hottest: Vec<u16> = cells
            .iter()
            .filter(|(_, _, color)| *color == super::theme::heat_shade(1, 1))
            .map(|(_, y, _)| *y)
            .collect();
        assert!(!hottest.is_empty(), "nothing carries the grid maximum");
        assert!(
            hottest.iter().all(|y| *y == top + 2),
            "the hottest cells are not on the busiest project's row"
        );
        assert!(
            box_lines.iter().all(|line| line.ends_with('\u{2502}')),
            "a strip ran over the right border:\n{}",
            box_lines.join("\n")
        );
    }

    /// An empty bucket keeps the surface behind it: only worked time is drawn.
    #[test]
    fn an_empty_bucket_carries_no_colour_of_its_own() {
        let _guard = env_guard();
        sandbox("summary-strip-empty");
        let today = Local::now().date_naive();
        seed(vec![logged(0, "one hour", "tt", &[], today, 60)], 1);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.toggle_summary();
        app.summary_heat = true;

        let cells = summary_heat_cells(&mut app, 120, 40);
        let rows = app.project_summary();
        let (cell_width, _) = summary::strip_cells(
            // the free width the one row leaves right of its separator
            120 - 2 - 1 - "(no project)".len().max(7) - 21 - 5,
            app.project_buckets(&rows).len(),
        );
        assert_eq!(
            cells.len(),
            cell_width,
            "the empty hours of the day were painted too"
        );
    }

    /// Every view heads its strip with the ticks of the period it covers.
    #[test]
    fn the_strip_ticks_head_the_period_each_view_covers() {
        let _guard = env_guard();
        sandbox("summary-strip-axis");
        let mut app = seed_summary();
        app.toggle_summary();
        app.summary_heat = true;

        for (mode, ticks) in [
            (ViewMode::Day, vec!["00", "06", "12", "18"]),
            (ViewMode::Week, vec!["Mon", "Sun"]),
            (ViewMode::Year, vec!["Jan", "Dec"]),
        ] {
            app.view_mode = mode;
            let header = summary_box(&mut app, 120, 40)[0].clone();
            for tick in ticks {
                assert!(header.contains(tick), "no `{tick}` on the header: {header}");
            }
        }

        // A month is a day axis too long for weekday names: sparse dates instead.
        app.view_mode = ViewMode::Month;
        let header = summary_box(&mut app, 120, 40)[0].clone();
        let axis = header
            .split_once('\u{2502}')
            .map(|(_, axis)| axis.to_string())
            .expect("no strip separator on the header");
        for date in ["1", "8", "15", "22", "29"] {
            assert!(axis.contains(date), "no `{date}` on the axis: {axis}");
        }
        assert!(
            !axis.contains("Mon") && !axis.contains("Tue"),
            "a month of weekday names: {axis}"
        );
    }

    /// The collapsed box is the total line alone; no strip comes with it, even
    /// with the strips asked for.
    #[test]
    fn a_collapsed_summary_draws_no_heat_strip() {
        let _guard = env_guard();
        sandbox("summary-strip-collapsed");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        app.summary_heat = true;

        assert!(summary_heat_cells(&mut app, 120, 40).is_empty());
    }

    /// The strips are opt-in: `m` is what asks for them.
    #[test]
    fn m_shows_and_hides_the_summary_heat_strips() {
        let _guard = env_guard();
        sandbox("summary-strip-toggle");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        app.toggle_summary();

        assert!(
            summary_heat_cells(&mut app, 120, 40).is_empty(),
            "the strips drew without being asked for"
        );
        app.toggle_summary_heat();
        assert!(
            !summary_heat_cells(&mut app, 120, 40).is_empty(),
            "`m` did not bring the strips"
        );
        app.toggle_summary_heat();
        assert!(
            summary_heat_cells(&mut app, 120, 40).is_empty(),
            "`m` did not take the strips away again"
        );
    }

    /// Too narrow for a strip worth reading, the row drops it whole rather
    /// than drawing a stub, and no row runs over the border.
    #[test]
    fn a_narrow_summary_sheds_the_heat_strip_whole() {
        let _guard = env_guard();
        sandbox("summary-strip-shed");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        app.toggle_summary();
        app.toggle_summary_split();
        app.summary_heat = true;

        assert!(
            summary_heat_cells(&mut app, 60, 40).is_empty(),
            "a stub strip"
        );
        let box_lines = summary_box(&mut app, 60, 40);
        assert!(
            box_lines.iter().all(|line| line.ends_with('\u{2502}')),
            "a row ran over the right border:\n{}",
            box_lines.join("\n")
        );
    }

    /// The columns the strip sits beside are unchanged in either split mode.
    #[test]
    fn the_heat_strip_leaves_the_number_columns_alone() {
        let _guard = env_guard();
        sandbox("summary-strip-columns");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Week;
        app.toggle_summary();
        app.summary_heat = true;

        for split in [false, true] {
            if split {
                app.toggle_summary_split();
            }
            let box_lines = summary_box(&mut app, 140, 40);
            assert!(box_lines[0].contains("count share"), "{}", box_lines[0]);
            assert!(box_lines[1].contains("2h 0m"), "{}", box_lines[1]);
            assert!(
                box_lines[1].contains("32%"),
                "split={split}: {}",
                box_lines[1]
            );
            assert!(
                !summary_heat_cells(&mut app, 140, 40).is_empty(),
                "split={split}: no strip"
            );
        }
    }

    /// `nothing in scope` keeps the box it has: no header over an empty surface.
    #[test]
    fn an_empty_summary_scope_gets_no_header_row() {
        let _guard = env_guard();
        sandbox("summary-empty-header");
        seed(Vec::new(), 0);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.toggle_summary();

        assert_eq!(app.summary_surface_height(), 3);
        let empty = summary_box(&mut app, 100, 40);
        assert!(
            empty[0].starts_with("\u{2502} nothing in scope"),
            "the empty box grew a header: {}",
            empty[0]
        );
    }

    /// The marker's count and the rows drawn must agree once the header takes a row.
    #[test]
    fn the_summary_marker_counts_the_rows_it_actually_draws() {
        let _guard = env_guard();
        sandbox("summary-header-budget");
        let today = Local::now().date_naive();
        let entries: Vec<TimeEntry> = (0..9)
            .map(|n| logged(n, "x", &format!("p{n}"), &[], today, 30 + n as i64))
            .collect();
        seed(entries, 9);
        let mut app = App::new().unwrap();
        app.selected_date = today;
        app.view_mode = ViewMode::Day;
        app.toggle_summary();

        // Two borders, the header, the six-project cap, the rule and the total.
        assert_eq!(app.summary_surface_height(), 11);
        let drawn = summary_box(&mut app, 100, 40);
        assert_eq!(drawn.len(), 9, "header, rows, rule and total: {drawn:#?}");
        let screen = frame_lines(&mut app, 100, 40);
        let title = screen
            .iter()
            .find(|line| line.contains("Summary (S)"))
            .unwrap()
            .clone();
        assert!(title.contains("6/9"), "the marker miscounted: {title}");
    }

    /// Collapsed, the surface is the total row between its borders; `S` opens
    /// the breakdown above it.
    #[test]
    fn the_summary_surface_is_three_rows_until_it_is_toggled_on() {
        let _guard = env_guard();
        sandbox("summary-height");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Day;

        assert!(!app.show_summary);
        assert_eq!(app.summary_surface_height(), 3, "collapsed: the total row");

        // Two borders, the header, the rule, the total, and one row per project:
        // the day has three. The strips ride the rows, so they cost no line.
        app.toggle_summary();
        assert_eq!(app.summary_surface_height(), 8);
        // Re-scoping re-sizes it: the week has four projects, all entries too.
        app.view_mode = ViewMode::Week;
        assert_eq!(app.summary_surface_height(), 9);

        // An empty scope still gets one row, so the box can say it is empty.
        app.view_mode = ViewMode::Day;
        app.selected_date = Local::now().date_naive() - chrono::Duration::days(400);
        assert!(app.project_summary().is_empty());
        assert_eq!(app.summary_surface_height(), 3);

        app.toggle_summary();
        assert_eq!(app.summary_surface_height(), 3, "collapsed again");
    }

    /// Focus never reads as resting on a hidden surface, however `focus` was set.
    #[test]
    fn the_summary_reads_as_focused_only_while_it_is_visible() {
        let _guard = env_guard();
        sandbox("summary-is-focused");
        let mut app = seed_summary();
        assert!(!app.summary_is_focused());

        app.focus = Focus::Summary;
        assert!(!app.summary_is_focused(), "hidden: focus does not count");

        app.toggle_summary();
        assert!(app.summary_is_focused());

        app.focus = Focus::Table;
        assert!(!app.summary_is_focused(), "visible but not focused");
    }

    /// The row carrying the panes' bottom borders. Side by side, both boxes
    /// close on the same row.
    fn pane_bottom_border(app: &mut App, width: u16, height: u16) -> String {
        let screen = frame_lines(app, width, height);
        let top = screen
            .iter()
            .position(|line| line.contains("Projects (P)"))
            .unwrap_or_else(|| panic!("no Projects pane:\n{}", screen.join("\n")));
        screen[top..]
            .iter()
            .find(|line| line.contains('\u{2518}'))
            .cloned()
            .unwrap_or_else(|| panic!("no bottom border:\n{}", screen.join("\n")))
    }

    /// Both panes open, so the equal-ratio split decides each legend's width.
    fn seed_both_panes() -> App {
        let mut app = seed_panes();
        app.toggle_pane(Pane::Projects);
        app.toggle_pane(Pane::Tags);
        assert_eq!(app.visible_panes().len(), 2, "both panes must be open");
        app
    }

    /// Each pane names its own keys, so `Enter` and `-` stop being invisible.
    #[test]
    fn both_panes_name_their_keys_on_their_bottom_border() {
        let _guard = env_guard();
        sandbox("pane-legend");
        let mut app = seed_both_panes();

        let border = pane_bottom_border(&mut app, 140, 40);
        assert_eq!(
            border
                .matches(" Enter: filter \u{b7} -: back \u{b7} j/k: move ")
                .count(),
            2,
            "one legend per pane: {border}"
        );
        assert!(
            border.starts_with("\u{2514} Enter: filter"),
            "a key legend sits on the left of its border: {border}"
        );

        // 80 columns split in two leave 38 inner cells; the legend is 37.
        let border = pane_bottom_border(&mut app, 80, 40);
        assert!(
            border.contains(" Enter: filter \u{b7} -: back \u{b7} j/k: move "),
            "the legend must survive a split pane at 80 columns: {border}"
        );
    }

    /// Narrowing sheds the least useful key first, then the whole legend, and
    /// never leaves a fragment that would name a key wrong.
    #[test]
    fn a_narrow_pane_sheds_the_move_key_and_then_the_whole_legend() {
        let _guard = env_guard();
        sandbox("pane-legend-narrow");
        let mut app = seed_both_panes();

        let border = pane_bottom_border(&mut app, 76, 40);
        assert!(
            border.contains(" Enter: filter \u{b7} -: back "),
            "the head of the legend must stay: {border}"
        );
        assert!(!border.contains("j/k"), "`j/k` must shed first: {border}");

        let border = pane_bottom_border(&mut app, 32, 40);
        assert!(!border.contains("Enter"), "no legend at all: {border}");
        assert!(
            !border.contains(':'),
            "a clipped legend was drawn: {border}"
        );
    }

    /// The pane has no mode of its own, so focus is the whole accent rule.
    #[test]
    fn the_pane_legend_accents_only_the_focused_pane() {
        let _guard = env_guard();
        sandbox("pane-legend-colour");
        let mut app = seed_both_panes();
        app.focus = Focus::Pane(Pane::Projects);

        let keys = drawn_fg(&mut app, "Enter: filter", 140, 40);
        assert_eq!(keys, vec![theme::accent(), theme::inactive()]);

        app.focus = Focus::Pane(Pane::Tags);
        let keys = drawn_fg(&mut app, "Enter: filter", 140, 40);
        assert_eq!(keys, vec![theme::inactive(), theme::accent()]);
    }

    /// The Summary box's bottom border, where its key legend sits.
    fn summary_bottom_border(app: &mut App, width: u16, height: u16) -> String {
        let screen = frame_lines(app, width, height);
        let top = screen
            .iter()
            .position(|line| line.contains("Summary (S)"))
            .unwrap_or_else(|| panic!("no Summary box:\n{}", screen.join("\n")));
        screen[top..]
            .iter()
            .find(|line| line.contains('\u{2518}'))
            .cloned()
            .unwrap_or_else(|| panic!("no bottom border:\n{}", screen.join("\n")))
    }

    /// The foreground of the first cell of every drawn `needle`, in reading
    /// order: two side-by-side panes put their legends on the same row.
    fn drawn_fg(
        app: &mut App,
        needle: &str,
        width: u16,
        height: u16,
    ) -> Vec<ratatui::style::Color> {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render::ui(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut found = Vec::new();
        for y in 0..height {
            let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            for (byte, _) in row.match_indices(needle) {
                let column = row[..byte].chars().count() as u16;
                found.push(buffer[(column, y)].fg);
            }
        }
        assert!(!found.is_empty(), "nothing drawn carries {needle}");
        found
    }

    /// The keys the Summary owns are named on its own border, not only in the
    /// help popup, and the border keeps its corner.
    #[test]
    fn the_summary_names_its_keys_on_its_bottom_border() {
        let _guard = env_guard();
        sandbox("summary-legend");
        let mut app = seed_summary();
        app.toggle_summary();

        let border = summary_bottom_border(&mut app, 100, 40);
        assert!(
            border.starts_with("\u{2514} v: split \u{b7} f: filter \u{b7} m: heat "),
            "the key legend is not on the left of its border: {border}"
        );
        assert!(
            border.ends_with('\u{2518}'),
            "the legend ate the corner: {border}"
        );
    }

    /// The bottom border of the box opened by `title`, whichever box it is.
    fn bottom_border(app: &mut App, title: &str, width: u16, height: u16) -> String {
        let screen = frame_lines(app, width, height);
        let top = screen
            .iter()
            .position(|line| line.contains(title))
            .unwrap_or_else(|| panic!("no `{title}` box:\n{}", screen.join("\n")));
        screen[top..]
            .iter()
            .find(|line| line.contains('\u{2518}'))
            .cloned()
            .unwrap_or_else(|| panic!("no bottom border:\n{}", screen.join("\n")))
    }

    /// The toggle is discoverable where it acts, in both representations and
    /// in every view.
    #[test]
    fn the_content_box_names_the_m_toggle_on_its_bottom_border() {
        let _guard = env_guard();
        sandbox("content-legend-render");
        let today = Local::now().date_naive();
        seed(vec![logged(0, "a", "tt", &[], today, 60)], 1);
        let mut app = App::new().unwrap();
        app.selected_date = today;

        for (view, name) in views() {
            app.view_mode = view;

            app.heat_view = false;
            // The column header, not the title: `All Entries` names the tabs row too.
            let border = bottom_border(&mut app, "Description", 120, 40);
            assert!(
                border.contains("\u{2514} M: heatmap"),
                "{name} list: no legend on the content border: {border}"
            );

            app.heat_view = true;
            let border = bottom_border(&mut app, "tracked over", 120, 40);
            assert!(
                border.contains("\u{2514} M: list"),
                "{name} heat: no legend on the content border: {border}"
            );
        }
    }

    /// One convention across the whole TUI: keys on the left, ramps on the
    /// right, so the two never fight for the same corner.
    #[test]
    fn every_colour_ramp_sits_on_the_right_of_its_border() {
        let _guard = env_guard();
        sandbox("legend-sides");
        let mut app = seed_summary();
        app.toggle_summary();
        app.heat_view = true;
        app.summary_heat = true;

        let ramp_is_right = |border: &str, what: &str| {
            let middle = border.chars().count() / 2;
            let less = border.find("Less").unwrap_or_else(|| {
                panic!("no ramp on the {what} border: {border}");
            });
            assert!(
                less > middle,
                "the {what} ramp is not on the right: {border}"
            );
        };

        for (view, title) in [
            (ViewMode::Day, "tracked over"),
            (ViewMode::Year, "tracked over"),
        ] {
            app.view_mode = view;
            ramp_is_right(&bottom_border(&mut app, title, 120, 40), "heat block");
        }

        // The Summary's border carries both, so it proves they share a border.
        app.view_mode = ViewMode::Week;
        let border = bottom_border(&mut app, "Summary (S)", 120, 40);
        ramp_is_right(&border, "summary strip");
        assert!(
            border.starts_with("\u{2514} v: split"),
            "the keys left the left of the border: {border}"
        );
    }

    /// Focus accents both keys; away from focus, a key stays accented only
    /// while its own mode is on — the rule the footer's `S` follows.
    #[test]
    fn the_summary_legend_accents_a_key_while_its_own_mode_is_on() {
        let _guard = env_guard();
        sandbox("summary-legend-colour");
        let mut app = seed_summary();
        app.toggle_summary();
        app.focus = Focus::Table;

        assert_eq!(
            drawn_fg(&mut app, "v: split", 100, 40)[0],
            theme::inactive(),
            "unfocused with the split off"
        );

        app.toggle_summary_split();
        assert_eq!(
            drawn_fg(&mut app, "v: split", 100, 40)[0],
            theme::accent(),
            "the split is on"
        );
        assert_eq!(
            drawn_fg(&mut app, "f: filter", 100, 40)[0],
            theme::inactive(),
            "`f` follows its own mode, not `v`"
        );

        app.toggle_summary_follows_filters();
        assert_eq!(drawn_fg(&mut app, "f: filter", 100, 40)[0], theme::accent());

        app.toggle_summary_split();
        app.toggle_summary_follows_filters();
        app.focus = Focus::Summary;
        assert_eq!(
            drawn_fg(&mut app, "v: split", 100, 40)[0],
            theme::accent(),
            "focus alone accents every key"
        );
        assert_eq!(drawn_fg(&mut app, "f: filter", 100, 40)[0], theme::accent());
    }

    #[test]
    fn s_focuses_the_summary_it_opens_and_falls_back_when_it_hides() {
        let _guard = env_guard();
        sandbox("summary-focus");
        let mut app = seed_summary();
        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        app.table_state.select(Some(1));

        app.toggle_summary();
        assert!(app.show_summary);
        assert_eq!(app.focus, Focus::Summary);
        assert_eq!(app.table_state.selected(), Some(1));

        app.toggle_summary();
        assert!(!app.show_summary);
        assert_eq!(app.focus, Focus::Pane(Pane::Projects), "back to the pane");
        assert_eq!(app.table_state.selected(), Some(1));

        // Hiding a pane never jumps focus to the Summary.
        app.toggle_summary();
        app.focus = Focus::Pane(Pane::Projects);
        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Table);

        let mut app = seed_summary();
        app.toggle_summary();
        assert_eq!(app.focus, Focus::Summary);
        app.toggle_summary();
        assert_eq!(app.focus, Focus::Table, "no pane to fall back to");
    }

    #[test]
    fn the_title_marker_says_all_projects_in_scope_only_mode() {
        let _guard = env_guard();
        sandbox("summary-marker");
        let mut app = seed_summary();
        app.toggle_summary();

        for (mode, word) in scopes() {
            app.set_view_mode(mode);
            let rows = app.project_summary();
            assert_eq!(
                app.summary_marker(&rows, 6),
                format!("{word} · all projects")
            );
        }

        // A filter changes the emphasis, never the words.
        app.set_view_mode(ViewMode::Day);
        assert!(!app.total_is_filtered());
        let rows = app.project_summary();
        let unfiltered = app.summary_marker(&rows, 6);
        app.project_filter.cycle("tt", true);
        assert!(app.total_is_filtered(), "the footer total is now narrowed");
        assert_eq!(app.summary_marker(&rows, 6), unfiltered);

        // The split says so last, after the scope and any overflow count.
        app.toggle_summary_split();
        assert_eq!(
            app.summary_marker(&rows, 6),
            format!("{unfiltered} \u{b7} split")
        );
        assert_eq!(
            app.summary_marker(&rows, 1),
            "day \u{b7} all projects \u{b7} 1/3 \u{b7} split"
        );
        app.toggle_summary_split();
        assert_eq!(app.summary_marker(&rows, 6), unfiltered);
    }

    #[test]
    fn the_title_marker_says_filtered_while_the_summary_follows() {
        let _guard = env_guard();
        sandbox("summary-marker-follow");
        let mut app = seed_summary();
        app.toggle_summary();
        app.toggle_summary_follows_filters();

        for (mode, word) in scopes() {
            app.set_view_mode(mode);
            let rows = app.project_summary();
            assert_eq!(
                app.summary_marker(&rows, 6),
                format!("{word} \u{b7} filtered")
            );
        }

        // The word says the mode, so an unset filter does not take it away.
        app.set_view_mode(ViewMode::Day);
        assert!(!app.total_is_filtered());
        let rows = app.project_summary();
        assert_eq!(app.summary_marker(&rows, 6), "day \u{b7} filtered");

        // The count and the split keep their places after the word.
        app.toggle_summary_split();
        assert_eq!(
            app.summary_marker(&rows, 1),
            "day \u{b7} filtered \u{b7} 1/3 \u{b7} split"
        );

        app.toggle_summary_follows_filters();
        assert_eq!(
            app.summary_marker(&rows, 1),
            "day \u{b7} all projects \u{b7} 1/3 \u{b7} split"
        );
    }

    /// The empty box blames a filter only when one is set and the mode reads it.
    #[test]
    fn a_filtered_to_empty_summary_says_the_filter_emptied_it() {
        let _guard = env_guard();
        sandbox("summary-empty-follow");
        let mut app = seed_summary();
        app.view_mode = ViewMode::Day;
        app.toggle_summary();
        app.toggle_summary_follows_filters();

        // Three projects in the day, each row plus header, rule, total, two borders.
        assert_eq!(app.summary_surface_height(), 8);

        // A filter that removes projects shrinks the box.
        app.project_filter.cycle("tt", true);
        assert_eq!(app.summary_surface_height(), 6);
        let one = summary_box(&mut app, 100, 40);
        assert!(one[1].contains("tt"), "{one:#?}");

        // A filter that removes every entry leaves the 3-row empty box.
        app.project_filter.clear();
        app.search_term.set_from("nothing matches this");
        assert_eq!(app.summary_surface_height(), 3);
        let empty = summary_box(&mut app, 100, 40);
        assert!(
            empty[0].starts_with("\u{2502} nothing matches the filter"),
            "the empty box does not blame the filter: {}",
            empty[0]
        );

        // Scope-only mode never empties, however the filter is set.
        app.toggle_summary_follows_filters();
        assert_eq!(app.summary_surface_height(), 8);

        // Following with nothing set says the scope is empty, not the filter.
        app.search_term.clear();
        app.toggle_summary_follows_filters();
        app.selected_date -= chrono::Duration::days(400);
        assert!(!app.total_is_filtered());
        let bare = summary_box(&mut app, 100, 40);
        assert!(
            bare[0].starts_with("\u{2502} nothing in scope"),
            "an unfiltered empty scope blamed a filter: {}",
            bare[0]
        );

        // Scope-only mode never blames the filter, even with one set.
        app.toggle_summary_follows_filters();
        app.project_filter.cycle("tt", true);
        assert!(app.total_is_filtered());
        let scoped = summary_box(&mut app, 100, 40);
        assert!(
            scoped[0].starts_with("\u{2502} nothing in scope"),
            "scope-only mode blamed the filter: {}",
            scoped[0]
        );
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
        assert_eq!(app.summary_surface_height(), 11);
        let rows = app.project_summary();
        assert_eq!(summary::summary_count(&rows, 6).as_deref(), Some("6/9"));
        assert_eq!(app.summary_marker(&rows, 6), "day · all projects · 6/9");
        assert_eq!(summary::visible_project_summary(&rows, 6).len(), 6);

        // A shorter box counts what *it* left out, not what the cap would have.
        assert_eq!(app.summary_marker(&rows, 2), "day · all projects · 2/9");
        assert_eq!(summary::visible_project_summary(&rows, 2).len(), 2);

        assert_eq!(summary::summary_count(&rows, 9), None);
        assert_eq!(app.summary_marker(&rows, 9), "day · all projects");
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

        app.project_filter.cycle("tt", true);
        assert!(app.total_is_filtered(), "the marker is now emphasised");
        assert!(
            app.filtered_total() < summed,
            "the footer total should have dropped below the summary's"
        );
        assert_eq!(app.project_summary(), rows, "the summary must not move");

        // Following, the summary tracks the footer instead of staying put.
        app.toggle_summary_follows_filters();
        let followed = app
            .project_summary()
            .iter()
            .fold(chrono::Duration::zero(), |acc, row| acc + row.total);
        assert_eq!(followed, app.filtered_total());
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
                    data: None,
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
            app.search_term.set_from(needle);
            let view = in_view(&app);
            assert!(
                view.contains(&"wrote the migration".to_string()),
                "search {needle:?} missed the entry: {view:?}"
            );
        }

        // The date matches both entries; every other needle above is unique to one.
        app.search_term.set_from("loremind");
        assert_eq!(in_view(&app), vec!["wrote the migration"]);
        app.search_term.set_from("no such thing");
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
        assert_eq!(app.input_project.value(), "acme");

        app.input_project.set_from("beta");
        app.submit_edit().unwrap();
        assert_eq!(on_disk().entries[0].project, Some("beta".to_string()));

        select(&mut app, "has a project");
        app.start_editing();
        app.input_project.clear();
        app.submit_edit().unwrap();
        assert_eq!(on_disk().entries[0].project, None);
    }
    // --- derived-view cache invalidation -------------------------------------
    //
    // `filtered_entries` and `pane_values` are cached against a key holding every
    // input they read (see `tui::cache`). The tests below warm both caches, cross
    // one mutation boundary, and assert the derived view moved with it — a cache
    // that failed to notice that boundary goes stale here.

    /// Fill both caches, so the assertion after a mutation is a real re-read.
    fn warm(app: &App) {
        let _ = app.filtered_entries();
        let _ = app.pane_values(Pane::Projects);
        let _ = app.pane_values(Pane::Tags);
    }

    /// The rows in view by description, name-sorted so the assertion does not
    /// also pin the sort order.
    fn in_view_sorted(app: &App) -> Vec<String> {
        let mut rows = in_view(app);
        rows.sort();
        rows
    }

    /// `mutate_store` — add, edit and delete all replace `data` through it.
    #[test]
    fn the_derived_views_follow_a_store_mutation() {
        let _guard = env_guard();
        sandbox("cache-mutate");
        seed(vec![entry(0, "first")], 1);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::All;
        warm(&app);

        app.mutate_store(|data| {
            data.add_entry(
                "second".to_string(),
                Some("acme".to_string()),
                vec!["new".to_string()],
                Local::now(),
                Some(Local::now()),
            );
        })
        .unwrap();
        assert_eq!(in_view_sorted(&app), vec!["first", "second"]);
        assert_eq!(values(&app, Pane::Projects), "acme=1");
        assert_eq!(values(&app, Pane::Tags), "new=1");

        warm(&app);
        app.mutate_store(|data| data.entries[0].description = "renamed".to_string())
            .unwrap();
        assert_eq!(in_view_sorted(&app), vec!["renamed", "second"]);

        warm(&app);
        app.delete_entry(0).unwrap();
        assert_eq!(in_view_sorted(&app), vec!["second"]);
        assert_eq!(values(&app, Pane::Projects), "acme=1");
    }

    /// `set_view_mode`, `previous_period`/`next_period` and `go_to_today`.
    #[test]
    fn the_derived_views_follow_the_view_mode_and_period() {
        let _guard = env_guard();
        sandbox("cache-scope");
        let mut app = seed_panes();

        app.set_view_mode(ViewMode::Day);
        assert_eq!(in_view_sorted(&app), vec!["a", "b", "c", "f"]);

        warm(&app);
        app.set_view_mode(ViewMode::Week);
        assert_eq!(in_view_sorted(&app), vec!["a", "b", "c", "d", "f"]);
        assert_eq!(values(&app, Pane::Projects), "tt=2 loremind=1 vinge=1");

        warm(&app);
        app.previous_period();
        assert_eq!(in_view_sorted(&app), vec!["e"]);
        assert_eq!(values(&app, Pane::Projects), "vinge=1");

        warm(&app);
        app.next_period();
        assert_eq!(in_view_sorted(&app), vec!["a", "b", "c", "d", "f"]);

        warm(&app);
        app.previous_period();
        app.go_to_today();
        assert_eq!(in_view_sorted(&app), vec!["a", "b", "c", "d", "f"]);
    }

    /// `toggle_sort_order` — same rows, reversed.
    #[test]
    fn the_entries_view_follows_the_sort_order() {
        let _guard = env_guard();
        sandbox("cache-sort");
        let mut app = seed_panes();
        app.view_mode = ViewMode::All;
        let newest_first = in_view(&app);
        warm(&app);

        app.toggle_sort_order();
        let oldest_first = in_view(&app);
        // Same rows, ends swapped. Not an exact reverse: same-second ties keep
        // their order under a stable sort.
        assert_ne!(newest_first, oldest_first);
        assert_eq!(oldest_first.first(), newest_first.last());
        assert_eq!(oldest_first.last(), newest_first.first());
    }

    /// `cycle_pane_value` and `clear_filters`, which move the rows but never the
    /// pane values — those are offered before any filter.
    #[test]
    fn the_entries_view_follows_a_pane_filter() {
        let _guard = env_guard();
        sandbox("cache-filter");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        warm(&app);

        point_at(&mut app, Pane::Projects, "tt");
        app.cycle_pane_value(true);
        assert_eq!(in_view_sorted(&app), vec!["a", "b"]);
        assert_eq!(values(&app, Pane::Projects), "tt=2 loremind=1");

        warm(&app);
        app.clear_filters();
        assert_eq!(in_view_sorted(&app), vec!["a", "b", "c", "f"]);
    }

    /// `handle_search_char`, `handle_search_backspace` and `clear_search`.
    #[test]
    fn the_entries_view_follows_the_search_term() {
        let _guard = env_guard();
        sandbox("cache-search");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        warm(&app);

        for c in "loremind".chars() {
            app.handle_search_char(c);
        }
        assert_eq!(in_view_sorted(&app), vec!["c"]);

        warm(&app);
        app.handle_search_backspace();
        assert_eq!(in_view_sorted(&app), vec!["c"]);

        warm(&app);
        app.clear_search();
        assert_eq!(in_view_sorted(&app), vec!["a", "b", "c", "f"]);
    }

    /// `sync_from_store` — a write from outside the TUI, which reloads `data`.
    #[test]
    fn the_derived_views_follow_an_outside_write() {
        let _guard = env_guard();
        sandbox("cache-sync");
        seed(vec![entry(0, "first")], 1);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::All;
        warm(&app);

        agent_write("outside");
        app.sync_from_store().unwrap();
        assert_eq!(in_view_sorted(&app), vec!["first", "outside"]);
        assert_eq!(values(&app, Pane::Projects), "probe=1");
        assert_eq!(values(&app, Pane::Tags), "probe=1");
    }
}
