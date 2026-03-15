use anyhow::Result;
use chrono::{Duration, Local, NaiveDate};
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

struct App {
    data: TimeData,
    table_state: TableState,
    should_quit: bool,
    view_mode: ViewMode,
    selected_date: NaiveDate,
}

impl App {
    fn new() -> Result<Self> {
        Ok(Self {
            data: load_data()?,
            table_state: TableState::default().with_selected(Some(0)),
            should_quit: false,
            view_mode: ViewMode::Day,
            selected_date: Local::now().date_naive(),
        })
    }

    fn reload(&mut self) -> Result<()> {
        self.data = load_data()?;
        Ok(())
    }

    fn filtered_entries(&self) -> Vec<&crate::tracker::TimeEntry> {
        match self.view_mode {
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
        }
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
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                        KeyCode::Char('j') | KeyCode::Down => app.next(),
                        KeyCode::Char('k') | KeyCode::Up => app.previous(),
                        KeyCode::Char('d') => app.delete_selected()?,
                        KeyCode::Char('s') => app.stop_active()?,
                        KeyCode::Char('r') => app.reload()?,
                        // View mode switching
                        KeyCode::Char('1') => app.set_view_mode(ViewMode::Day),
                        KeyCode::Char('2') => app.set_view_mode(ViewMode::Week),
                        KeyCode::Char('3') => app.set_view_mode(ViewMode::All),
                        // Date navigation
                        KeyCode::Char('h') | KeyCode::Left => app.previous_period(),
                        KeyCode::Char('l') | KeyCode::Right => app.next_period(),
                        KeyCode::Char('t') => app.go_to_today(),
                        _ => {}
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Status
            Constraint::Length(3), // Tabs + date info
            Constraint::Min(10),   // Main content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Header with status
    let status_text = match app.data.active_entry() {
        Some(entry) => format!(
            "{}  {} - {} ",
            icons::ACTIVE,
            entry.description,
            entry.format_duration()
        ),
        None => "No active task".to_string(),
    };
    let header =
        Paragraph::new(status_text).block(Block::default().borders(Borders::ALL).title(" Status "));
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
        .highlight_style(Style::default().fg(Color::Yellow).bold())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} | {} ", app.view_mode.title(), date_info)),
        );
    f.render_widget(tabs, chunks[1]);

    // Main content area - split between breakdown and entries in Week view
    if app.view_mode == ViewMode::Week {
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(40)])
            .split(chunks[2]);

        // Daily breakdown panel
        render_weekly_breakdown(f, app, content_chunks[0]);

        // Entries table
        render_entries_table(f, app, content_chunks[1]);
    } else {
        render_entries_table(f, app, chunks[2]);
    }

    // Footer with help
    let total = match app.view_mode {
        ViewMode::All => app.data.today_total(),
        ViewMode::Day => app.data.total_for_date(app.selected_date),
        ViewMode::Week => {
            let week_start = TimeData::week_start(app.selected_date);
            app.data.total_for_week(week_start)
        }
    };

    let footer_text = format!(
        " Total: {} | h/l: prev/next | t: today | 1-3: views | j/k: nav | d: del | s: stop | q: quit ",
        duration::format(total)
    );
    let footer = Paragraph::new(footer_text).block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[3]);
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
            let style = if is_today {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(day_name),
                Cell::from(date_str),
                Cell::from(dur_str),
            ])
            .style(style)
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
            .title(" Daily Totals "),
    );

    f.render_widget(table, area);
}

fn render_entries_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = ["Date", "Time", "Description", "Duration", ""]
        .into_iter()
        .map(|h| Cell::from(h).style(Style::default().fg(Color::Yellow)));
    let header_row = Row::new(header_cells).height(1);

    let entries = app.filtered_entries();
    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            Row::new(vec![
                Cell::from(entry.start_time.format("%Y-%m-%d").to_string()),
                Cell::from(entry.start_time.format("%H:%M").to_string()),
                Cell::from(entry.description.clone()),
                Cell::from(entry.format_duration()),
                Cell::from(entry.status_icon()),
            ])
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
    .block(Block::default().borders(Borders::ALL).title(" Entries "))
    .row_highlight_style(Style::default().bg(Color::DarkGray))
    .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}
