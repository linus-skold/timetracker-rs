use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate};
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
    Searching,
}

#[derive(Clone, Copy, PartialEq)]
enum InputField {
    Description,
    Duration,
    Timestamp,
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
    input_duration: String,
    input_timestamp: String, // Optional: end time like "14:30" or "2024-03-16 14:30"
    // Search state
    search_term: String,
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
            input_duration: String::new(),
            input_timestamp: String::new(),
            search_term: String::new(),
        })
    }

    fn reload(&mut self) -> Result<()> {
        self.data = load_data()?;
        Ok(())
    }

    fn filtered_entries(&self) -> Vec<&crate::tracker::TimeEntry> {
        let entries: Vec<_> = match self.view_mode {
            ViewMode::All => self.data.entries.iter().rev().collect(),
            ViewMode::Day => {
                let mut entries: Vec<_> = self.data.entries_for_date(self.selected_date);
                entries.reverse();
                entries
            }
            ViewMode::Week => {
                let week_start = TimeData::week_start(self.selected_date);
                let mut entries: Vec<_> = self.data.entries_for_week(week_start);
                entries.reverse();
                entries
            }
        };

        // Apply search filter if search term is not empty
        if self.search_term.is_empty() {
            entries
        } else {
            let search_lower = self.search_term.to_lowercase();
            entries
                .into_iter()
                .filter(|e| e.description.to_lowercase().contains(&search_lower))
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

    fn start_adding(&mut self) {
        self.input_mode = InputMode::AddingEntry;
        self.input_field = InputField::Description;
        self.input_description.clear();
        self.input_duration.clear();
        self.input_timestamp.clear();
    }

    fn cancel_adding(&mut self) {
        self.input_mode = InputMode::Normal;
        self.input_description.clear();
        self.input_duration.clear();
        self.input_timestamp.clear();
    }

    fn next_input_field(&mut self) {
        self.input_field = match self.input_field {
            InputField::Description => InputField::Duration,
            InputField::Duration => InputField::Timestamp,
            InputField::Timestamp => InputField::Description,
        };
    }

    fn submit_entry(&mut self) -> Result<()> {
        if self.input_description.is_empty() || self.input_duration.is_empty() {
            return Ok(()); // Don't submit if empty
        }

        let dur = duration::parse(&self.input_duration);
        if dur.num_seconds() <= 0 {
            return Ok(()); // Invalid duration
        }

        // Parse end time from timestamp or use now
        let end_time = if self.input_timestamp.is_empty() {
            Local::now()
        } else {
            self.parse_timestamp().unwrap_or_else(Local::now)
        };
        let start_time = end_time - dur;

        self.data.add_entry(
            self.input_description.clone(),
            start_time,
            Some(end_time),
        );
        save_data(&self.data)?;

        self.cancel_adding();
        Ok(())
    }

    fn parse_timestamp(&self) -> Option<DateTime<Local>> {
        use chrono::NaiveTime;
        
        let input = self.input_timestamp.trim();
        
        // Try parsing as time only (HH:MM) - assumes today or selected date
        if let Ok(time) = NaiveTime::parse_from_str(input, "%H:%M") {
            let date = self.selected_date;
            let naive_dt = date.and_time(time);
            return Some(naive_dt.and_local_timezone(Local).single()?);
        }
        
        // Try parsing as full datetime (YYYY-MM-DD HH:MM)
        if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M") {
            return Some(naive_dt.and_local_timezone(Local).single()?);
        }
        
        // Try parsing as date and time (MM-DD HH:MM) - assumes current year
        if input.len() >= 11 {
            let with_year = format!("{}-{}", Local::now().format("%Y"), input);
            if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(&with_year, "%Y-%m-%d %H:%M") {
                return Some(naive_dt.and_local_timezone(Local).single()?);
            }
        }
        
        None
    }

    fn handle_input_char(&mut self, c: char) {
        match self.input_field {
            InputField::Description => self.input_description.push(c),
            InputField::Duration => self.input_duration.push(c),
            InputField::Timestamp => self.input_timestamp.push(c),
        }
    }

    fn handle_input_backspace(&mut self) {
        match self.input_field {
            InputField::Description => { self.input_description.pop(); }
            InputField::Duration => { self.input_duration.pop(); }
            InputField::Timestamp => { self.input_timestamp.pop(); }
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
                            KeyCode::Char('/') => app.start_search(),
                            // View mode switching
                            KeyCode::Char('1') => app.set_view_mode(ViewMode::Day),
                            KeyCode::Char('2') => app.set_view_mode(ViewMode::Week),
                            KeyCode::Char('3') => app.set_view_mode(ViewMode::All),
                            // Date navigation
                            KeyCode::Char('h') | KeyCode::Left => app.previous_period(),
                            KeyCode::Char('l') | KeyCode::Right => app.next_period(),
                            KeyCode::Char('t') => app.go_to_today(),
                            _ => {}
                        },
                        InputMode::AddingEntry => match key.code {
                            KeyCode::Esc => app.cancel_adding(),
                            KeyCode::Enter => app.submit_entry()?,
                            KeyCode::Tab => app.next_input_field(),
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
                    format!(" {} | {} ", app.view_mode.title(), date_info),
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
    if app.input_mode == InputMode::AddingEntry {
        // Show add entry form
        render_add_entry_form(f, app, chunks[content_idx]);
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

    // Footer with help - show filtered total when searching
    let (total, total_label) = if app.is_searching() && !app.search_term.is_empty() {
        (app.filtered_total(), "Filtered: ")
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
        Span::styled("1-3", Style::default().fg(theme::ACCENT)),
        Span::styled(": views | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("j/k", Style::default().fg(theme::ACCENT)),
        Span::styled(": nav | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("/", Style::default().fg(theme::ACCENT)),
        Span::styled(": search | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("a", Style::default().fg(theme::ACCENT)),
        Span::styled(": add | ", Style::default().fg(theme::INACTIVE)),
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

fn render_add_entry_form(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
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

    // Duration input
    let dur_style = if app.input_field == InputField::Duration {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::INACTIVE)
    };
    let dur_block = Block::default()
        .borders(Borders::ALL)
        .border_style(dur_style)
        .title(Span::styled(
            " Duration (e.g., 1h30m, 45m, 2h) ",
            if app.input_field == InputField::Duration {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));
    let dur_input = Paragraph::new(app.input_duration.as_str())
        .style(Style::default().fg(Color::White))
        .block(dur_block);
    f.render_widget(dur_input, chunks[1]);

    // Timestamp input (optional)
    let ts_style = if app.input_field == InputField::Timestamp {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::INACTIVE)
    };
    let ts_block = Block::default()
        .borders(Borders::ALL)
        .border_style(ts_style)
        .title(Span::styled(
            " End Time (optional: HH:MM or YYYY-MM-DD HH:MM, blank=now) ",
            if app.input_field == InputField::Timestamp {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));
    let ts_input = Paragraph::new(app.input_timestamp.as_str())
        .style(Style::default().fg(Color::White))
        .block(ts_block);
    f.render_widget(ts_input, chunks[2]);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("Tab", Style::default().fg(theme::ACCENT)),
        Span::styled(": switch field | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Enter", Style::default().fg(theme::ACCENT)),
        Span::styled(": save | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Esc", Style::default().fg(theme::ACCENT)),
        Span::styled(": cancel", Style::default().fg(theme::INACTIVE)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(" Add Log Entry ", Style::default().fg(theme::HIGHLIGHT))),
    );
    f.render_widget(help, chunks[3]);

    // Show cursor in active field
    let (cursor_x, cursor_y) = match app.input_field {
        InputField::Description => (
            chunks[0].x + app.input_description.len() as u16 + 1,
            chunks[0].y + 1,
        ),
        InputField::Duration => (
            chunks[1].x + app.input_duration.len() as u16 + 1,
            chunks[1].y + 1,
        ),
        InputField::Timestamp => (
            chunks[2].x + app.input_timestamp.len() as u16 + 1,
            chunks[2].y + 1,
        ),
    };
    f.set_cursor_position((cursor_x, cursor_y));
}

fn render_entries_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = ["Date", "Time", "Description", "Duration", ""]
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
    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
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

            let row_style = if i % 2 == 0 {
                Style::default()
            } else {
                Style::default().bg(Color::Rgb(35, 35, 35))
            };

            Row::new(vec![
                Cell::from(entry.start_time.format("%Y-%m-%d").to_string())
                    .style(Style::default().fg(theme::TITLE)),
                Cell::from(entry.start_time.format("%H:%M").to_string())
                    .style(Style::default().fg(theme::ACCENT)),
                Cell::from(entry.description.clone()),
                Cell::from(entry.format_duration()).style(Style::default().fg(dur_color)),
                Cell::from(entry.status_icon()).style(status_style),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(3),
        ],
    )
    .header(header_row)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(" Entries ", Style::default().fg(theme::TITLE))),
    )
    .row_highlight_style(Style::default().bg(theme::SELECTED_BG))
    .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}
