use std::collections::HashMap;
use chrono::{Duration, Local, NaiveDate};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Tabs},
};
use crate::tracker::TimeData;
use super::{theme, App};
use super::types::{InputField, InputMode, ViewMode};

pub fn ui(f: &mut Frame, app: &mut App) {
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

    // Status header
    let (status_text, status_style) = match app.data.active_entry() {
        Some(entry) => (
            format!(
                "{}  {} - {} ",
                crate::icons::ACTIVE,
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

    // View tabs
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

    let (content_idx, footer_idx) = if show_search { (3, 4) } else { (2, 3) };

    if show_search {
        render_search_bar(f, app, chunks[2]);
    }

    if app.input_mode == InputMode::AddingEntry || app.input_mode == InputMode::EditingEntry {
        render_entry_form(f, app, chunks[content_idx]);
    } else if app.view_mode == ViewMode::Week {
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(40)])
            .split(chunks[content_idx]);
        render_weekly_breakdown(f, app, content_chunks[0]);
        render_entries_table(f, app, content_chunks[1]);
    } else {
        render_entries_table(f, app, chunks[content_idx]);
    }

    // Footer: left = hints (clips), right = "? : help" (always visible)
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

    let total_str = crate::duration::format(total);
    let footer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER));
    let footer_inner = footer_block.inner(chunks[footer_idx]);
    f.render_widget(footer_block, chunks[footer_idx]);

    const HELP_WIDTH: u16 = 11;
    let hints_width = footer_inner.width.saturating_sub(HELP_WIDTH);
    let footer_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(hints_width), Constraint::Length(HELP_WIDTH)])
        .split(footer_inner);

    let hints = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {}", total_label), Style::default().fg(theme::TITLE)),
        Span::styled(total_str, Style::default().fg(theme::HIGHLIGHT).bold()),
        Span::styled(" | ", Style::default().fg(theme::BORDER)),
        Span::styled("t", Style::default().fg(theme::ACCENT)),
        Span::styled(": today | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("/", Style::default().fg(theme::ACCENT)),
        Span::styled(": search | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("a", Style::default().fg(theme::ACCENT)),
        Span::styled(": add | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("e", Style::default().fg(theme::ACCENT)),
        Span::styled(": edit | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("d", Style::default().fg(theme::ACCENT)),
        Span::styled(": del | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("s", Style::default().fg(theme::ACCENT)),
        Span::styled(": stop", Style::default().fg(theme::INACTIVE)),
    ]));
    f.render_widget(hints, footer_chunks[0]);

    let help_hint = Paragraph::new(Line::from(vec![
        Span::styled(" | ", Style::default().fg(theme::BORDER)),
        Span::styled("?", Style::default().fg(theme::ACCENT)),
        Span::styled(": help", Style::default().fg(theme::INACTIVE)),
    ]));
    f.render_widget(help_hint, footer_chunks[1]);

    if app.input_mode == InputMode::Help {
        render_help_popup(f);
    }
}

pub fn render_help_popup(f: &mut Frame) {
    let area = f.area();
    let popup_width = 52u16.min(area.width.saturating_sub(4));
    let popup_height = 26u16.min(area.height.saturating_sub(4));
    let popup_area = Rect {
        x: (area.width.saturating_sub(popup_width)) / 2,
        y: (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    f.render_widget(Clear, popup_area);

    fn key(k: &'static str) -> Span<'static> {
        Span::styled(k, Style::default().fg(theme::ACCENT).bold())
    }
    fn sep(s: &'static str) -> Span<'static> {
        Span::styled(s, Style::default().fg(theme::INACTIVE))
    }
    fn heading(s: &'static str) -> Line<'static> {
        Line::from(Span::styled(s, Style::default().fg(theme::HIGHLIGHT).bold()))
    }

    let lines: Vec<Line> = vec![
        heading("  Navigation"),
        Line::from(vec![key("  h / ←"), sep("  previous period")]),
        Line::from(vec![key("  l / →"), sep("  next period")]),
        Line::from(vec![key("  j / ↓"), sep("  select next entry")]),
        Line::from(vec![key("  k / ↑"), sep("  select previous entry")]),
        Line::from(vec![key("  t"), sep("        go to today")]),
        Line::from(vec![key("  1 / 2 / 3"), sep("  day / week / all view")]),
        Line::from(Span::raw("")),
        heading("  Entries"),
        Line::from(vec![key("  a"), sep("        add entry (uses browsed date)")]),
        Line::from(vec![key("  e"), sep("        edit selected entry")]),
        Line::from(vec![key("  d"), sep("        delete selected entry")]),
        Line::from(vec![key("  s"), sep("        stop active entry")]),
        Line::from(Span::raw("")),
        heading("  Search & Filter"),
        Line::from(vec![key("  /"), sep("        search entries")]),
        Line::from(vec![key("  f"), sep("        filter by selected tags")]),
        Line::from(Span::raw("")),
        heading("  Other"),
        Line::from(vec![key("  o"), sep("        toggle sort order")]),
        Line::from(vec![key("  r"), sep("        reload data from disk")]),
        Line::from(vec![key("  ?"), sep("        toggle this help")]),
        Line::from(vec![key("  q / Esc"), sep("  quit")]),
    ];

    let popup = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::ACCENT))
                .title(Span::styled(
                    " Keybindings ",
                    Style::default().fg(theme::HIGHLIGHT).bold(),
                )),
        )
        .style(Style::default().bg(Color::Rgb(28, 28, 28)));
    f.render_widget(popup, popup_area);
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

    if is_active {
        f.set_cursor_position((
            area.x + Line::from(app.search_term.as_str()).width() as u16 + 1,
            area.y + 1,
        ));
    }
}

fn render_entry_form(f: &mut Frame, app: &App, area: Rect) {
    let is_editing = app.input_mode == InputMode::EditingEntry;
    let form_title = if is_editing {
        " Edit Entry ".to_string()
    } else {
        let today = Local::now().date_naive();
        if app.selected_date == today {
            " Add Log Entry ".to_string()
        } else {
            format!(" Add Log Entry — {} ", app.selected_date.format("%a, %d %b %Y"))
        }
    };

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

    fn field_block(label: &'static str, active: bool) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_style(if active {
                Style::default().fg(theme::ACCENT)
            } else {
                Style::default().fg(theme::INACTIVE)
            })
            .title(Span::styled(
                label,
                if active {
                    Style::default().fg(theme::HIGHLIGHT)
                } else {
                    Style::default().fg(theme::TITLE)
                },
            ))
    }

    let active = app.input_field;

    f.render_widget(
        Paragraph::new(app.input_description.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Description ", active == InputField::Description)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(app.input_tags.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Tags (space-separated, e.g., work meeting) ", active == InputField::Tags)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(app.input_duration.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Duration (optional: 1h30m, 45m, 2h) ", active == InputField::Duration)),
        chunks[2],
    );
    f.render_widget(
        Paragraph::new(app.input_start_time.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Start Time (e.g. 9am, 14:30, 25/03 9.30am) ", active == InputField::StartTime)),
        chunks[3],
    );
    f.render_widget(
        Paragraph::new(app.input_end_time.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" End Time (optional: e.g. 9am, 14:30, 25/03 9.30am) ", active == InputField::EndTime)),
        chunks[4],
    );

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

    // Cursor: use Line::width() for correct Unicode display-column measurement
    let (cursor_x, cursor_y) = match app.input_field {
        InputField::Description => (
            chunks[0].x + Line::from(app.input_description.as_str()).width() as u16 + 1,
            chunks[0].y + 1,
        ),
        InputField::Tags => (
            chunks[1].x + Line::from(app.input_tags.as_str()).width() as u16 + 1,
            chunks[1].y + 1,
        ),
        InputField::Duration => (
            chunks[2].x + Line::from(app.input_duration.as_str()).width() as u16 + 1,
            chunks[2].y + 1,
        ),
        InputField::StartTime => (
            chunks[3].x + Line::from(app.input_start_time.as_str()).width() as u16 + 1,
            chunks[3].y + 1,
        ),
        InputField::EndTime => (
            chunks[4].x + Line::from(app.input_end_time.as_str()).width() as u16 + 1,
            chunks[4].y + 1,
        ),
    };
    f.set_cursor_position((cursor_x, cursor_y));
}

fn render_weekly_breakdown(f: &mut Frame, app: &App, area: Rect) {
    let week_start = TimeData::week_start(app.selected_date);
    let breakdown = app.data.daily_breakdown(week_start);

    let rows: Vec<Row> = breakdown
        .iter()
        .map(|(date, dur)| {
            let day_name = date.format("%a").to_string();
            let date_str = date.format("%m/%d").to_string();
            let dur_str = crate::duration::format(*dur);
            let is_today = *date == Local::now().date_naive();
            let hours = dur.num_hours();

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
            .title(Span::styled(" Daily Totals ", Style::default().fg(theme::TITLE))),
    );
    f.render_widget(table, area);
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
        Cell::from(entry.format_tags()).style(Style::default().fg(theme::HIGHLIGHT)),
        Cell::from(entry.format_duration()).style(Style::default().fg(dur_color)),
        Cell::from(entry.status_icon()).style(status_style),
    ])
    .style(row_style)
}

fn day_header_row(date: NaiveDate, total: Duration) -> Row<'static> {
    let weekday = format!("\n{}", date.format("%A"));
    let date_str = format!("\n{}", date.format("%B %d, %Y"));
    let total_str = format!("\n{}", crate::duration::format(total));

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

    let (rows, visual_selected): (Vec<Row>, Option<usize>) =
        if app.view_mode == ViewMode::Week {
            let mut day_totals: HashMap<NaiveDate, Duration> = HashMap::new();
            for entry in &entries {
                let date = entry.start_time.date_naive();
                *day_totals.entry(date).or_insert_with(Duration::zero) += entry.duration();
            }

            let mut rows: Vec<Row> = Vec::new();
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

    let mut render_state = TableState::default().with_selected(visual_selected);
    f.render_stateful_widget(table, area, &mut render_state);
}
