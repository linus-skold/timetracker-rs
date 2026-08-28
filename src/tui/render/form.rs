use crate::tui::text_input::TextInput;
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
        app.search_term.value()
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
        f.set_cursor_position((
            area.x + Line::from(app.search_term.before_cursor()).width() as u16 + 1,
            area.y + 1,
        ));
    }
}

/// One form row: the variant, its block label, and the buffer it edits.
type FormField = (InputField, &'static str, fn(&App) -> &TextInput);

/// Every form field in layout order. Row order here *is* the on-screen order,
/// the Tab order and the order of the layout chunks, so adding or moving a
/// field is one row.
const FIELDS: &[FormField] = &[
    (InputField::Description, " Description ", |a| {
        &a.input_description
    }),
    (
        InputField::Project,
        " Project (optional: single name, e.g. acme) ",
        |a| &a.input_project,
    ),
    (
        InputField::Tags,
        " Tags (space-separated, e.g., work meeting) ",
        |a| &a.input_tags,
    ),
    (
        InputField::Duration,
        " Duration (optional: 1h30m, 45m, 2h) ",
        |a| &a.input_duration,
    ),
    (
        InputField::StartTime,
        " Start Time (e.g. 9am, 14:30, 25/03 9.30am) ",
        |a| &a.input_start_time,
    ),
    (
        InputField::EndTime,
        " End Time (optional: e.g. 9am, 14:30, 25/03 9.30am) ",
        |a| &a.input_end_time,
    ),
];

/// The chunk the help row sits in: straight after the last field.
const HELP_CHUNK: usize = FIELDS.len();

/// One three-line chunk per field, then the help row, then the slack.
fn form_constraints() -> Vec<Constraint> {
    let mut constraints = vec![Constraint::Length(3); FIELDS.len() + 1];
    constraints.push(Constraint::Min(0));
    constraints
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
        .constraints(form_constraints())
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

    for (i, (field, label, input)) in FIELDS.iter().copied().enumerate() {
        f.render_widget(
            Paragraph::new(input(app).value())
                .style(Style::default().fg(Color::White))
                .block(field_block(label, active == field)),
            chunks[i],
        );
    }

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
    f.render_widget(help, chunks[HELP_CHUNK]);

    if let Some((i, (_, _, input))) = FIELDS
        .iter()
        .copied()
        .enumerate()
        .find(|(_, (field, _, _))| *field == active)
    {
        let width = Line::from(input(app).before_cursor()).width() as u16;
        f.set_cursor_position((chunks[i].x + width + 1, chunks[i].y + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constraints_follow_the_field_table() {
        let constraints = form_constraints();
        // one chunk per field, then help, then the slack
        assert_eq!(constraints.len(), FIELDS.len() + 2);
        assert!(
            constraints[..=HELP_CHUNK]
                .iter()
                .all(|c| *c == Constraint::Length(3))
        );
        assert_eq!(constraints[constraints.len() - 1], Constraint::Min(0));
    }

    #[test]
    fn the_table_holds_every_field_in_tab_order() {
        let rows: Vec<InputField> = FIELDS.iter().map(|(field, _, _)| *field).collect();
        assert_eq!(
            rows,
            vec![
                InputField::Description,
                InputField::Project,
                InputField::Tags,
                InputField::Duration,
                InputField::StartTime,
                InputField::EndTime,
            ]
        );
    }
}
