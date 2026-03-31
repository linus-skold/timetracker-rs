use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate};
use std::collections::HashMap;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Stdout};

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs},
};

use crate::duration;
use crate::icons;
use crate::storage::{load_data, save_data};
use crate::tracker::TimeData;

// Color theme
mod theme {
    use ratatui::style::Color;

    pub const ACCENT: Color = Color::Rgb(138, 180, 248);       // Light blue
    pub const ACTIVE: Color = Color::Rgb(129, 199, 132);       // Green
    pub const INACTIVE: Color = Color::Rgb(144, 144, 144);     // Gray
    pub const HEADER_BG: Color = Color::Rgb(48, 48, 48);       // Dark gray
    pub const SELECTED_BG: Color = Color::Rgb(66, 66, 66);     // Medium gray
    pub const HIGHLIGHT: Color = Color::Rgb(255, 213, 79);     // Yellow/gold
    pub const DURATION_HIGH: Color = Color::Rgb(239, 154, 154); // Light red
    pub const DURATION_MED: Color = Color::Rgb(255, 224, 130);  // Light yellow
    pub const DURATION_LOW: Color = Color::Rgb(165, 214, 167);  // Light green
    pub const BORDER: Color = Color::Rgb(88, 88, 88);          // Border gray
    pub const TITLE: Color = Color::Rgb(186, 186, 186);        // Light gray
    pub const DAY_HEADER_BG: Color = Color::Rgb(38, 48, 68);   // Dark blue for day separators
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    All,
    Day,
    Week,
}

impl ViewMode {
    fn title(&self) -> &'static str {
        match self {
            ViewMode::All => "All Entries",
            ViewMode::Day => "Daily View",
            ViewMode::Week => "Weekly View",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Normal,
    AddingEntry,
    EditingEntry,
    Searching,
}

#[derive(Clone, Copy, PartialEq)]
enum InputField {
    Description,
    Tags,
    StartTime,
    EndTime,
    Duration,
}

#[derive(Clone, Copy, PartialEq)]
enum SortOrder {
    NewestFirst,
    OldestFirst,
}

impl SortOrder {
    fn toggle(self) -> Self {
        match self {
            SortOrder::NewestFirst => SortOrder::OldestFirst,
            SortOrder::OldestFirst => SortOrder::NewestFirst,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortOrder::NewestFirst => "newest first",
            SortOrder::OldestFirst => "oldest first",
        }
    }
}

struct App {
    data: TimeData,
    table_state: TableState,
    should_quit: bool,
    view_mode: ViewMode,
    selected_date: NaiveDate,
    // Input mode state
    input_mode: InputMode,
    input_field: InputField,
    input_description: String,
    input_tags: String, // Space-separated tags (without #)
    input_start_time: String, // Optional: start time like "14:30" or "2024-03-16 14:30"
    input_end_time: String,   // Optional: end time like "14:30" or "2024-03-16 14:30"
    input_duration: String,
    // Search state
    search_term: String,
    // Tag filter state
    tag_filter: Vec<String>,
    // Edit state
    editing_entry_id: Option<u64>,
    // Sort order
    sort_order: SortOrder,
}

impl App {
    fn new() -> Result<Self> {
        Ok(Self {
            data: load_data()?,
            table_state: TableState::default().with_selected(Some(0)),
            should_quit: false,
            view_mode: ViewMode::Day,
            selected_date: Local::now().date_naive(),
            input_mode: InputMode::Normal,
            input_field: InputField::Description,
            input_description: String::new(),
            input_tags: String::new(),
            input_start_time: String::new(),
            input_end_time: String::new(),
            input_duration: String::new(),
            search_term: String::new(),
            tag_filter: Vec::new(),
            editing_entry_id: None,
            sort_order: SortOrder::NewestFirst,
        })
    }

    fn reload(&mut self) -> Result<()> {
        self.data = load_data()?;
        Ok(())
    }

    fn filtered_entries(&self) -> Vec<&crate::tracker::TimeEntry> {
        let mut entries: Vec<_> = match self.view_mode {
            ViewMode::All => self.data.entries.iter().collect(),
            ViewMode::Day => self.data.entries_for_date(self.selected_date),
            ViewMode::Week => {
                let week_start = TimeData::week_start(self.selected_date);
                self.data.entries_for_week(week_start)
            }
        };

        // Sort: Day view by start hour; Week view by day then hour; All view by start time.
        // The secondary sort key for Week (day) is implicit in start_time, so sorting by
        // start_time covers all cases uniformly.
        match self.sort_order {
            SortOrder::NewestFirst => entries.sort_by(|a, b| b.start_time.cmp(&a.start_time)),
            SortOrder::OldestFirst => entries.sort_by(|a, b| a.start_time.cmp(&b.start_time)),
        }

        // Apply tag filter first
        let entries = if self.tag_filter.is_empty() {
            entries
        } else {
            entries
                .into_iter()
                .filter(|e| e.has_any_tag(&self.tag_filter))
                .collect()
        };

        // Apply search filter if search term is not empty
        if self.search_term.is_empty() {
            entries
        } else {
            let search_lower = self.search_term.to_lowercase();
            entries
                .into_iter()
                .filter(|e| {
                    e.description.to_lowercase().contains(&search_lower)
                        || e.tags.iter().any(|t| t.to_lowercase().contains(&search_lower))
                })
                .collect()
        }
    }

    fn filtered_total(&self) -> Duration {
        self.filtered_entries()
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    fn is_searching(&self) -> bool {
        !self.search_term.is_empty() || self.input_mode == InputMode::Searching
    }

    fn is_tag_filtering(&self) -> bool {
        !self.tag_filter.is_empty()
    }

    fn toggle_tag_filter(&mut self, tag: &str) {
        let tag = tag.to_string();
        if let Some(pos) = self.tag_filter.iter().position(|t| t == &tag) {
            self.tag_filter.remove(pos);
        } else {
            self.tag_filter.push(tag);
        }
        self.table_state.select(Some(0));
    }

    fn clear_tag_filter(&mut self) {
        self.tag_filter.clear();
        self.table_state.select(Some(0));
    }

    fn filter_by_selected_tags(&mut self) {
        // Get tags from selected entry
        let tags = {
            let filtered = self.filtered_entries();
            self.table_state.selected().and_then(|idx| {
                filtered.get(idx).map(|entry| entry.tags.clone())
            })
        };

        if let Some(tags) = tags {
            if tags.is_empty() {
                return;
            }
            // If already filtering by these exact tags, clear the filter
            if self.tag_filter == tags {
                self.clear_tag_filter();
            } else {
                self.tag_filter = tags;
                self.table_state.select(Some(0));
            }
        }
    }

    fn start_search(&mut self) {
        self.input_mode = InputMode::Searching;
    }

    fn clear_search(&mut self) {
        self.search_term.clear();
        self.input_mode = InputMode::Normal;
        self.table_state.select(Some(0));
    }

    fn handle_search_char(&mut self, c: char) {
        self.search_term.push(c);
        self.table_state.select(Some(0));
    }

    fn handle_search_backspace(&mut self) {
        self.search_term.pop();
        self.table_state.select(Some(0));
    }

    fn confirm_search(&mut self) {
        self.input_mode = InputMode::Normal;
    }

    fn next(&mut self) {
        let len = self.filtered_entries().len();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn previous(&mut self) {
        let len = self.filtered_entries().len();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    len - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn delete_selected(&mut self) -> Result<()> {
        let filtered = self.filtered_entries();
        if let Some(idx) = self.table_state.selected() {
            if idx < filtered.len() {
                let entry_id = filtered[idx].id;
                self.data.entries.retain(|e| e.id != entry_id);
                save_data(&self.data)?;
                let new_len = self.filtered_entries().len();
                if idx >= new_len && new_len > 0 {
                    self.table_state.select(Some(new_len - 1));
                }
            }
        }
        Ok(())
    }

    fn stop_active(&mut self) -> Result<()> {
        self.data.stop_active();
        save_data(&self.data)?;
        Ok(())
    }

    fn next_period(&mut self) {
        match self.view_mode {
            ViewMode::All => {}
            ViewMode::Day => {
                self.selected_date += Duration::days(1);
            }
            ViewMode::Week => {
                self.selected_date += Duration::days(7);
            }
        }
        self.table_state.select(Some(0));
    }

    fn previous_period(&mut self) {
        match self.view_mode {
            ViewMode::All => {}
            ViewMode::Day => {
                self.selected_date -= Duration::days(1);
            }
            ViewMode::Week => {
                self.selected_date -= Duration::days(7);
            }
        }
        self.table_state.select(Some(0));
    }

    fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
        self.table_state.select(Some(0));
    }

    fn go_to_today(&mut self) {
        self.selected_date = Local::now().date_naive();
        self.table_state.select(Some(0));
    }

    fn toggle_sort_order(&mut self) {
        self.sort_order = self.sort_order.toggle();
        self.table_state.select(Some(0));
    }

    fn start_adding(&mut self) {
        self.input_mode = InputMode::AddingEntry;
        self.input_field = InputField::Description;
        self.input_description.clear();
        self.input_tags.clear();
        self.input_start_time.clear();
        self.input_end_time.clear();
        self.input_duration.clear();
    }

    fn cancel_adding(&mut self) {
        self.input_mode = InputMode::Normal;
        self.input_description.clear();
        self.input_tags.clear();
        self.input_start_time.clear();
        self.input_end_time.clear();
        self.input_duration.clear();
        self.editing_entry_id = None;
    }

    fn start_editing(&mut self) {
        // Extract entry data first to avoid borrow conflicts
        let entry_data = {
            let filtered = self.filtered_entries();
            self.table_state.selected().and_then(|idx| {
                filtered.get(idx).map(|entry| {
                    (
                        entry.id,
                        entry.description.clone(),
                        entry.tags.join(" "),
                        entry.start_time.format("%Y-%m-%d %H:%M").to_string(),
                        entry.end_time.map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                        entry.end_time.map(|_| entry.format_duration()),
                    )
                })
            })
        };

        if let Some((id, description, tags, start_time, end_time, duration)) = entry_data {
            self.editing_entry_id = Some(id);
            self.input_description = description;
            self.input_tags = tags;
            self.input_start_time = start_time;
            self.input_end_time = end_time.unwrap_or_default();
            self.input_duration = duration.unwrap_or_default();
            self.input_mode = InputMode::EditingEntry;
            self.input_field = InputField::Description;
        }
    }

    fn submit_edit(&mut self) -> Result<()> {
        let entry_id = match self.editing_entry_id {
            Some(id) => id,
            None => return Ok(()),
        };

        if self.input_description.is_empty() {
            return Ok(());
        }

        let Some((start_time, end_time)) = self.resolve_times() else {
            return Ok(());
        };

        let tags: Vec<String> = self.input_tags
            .split_whitespace()
            .map(|s| s.trim_start_matches('#').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        self.data.update_entry(entry_id, self.input_description.clone(), tags, start_time, end_time);
        save_data(&self.data)?;

        self.cancel_adding();
        Ok(())
    }

    fn next_input_field(&mut self) {
        // Apply auto-fill before leaving the current field
        let leaving = self.input_field;
        self.apply_time_calculations(leaving);

        self.input_field = match self.input_field {
            InputField::Description => InputField::Tags,
            InputField::Tags => InputField::Duration,
            InputField::Duration => InputField::StartTime,
            InputField::StartTime => InputField::EndTime,
            InputField::EndTime => InputField::Description,
        };
    }

    fn prev_input_field(&mut self) {
        let leaving = self.input_field;
        self.apply_time_calculations(leaving);

        self.input_field = match self.input_field {
            InputField::Description => InputField::EndTime,
            InputField::Tags => InputField::Description,
            InputField::Duration => InputField::Tags,
            InputField::StartTime => InputField::Duration,
            InputField::EndTime => InputField::StartTime,
        };
    }

    fn submit_entry(&mut self) -> Result<()> {
        if self.input_description.is_empty() {
            return Ok(());
        }

        let Some((start_time, end_time)) = self.resolve_times() else {
            return Ok(());
        };

        let tags: Vec<String> = self.input_tags
            .split_whitespace()
            .map(|s| s.trim_start_matches('#').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        self.data.add_entry(
            self.input_description.clone(),
            tags,
            start_time,
            end_time,
        );
        save_data(&self.data)?;

        self.cancel_adding();
        Ok(())
    }

    /// Resolve start/end times from the three input fields using the following priority:
    /// - Start + Duration → end = start + duration (duration takes priority over a filled end time)
    /// - Start + End (no duration) → save both
    /// - End + Duration → start = end - duration
    /// - Duration only → assume end = now, start = now - duration
    /// - Start only → active entry (no end time)
    /// Returns None if there is not enough information to determine a start time.
    fn resolve_times(&self) -> Option<(DateTime<Local>, Option<DateTime<Local>>)> {
        let start = if !self.input_start_time.is_empty() {
            self.parse_time_str(&self.input_start_time)
        } else {
            None
        };
        let end = if !self.input_end_time.is_empty() {
            self.parse_time_str(&self.input_end_time)
        } else {
            None
        };
        let dur = if !self.input_duration.is_empty() {
            let d = duration::parse(&self.input_duration);
            if d.num_seconds() > 0 { Some(d) } else { None }
        } else {
            None
        };

        match (start, end, dur) {
            (Some(s), _, Some(d)) => Some((s, Some(s + d))),
            (Some(s), Some(e), None) => Some((s, Some(e))),
            (None, Some(e), Some(d)) => Some((e - d, Some(e))),
            (None, None, Some(d)) => {
                let now = Local::now();
                Some((now - d, Some(now)))
            }
            (Some(s), None, None) => Some((s, None)),
            _ => None,
        }
    }

    /// When the user tabs away from a time-related field, auto-fill the missing field if possible.
    ///
    /// Rules (applied after leaving `leaving_field`):
    /// - Leave StartTime:  if Dur set → End = Start + Dur; else if End set → Dur = End − Start
    /// - Leave EndTime:    if Start + Dur set → Start = End − Dur (preserve duration, move start);
    ///                     else if Start set (Dur empty) → Dur = End − Start;
    ///                     else if Dur set (Start empty) → Start = End − Dur
    /// - Leave Duration:   if Start set → End = Start + Dur; else if End set → Start = End − Dur
    fn apply_time_calculations(&mut self, leaving_field: InputField) {
        let start_str = self.input_start_time.clone();
        let end_str = self.input_end_time.clone();
        let dur_str = self.input_duration.clone();

        let start = if !start_str.is_empty() { self.parse_time_str(&start_str) } else { None };
        let end = if !end_str.is_empty() { self.parse_time_str(&end_str) } else { None };
        let dur = if !dur_str.is_empty() {
            let d = duration::parse(&dur_str);
            if d.num_seconds() > 0 { Some(d) } else { None }
        } else {
            None
        };

        match leaving_field {
            InputField::StartTime => {
                if let (Some(s), Some(d)) = (start, dur) {
                    self.input_end_time = (s + d).format("%Y-%m-%d %H:%M").to_string();
                } else if let (Some(s), Some(e), None) = (start, end, dur) {
                    let diff = e.signed_duration_since(s);
                    if diff.num_seconds() > 0 {
                        self.input_duration = duration::format(diff);
                    }
                }
            }
            InputField::EndTime => {
                if let (Some(_s), Some(e), Some(d)) = (start, end, dur) {
                    // Preserve duration, adjust start to maintain the same span
                    self.input_start_time = (e - d).format("%Y-%m-%d %H:%M").to_string();
                } else if let (Some(s), Some(e), None) = (start, end, dur) {
                    let diff = e.signed_duration_since(s);
                    if diff.num_seconds() > 0 {
                        self.input_duration = duration::format(diff);
                    }
                } else if let (None, Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time = (e - d).format("%Y-%m-%d %H:%M").to_string();
                }
            }
            InputField::Duration => {
                if let (Some(s), Some(d)) = (start, dur) {
                    self.input_end_time = (s + d).format("%Y-%m-%d %H:%M").to_string();
                } else if let (None, Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time = (e - d).format("%Y-%m-%d %H:%M").to_string();
                }
            }
            _ => {}
        }
    }

    fn parse_time_str(&self, input: &str) -> Option<DateTime<Local>> {
        use chrono::Datelike;

        let input = input.trim();
        let current_year = Local::now().year();

        // Check whether the input starts with a date prefix (DD/MM, MM-DD, or YYYY-MM-DD).
        // A date prefix is recognised when it is followed by a space and a time component.
        let (naive_date, time_input) = if let Some(space_idx) = input.find(' ') {
            let date_part = &input[..space_idx];
            let time_part = input[space_idx + 1..].trim();
            match Self::parse_date_part(date_part, current_year) {
                Some(d) => (Some(d), time_part),
                None => (Some(self.selected_date), input),
            }
        } else {
            (Some(self.selected_date), input)
        };

        let date = naive_date?;
        let time = Self::parse_time_part(time_input)?;
        date.and_time(time).and_local_timezone(Local).single()
    }

    /// Parse a date-only string into a `NaiveDate`, assuming `current_year` when the year is
    /// absent. Supported formats: `DD/MM`, `MM-DD`, `YYYY-MM-DD`.
    fn parse_date_part(s: &str, current_year: i32) -> Option<chrono::NaiveDate> {
        use chrono::NaiveDate;

        // DD/MM
        if s.contains('/') {
            let mut parts = s.splitn(2, '/');
            if let (Some(d), Some(m)) = (parts.next(), parts.next()) {
                if let (Ok(day), Ok(month)) = (d.parse::<u32>(), m.parse::<u32>()) {
                    return NaiveDate::from_ymd_opt(current_year, month, day);
                }
            }
        }

        // YYYY-MM-DD
        if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Some(nd);
        }

        // MM-DD (current year assumed)
        if s.len() == 5 && s.contains('-') {
            let with_year = format!("{}-{}", current_year, s);
            if let Ok(nd) = NaiveDate::parse_from_str(&with_year, "%Y-%m-%d") {
                return Some(nd);
            }
        }

        None
    }

    /// Parse a time string into a `NaiveTime`.
    ///
    /// Supported formats:
    /// - `HH:MM` / `H:MM`          – 24-hour with colon separator
    /// - `HH.MM` / `H.MM`          – 24-hour with dot separator
    /// - `HH` / `H`                – hour only (minutes default to 00)
    /// - All of the above with an `am`/`pm` suffix for 12-hour clock
    ///   e.g. `9am`, `9.30am`, `9:30pm`, `11.45 pm`
    fn parse_time_part(s: &str) -> Option<chrono::NaiveTime> {
        use chrono::NaiveTime;

        let s = s.trim().to_lowercase();

        // Detect and strip am/pm suffix (allow optional space before it)
        let (is_12h, is_pm, rest) = if s.ends_with("pm") {
            (true, true, s[..s.len() - 2].trim().to_string())
        } else if s.ends_with("am") {
            (true, false, s[..s.len() - 2].trim().to_string())
        } else {
            (false, false, s.clone())
        };

        // Normalise dot separator → colon so we only need one parsing path
        let rest = rest.replace('.', ":");

        let (hour, minute) = if let Some(colon_pos) = rest.find(':') {
            let h: u32 = rest[..colon_pos].trim().parse().ok()?;
            let m: u32 = rest[colon_pos + 1..].trim().parse().ok()?;
            if m > 59 {
                return None;
            }
            (h, m)
        } else {
            let h: u32 = rest.trim().parse().ok()?;
            (h, 0)
        };

        // Convert 12-hour to 24-hour
        let hour_24 = if is_12h {
            if hour == 0 || hour > 12 {
                return None; // 0 and 13-23 are invalid in 12-hour notation
            }
            match (is_pm, hour) {
                (false, 12) => 0,        // 12 am = midnight
                (true, 12) => 12,        // 12 pm = noon
                (false, h) => h,         // 1 am – 11 am
                (true, h) => h + 12,     // 1 pm – 11 pm
            }
        } else {
            hour
        };

        NaiveTime::from_hms_opt(hour_24, minute, 0)
    }

    fn handle_input_char(&mut self, c: char) {
        match self.input_field {
            InputField::Description => self.input_description.push(c),
            InputField::Tags => self.input_tags.push(c),
            InputField::StartTime => self.input_start_time.push(c),
            InputField::EndTime => self.input_end_time.push(c),
            InputField::Duration => self.input_duration.push(c),
        }
    }

    fn handle_input_backspace(&mut self) {
        match self.input_field {
            InputField::Description => { self.input_description.pop(); }
            InputField::Tags => { self.input_tags.pop(); }
            InputField::StartTime => { self.input_start_time.pop(); }
            InputField::EndTime => { self.input_end_time.pop(); }
            InputField::Duration => { self.input_duration.pop(); }
        }
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
        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.input_mode {
                        InputMode::Normal => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                if app.is_searching() {
                                    app.clear_search();
                                } else if app.is_tag_filtering() {
                                    app.clear_tag_filter();
                                } else {
                                    app.should_quit = true;
                                }
                            }
                            KeyCode::Char('j') | KeyCode::Down => app.next(),
                            KeyCode::Char('k') | KeyCode::Up => app.previous(),
                            KeyCode::Char('d') => app.delete_selected()?,
                            KeyCode::Char('s') => app.stop_active()?,
                            KeyCode::Char('r') => app.reload()?,
                            KeyCode::Char('a') => app.start_adding(),
                            KeyCode::Char('e') => app.start_editing(),
                            KeyCode::Char('f') => app.filter_by_selected_tags(),
                            KeyCode::Char('/') => app.start_search(),
                            // View mode switching
                            KeyCode::Char('1') => app.set_view_mode(ViewMode::Day),
                            KeyCode::Char('2') => app.set_view_mode(ViewMode::Week),
                            KeyCode::Char('3') => app.set_view_mode(ViewMode::All),
                            // Date navigation
                            KeyCode::Char('h') | KeyCode::Left => app.previous_period(),
                            KeyCode::Char('l') | KeyCode::Right => app.next_period(),
                            KeyCode::Char('t') => app.go_to_today(),
                            KeyCode::Char('o') => app.toggle_sort_order(),
                            _ => {}
                        },
                        InputMode::AddingEntry => match key.code {
                            KeyCode::Esc => app.cancel_adding(),
                            KeyCode::Enter => app.submit_entry()?,
                            KeyCode::Tab => app.next_input_field(),
                            KeyCode::BackTab => app.prev_input_field(),
                            KeyCode::Backspace => app.handle_input_backspace(),
                            KeyCode::Char(c) => app.handle_input_char(c),
                            _ => {}
                        },
                        InputMode::EditingEntry => match key.code {
                            KeyCode::Esc => app.cancel_adding(),
                            KeyCode::Enter => app.submit_edit()?,
                            KeyCode::Tab => app.next_input_field(),
                            KeyCode::BackTab => app.prev_input_field(),
                            KeyCode::Backspace => app.handle_input_backspace(),
                            KeyCode::Char(c) => app.handle_input_char(c),
                            _ => {}
                        },
                        InputMode::Searching => match key.code {
                            KeyCode::Esc => app.clear_search(),
                            KeyCode::Enter => app.confirm_search(),
                            KeyCode::Backspace => app.handle_search_backspace(),
                            KeyCode::Char(c) => app.handle_search_char(c),
                            _ => {}
                        },
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    // Adjust layout based on whether search is active
    let show_search = app.is_searching();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_search {
            vec![
                Constraint::Length(3), // Status
                Constraint::Length(3), // Tabs + date info
                Constraint::Length(3), // Search bar
                Constraint::Min(10),   // Main content
                Constraint::Length(3), // Footer
            ]
        } else {
            vec![
                Constraint::Length(3), // Status
                Constraint::Length(3), // Tabs + date info
                Constraint::Min(10),   // Main content
                Constraint::Length(3), // Footer
            ]
        })
        .split(f.area());

    // Header with status
    let (status_text, status_style) = match app.data.active_entry() {
        Some(entry) => (
            format!(
                "{}  {} - {} ",
                icons::ACTIVE,
                entry.description,
                entry.format_duration()
            ),
            Style::default().fg(theme::ACTIVE).bold(),
        ),
        None => (
            "No active task".to_string(),
            Style::default().fg(theme::INACTIVE).italic(),
        ),
    };
    let header = Paragraph::new(status_text)
        .style(status_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER))
                .title(Span::styled(" Status ", Style::default().fg(theme::TITLE))),
        );
    f.render_widget(header, chunks[0]);

    // Tabs for view mode with date info
    let tab_titles = vec!["[1] Day", "[2] Week", "[3] All"];
    let selected_tab = match app.view_mode {
        ViewMode::Day => 0,
        ViewMode::Week => 1,
        ViewMode::All => 2,
    };

    let date_info = match app.view_mode {
        ViewMode::All => "All entries".to_string(),
        ViewMode::Day => app.selected_date.format("%A, %B %d, %Y").to_string(),
        ViewMode::Week => {
            let week_start = TimeData::week_start(app.selected_date);
            let week_end = week_start + Duration::days(6);
            format!(
                "{} - {}",
                week_start.format("%b %d"),
                week_end.format("%b %d, %Y")
            )
        }
    };

    let tabs = Tabs::new(tab_titles)
        .select(selected_tab)
        .style(Style::default().fg(theme::INACTIVE))
        .highlight_style(Style::default().fg(theme::ACCENT).bold())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER))
                .title(Span::styled(
                    format!(" {} | {} | {} ", app.view_mode.title(), date_info, app.sort_order.label()),
                    Style::default().fg(theme::HIGHLIGHT),
                )),
        );
    f.render_widget(tabs, chunks[1]);

    // Determine chunk indices based on layout
    let (content_idx, footer_idx) = if show_search { (3, 4) } else { (2, 3) };

    // Render search bar if active
    if show_search {
        render_search_bar(f, app, chunks[2]);
    }

    // Main content area - split between breakdown and entries in Week view
    if app.input_mode == InputMode::AddingEntry || app.input_mode == InputMode::EditingEntry {
        // Show add/edit entry form
        render_entry_form(f, app, chunks[content_idx]);
    } else if app.view_mode == ViewMode::Week {
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(40)])
            .split(chunks[content_idx]);

        // Daily breakdown panel
        render_weekly_breakdown(f, app, content_chunks[0]);

        // Entries table
        render_entries_table(f, app, content_chunks[1]);
    } else {
        render_entries_table(f, app, chunks[content_idx]);
    }

    // Footer with help - show filtered total when searching or tag filtering
    let (total, total_label) = if app.is_searching() && !app.search_term.is_empty() {
        (app.filtered_total(), "Filtered: ")
    } else if app.is_tag_filtering() {
        (app.filtered_total(), "Tagged: ")
    } else {
        let t = match app.view_mode {
            ViewMode::All => app.data.today_total(),
            ViewMode::Day => app.data.total_for_date(app.selected_date),
            ViewMode::Week => {
                let week_start = TimeData::week_start(app.selected_date);
                app.data.total_for_week(week_start)
            }
        };
        (t, "Total: ")
    };

    let total_str = duration::format(total);
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {}", total_label), Style::default().fg(theme::TITLE)),
        Span::styled(total_str, Style::default().fg(theme::HIGHLIGHT).bold()),
        Span::styled(" | ", Style::default().fg(theme::BORDER)),
        Span::styled("h/l", Style::default().fg(theme::ACCENT)),
        Span::styled(": prev/next | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("t", Style::default().fg(theme::ACCENT)),
        Span::styled(": today | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("o", Style::default().fg(theme::ACCENT)),
        Span::styled(": sort order | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("1-3", Style::default().fg(theme::ACCENT)),
        Span::styled(": views | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("j/k", Style::default().fg(theme::ACCENT)),
        Span::styled(": nav | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("/", Style::default().fg(theme::ACCENT)),
        Span::styled(": search | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("f", Style::default().fg(theme::ACCENT)),
        Span::styled(": filter tags | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("a", Style::default().fg(theme::ACCENT)),
        Span::styled(": add | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("e", Style::default().fg(theme::ACCENT)),
        Span::styled(": edit | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("d", Style::default().fg(theme::ACCENT)),
        Span::styled(": del | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("s", Style::default().fg(theme::ACCENT)),
        Span::styled(": stop | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("q", Style::default().fg(theme::ACCENT)),
        Span::styled(": quit ", Style::default().fg(theme::INACTIVE)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER)),
    );
    f.render_widget(footer, chunks[footer_idx]);
}

fn render_weekly_breakdown(f: &mut Frame, app: &App, area: Rect) {
    let week_start = TimeData::week_start(app.selected_date);
    let breakdown = app.data.daily_breakdown(week_start);

    let rows: Vec<Row> = breakdown
        .iter()
        .map(|(date, dur)| {
            let day_name = date.format("%a").to_string();
            let date_str = date.format("%m/%d").to_string();
            let dur_str = duration::format(*dur);
            let is_today = *date == Local::now().date_naive();
            let hours = dur.num_hours();

            // Color code duration
            let dur_color = if hours >= 8 {
                theme::DURATION_HIGH
            } else if hours >= 4 {
                theme::DURATION_MED
            } else {
                theme::DURATION_LOW
            };

            let (day_style, date_style) = if is_today {
                (
                    Style::default().fg(theme::HIGHLIGHT).bold(),
                    Style::default().fg(theme::HIGHLIGHT),
                )
            } else {
                (
                    Style::default().fg(theme::ACCENT),
                    Style::default().fg(theme::TITLE),
                )
            };

            Row::new(vec![
                Cell::from(day_name).style(day_style),
                Cell::from(date_str).style(date_style),
                Cell::from(dur_str).style(Style::default().fg(dur_color)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Min(8),
        ],
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(
                " Daily Totals ",
                Style::default().fg(theme::TITLE),
            )),
    );

    f.render_widget(table, area);
}

fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let is_active = app.input_mode == InputMode::Searching;
    let border_style = if is_active {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::BORDER)
    };

    let match_count = app.filtered_entries().len();
    let match_info = if app.search_term.is_empty() {
        String::new()
    } else {
        format!(" ({} matches)", match_count)
    };

    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            format!(" Search{} ", match_info),
            if is_active {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));

    let search_text = if is_active && app.search_term.is_empty() {
        "Type to search... (Enter to confirm, Esc to clear)"
    } else {
        &app.search_term
    };

    let search_input = Paragraph::new(search_text)
        .style(if app.search_term.is_empty() && is_active {
            Style::default().fg(theme::INACTIVE).italic()
        } else {
            Style::default().fg(Color::White)
        })
        .block(search_block);
    f.render_widget(search_input, area);

    // Show cursor if actively searching
    if is_active {
        f.set_cursor_position((
            area.x + app.search_term.len() as u16 + 1,
            area.y + 1,
        ));
    }
}

fn render_entry_form(f: &mut Frame, app: &App, area: Rect) {
    let is_editing = app.input_mode == InputMode::EditingEntry;
    let form_title = if is_editing { " Edit Entry " } else { " Add Log Entry " };
    
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Description
            Constraint::Length(3), // Tags
            Constraint::Length(3), // Duration
            Constraint::Length(3), // Start Time
            Constraint::Length(3), // End Time
            Constraint::Length(3), // Help
            Constraint::Min(0),
        ])
        .split(area);

    // Description input
    let desc_style = if app.input_field == InputField::Description {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::INACTIVE)
    };
    let desc_block = Block::default()
        .borders(Borders::ALL)
        .border_style(desc_style)
        .title(Span::styled(
            " Description ",
            if app.input_field == InputField::Description {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));
    let desc_input = Paragraph::new(app.input_description.as_str())
        .style(Style::default().fg(Color::White))
        .block(desc_block);
    f.render_widget(desc_input, chunks[0]);

    // Tags input
    let tags_style = if app.input_field == InputField::Tags {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::INACTIVE)
    };
    let tags_block = Block::default()
        .borders(Borders::ALL)
        .border_style(tags_style)
        .title(Span::styled(
            " Tags (space-separated, e.g., work meeting) ",
            if app.input_field == InputField::Tags {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));
    let tags_input = Paragraph::new(app.input_tags.as_str())
        .style(Style::default().fg(Color::White))
        .block(tags_block);
    f.render_widget(tags_input, chunks[1]);

    // Duration input (optional)
    let dur_style = if app.input_field == InputField::Duration {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::INACTIVE)
    };
    let dur_block = Block::default()
        .borders(Borders::ALL)
        .border_style(dur_style)
        .title(Span::styled(
            " Duration (optional: 1h30m, 45m, 2h) ",
            if app.input_field == InputField::Duration {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));
    let dur_input = Paragraph::new(app.input_duration.as_str())
        .style(Style::default().fg(Color::White))
        .block(dur_block);
    f.render_widget(dur_input, chunks[2]);

    // Start Time input
    let start_style = if app.input_field == InputField::StartTime {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::INACTIVE)
    };
    let start_block = Block::default()
        .borders(Borders::ALL)
        .border_style(start_style)
        .title(Span::styled(
            " Start Time (e.g. 9am, 14:30, 25/03 9.30am) ",
            if app.input_field == InputField::StartTime {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));
    let start_input = Paragraph::new(app.input_start_time.as_str())
        .style(Style::default().fg(Color::White))
        .block(start_block);
    f.render_widget(start_input, chunks[3]);

    // End Time input (optional)
    let end_style = if app.input_field == InputField::EndTime {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::INACTIVE)
    };
    let end_block = Block::default()
        .borders(Borders::ALL)
        .border_style(end_style)
        .title(Span::styled(
            " End Time (optional: e.g. 9am, 14:30, 25/03 9.30am) ",
            if app.input_field == InputField::EndTime {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));
    let end_input = Paragraph::new(app.input_end_time.as_str())
        .style(Style::default().fg(Color::White))
        .block(end_block);
    f.render_widget(end_input, chunks[4]);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("Tab", Style::default().fg(theme::ACCENT)),
        Span::styled(": switch field | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Enter", Style::default().fg(theme::ACCENT)),
        Span::styled(": save | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Esc", Style::default().fg(theme::ACCENT)),
        Span::styled(": cancel  ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Need ≥2 of: Start, End, Duration", Style::default().fg(theme::BORDER)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(form_title, Style::default().fg(theme::HIGHLIGHT))),
    );
    f.render_widget(help, chunks[5]);

    // Show cursor in active field
    let (cursor_x, cursor_y) = match app.input_field {
        InputField::Description => (
            chunks[0].x + app.input_description.len() as u16 + 1,
            chunks[0].y + 1,
        ),
        InputField::Tags => (
            chunks[1].x + app.input_tags.len() as u16 + 1,
            chunks[1].y + 1,
        ),
        InputField::Duration => (
            chunks[2].x + app.input_duration.len() as u16 + 1,
            chunks[2].y + 1,
        ),
        InputField::StartTime => (
            chunks[3].x + app.input_start_time.len() as u16 + 1,
            chunks[3].y + 1,
        ),
        InputField::EndTime => (
            chunks[4].x + app.input_end_time.len() as u16 + 1,
            chunks[4].y + 1,
        ),
    };
    f.set_cursor_position((cursor_x, cursor_y));
}

fn entry_row(entry: &crate::tracker::TimeEntry, stripe: bool) -> Row<'_> {
    let hours = entry.duration().num_hours();
    let dur_color = if hours >= 4 {
        theme::DURATION_HIGH
    } else if hours >= 2 {
        theme::DURATION_MED
    } else {
        theme::DURATION_LOW
    };

    let status_style = if entry.is_active() {
        Style::default().fg(theme::ACTIVE)
    } else {
        Style::default().fg(theme::INACTIVE)
    };

    let row_style = if stripe {
        Style::default().bg(Color::Rgb(35, 35, 35))
    } else {
        Style::default()
    };

    let end_str = entry
        .end_time
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_else(|| "—".to_string());

    Row::new(vec![
        Cell::from(entry.start_time.format("%Y-%m-%d").to_string())
            .style(Style::default().fg(theme::TITLE)),
        Cell::from(entry.start_time.format("%H:%M").to_string())
            .style(Style::default().fg(theme::ACCENT)),
        Cell::from(end_str).style(Style::default().fg(theme::INACTIVE)),
        Cell::from(entry.description.clone()),
        Cell::from(entry.format_tags())
            .style(Style::default().fg(theme::HIGHLIGHT)),
        Cell::from(entry.format_duration()).style(Style::default().fg(dur_color)),
        Cell::from(entry.status_icon()).style(status_style),
    ])
    .style(row_style)
}

fn day_header_row(date: NaiveDate, total: Duration) -> Row<'static> {
    // Prepend a newline so the content sits on the second line of the 2-row cell,
    // making it feel visually attached to the entries that follow.
    let weekday = format!("\n{}", date.format("%A"));
    let date_str = format!("\n{}", date.format("%B %d, %Y"));
    let total_str = format!("\n{}", duration::format(total));

    Row::new(vec![
        Cell::from(weekday).style(
            Style::default()
                .fg(theme::HIGHLIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
        Cell::from(""),
        Cell::from(date_str).style(Style::default().fg(theme::TITLE)),
        Cell::from(""),
        Cell::from(total_str).style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
    ])
    .height(2)
    .style(Style::default().bg(theme::DAY_HEADER_BG))
}

fn render_entries_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = ["Date", "Start", "End", "Description", "Tags", "Duration", ""]
        .into_iter()
        .map(|h| {
            Cell::from(h).style(
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
        });
    let header_row = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme::HEADER_BG));

    let entries = app.filtered_entries();

    // In Week view, interleave day-separator header rows before each day's entries.
    // table_state tracks the entry index (not visual row index), so we compute a
    // separate visual index for the selected entry to pass to the stateful widget.
    let (rows, visual_selected): (Vec<Row>, Option<usize>) =
        if app.view_mode == ViewMode::Week {
            // Sum durations per day from the filtered entries
            let mut day_totals: HashMap<NaiveDate, Duration> = HashMap::new();
            for entry in &entries {
                let date = entry.start_time.date_naive();
                *day_totals.entry(date).or_insert_with(Duration::zero) += entry.duration();
            }

            let mut rows: Vec<Row> = Vec::new();
            // Maps entry_idx -> visual row index (accounting for inserted day headers)
            let mut visual_idx_map: Vec<usize> = Vec::with_capacity(entries.len());
            let mut current_date: Option<NaiveDate> = None;
            let mut stripe = false;

            for entry in entries.iter() {
                let entry_date = entry.start_time.date_naive();

                if current_date != Some(entry_date) {
                    current_date = Some(entry_date);
                    stripe = false;
                    let total = day_totals
                        .get(&entry_date)
                        .copied()
                        .unwrap_or_else(Duration::zero);
                    rows.push(day_header_row(entry_date, total));
                }

                visual_idx_map.push(rows.len());
                rows.push(entry_row(entry, stripe));
                stripe = !stripe;
            }

            let visual_sel = app
                .table_state
                .selected()
                .and_then(|idx| visual_idx_map.get(idx).copied());

            (rows, visual_sel)
        } else {
            let rows = entries
                .iter()
                .enumerate()
                .map(|(i, entry)| entry_row(entry, i % 2 != 0))
                .collect();
            (rows, app.table_state.selected())
        };

    // Build title with tag filter info
    let title = if app.is_tag_filtering() {
        format!(
            " Entries [filtered: {}] ",
            app.tag_filter
                .iter()
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else {
        " Entries ".to_string()
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(12),
            Constraint::Length(14),
            Constraint::Length(9),
            Constraint::Length(3),
        ],
    )
    .header(header_row)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(title, Style::default().fg(theme::TITLE))),
    )
    .row_highlight_style(Style::default().bg(theme::SELECTED_BG))
    .highlight_symbol(">> ");

    // Use a temporary render state so that table_state continues to track the
    // entry index (not the visual row index which includes day header rows).
    let mut render_state = TableState::default().with_selected(visual_selected);
    f.render_stateful_widget(table, area, &mut render_state);
}
