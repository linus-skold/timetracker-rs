use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Stdout};

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::duration;
use crate::icons;
use crate::storage::{load_data, save_data};
use crate::time::TimeData;


struct App {
    data: TimeData,
    table_state: TableState,
    should_quit: bool,
}

impl App {
    fn new() -> Result<Self> {
        Ok(Self {
            data: load_data()?,
            table_state: TableState::default().with_selected(Some(0)),
            should_quit: false,
        })
    }

    fn reload(&mut self) -> Result<()> {
        self.data = load_data()?;
        Ok(())
    }

    fn next(&mut self) {
        let len = self.data.entries.len();
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
        let len = self.data.entries.len();
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
        if let Some(idx) = self.table_state.selected() {
            if idx < self.data.entries.len() {
                self.data.entries.remove(idx);
                save_data(&self.data)?;
                if idx >= self.data.entries.len() && !self.data.entries.is_empty() {
                    self.table_state.select(Some(self.data.entries.len() - 1));
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
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header with status
    let status_text = match app.data.active_entry() {
        Some(entry) => format!("{}  {} - {} ", icons::ACTIVE, entry.description, entry.format_duration()),
        None => "No active task".to_string(),
    };
    let header =
        Paragraph::new(status_text).block(Block::default().borders(Borders::ALL).title(" Status "));
    f.render_widget(header, chunks[0]);

    // Table of entries
    let header_cells = ["Date", "Time", "Description", "Duration", ""]
        .into_iter()
        .map(|h| Cell::from(h).style(Style::default().fg(Color::Yellow)));
    let header_row = Row::new(header_cells).height(1);

    let rows: Vec<Row> = app
        .data
        .entries
        .iter()
        .rev()
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

    f.render_stateful_widget(table, chunks[1], &mut app.table_state);

    // Footer with help and today's total
    let footer_text = format!(
        " Today: {} | j/k: navigate | d: delete | s: stop | r: reload | q: quit ",
        duration::format(app.data.today_total())
    );
    let footer = Paragraph::new(footer_text).block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
