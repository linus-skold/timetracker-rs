use crate::tui::types::{InputField, InputMode};
use crate::tui::{App, theme};
use chrono::Local;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

pub(super) fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let is_active = app.input_mode == InputMode::Searching;
    let border_style = if is_active {
        Style::default().fg(theme::accent())
    } else {
        Style::default().fg(theme::border())
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
                Style::default().fg(theme::highlight())
            } else {
                Style::default().fg(theme::title())
            },
        ));

    let search_text = if is_active && app.search_term.is_empty() {
        "Type to search... (Enter to confirm, Esc to clear)"
    } else {
        &app.search_term
    };

    let search_input = Paragraph::new(search_text)
        .style(if app.search_term.is_empty() && is_active {
            Style::default().fg(theme::inactive()).italic()
        } else {
            Style::default().fg(Color::White)
        })
        .block(search_block);
    f.render_widget(search_input, area);

    if is_active {
        let byte_idx = app
            .search_term
            .char_indices()
            .nth(app.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(app.search_term.len());
        f.set_cursor_position((
            area.x + Line::from(&app.search_term[..byte_idx]).width() as u16 + 1,
            area.y + 1,
        ));
    }
}

pub(super) fn render_entry_form(f: &mut Frame, app: &App, area: Rect) {
    let is_editing = app.input_mode == InputMode::EditingEntry;
    let form_title = if is_editing {
        " Edit Entry ".to_string()
    } else {
        let today = Local::now().date_naive();
        if app.selected_date == today {
            " Add Log Entry ".to_string()
        } else {
            format!(
                " Add Log Entry — {} ",
                app.selected_date.format("%a, %d %b %Y")
            )
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Description
            Constraint::Length(3), // Project
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
                Style::default().fg(theme::accent())
            } else {
                Style::default().fg(theme::inactive())
            })
            .title(Span::styled(
                label,
                if active {
                    Style::default().fg(theme::highlight())
                } else {
                    Style::default().fg(theme::title())
                },
            ))
    }

    let active = app.input_field;

    f.render_widget(
        Paragraph::new(app.input_description.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(
                " Description ",
                active == InputField::Description,
            )),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(app.input_project.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(
                " Project (optional: single name, e.g. acme) ",
                active == InputField::Project,
            )),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(app.input_tags.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(
                " Tags (space-separated, e.g., work meeting) ",
                active == InputField::Tags,
            )),
        chunks[2],
    );
    f.render_widget(
        Paragraph::new(app.input_duration.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(
                " Duration (optional: 1h30m, 45m, 2h) ",
                active == InputField::Duration,
            )),
        chunks[3],
    );
    f.render_widget(
        Paragraph::new(app.input_start_time.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(
                " Start Time (e.g. 9am, 14:30, 25/03 9.30am) ",
                active == InputField::StartTime,
            )),
        chunks[4],
    );
    f.render_widget(
        Paragraph::new(app.input_end_time.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(
                " End Time (optional: e.g. 9am, 14:30, 25/03 9.30am) ",
                active == InputField::EndTime,
            )),
        chunks[5],
    );

    let help = Paragraph::new(Line::from(vec![
        Span::styled("Tab", Style::default().fg(theme::accent())),
        Span::styled(": switch field | ", Style::default().fg(theme::inactive())),
        Span::styled("Enter", Style::default().fg(theme::accent())),
        Span::styled(": save | ", Style::default().fg(theme::inactive())),
        Span::styled("Esc", Style::default().fg(theme::accent())),
        Span::styled(": cancel  ", Style::default().fg(theme::inactive())),
        Span::styled(
            "Need ≥2 of: Start, End, Duration",
            Style::default().fg(theme::border()),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(
                form_title,
                Style::default().fg(theme::highlight()),
            )),
    );
    f.render_widget(help, chunks[6]);

    let cursor_text_width = |text: &str, pos: usize| -> u16 {
        let byte_idx = text
            .char_indices()
            .nth(pos)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        Line::from(&text[..byte_idx]).width() as u16
    };
    let (cursor_x, cursor_y) = match app.input_field {
        InputField::Description => (
            chunks[0].x + cursor_text_width(&app.input_description, app.cursor_pos) + 1,
            chunks[0].y + 1,
        ),
        InputField::Project => (
            chunks[1].x + cursor_text_width(&app.input_project, app.cursor_pos) + 1,
            chunks[1].y + 1,
        ),
        InputField::Tags => (
            chunks[2].x + cursor_text_width(&app.input_tags, app.cursor_pos) + 1,
            chunks[2].y + 1,
        ),
        InputField::Duration => (
            chunks[3].x + cursor_text_width(&app.input_duration, app.cursor_pos) + 1,
            chunks[3].y + 1,
        ),
        InputField::StartTime => (
            chunks[4].x + cursor_text_width(&app.input_start_time, app.cursor_pos) + 1,
            chunks[4].y + 1,
        ),
        InputField::EndTime => (
            chunks[5].x + cursor_text_width(&app.input_end_time, app.cursor_pos) + 1,
            chunks[5].y + 1,
        ),
    };
    f.set_cursor_position((cursor_x, cursor_y));
}
