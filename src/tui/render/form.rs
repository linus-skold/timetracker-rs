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
    (
        InputField::Data,
        " Data (optional: a JSON object, e.g. {\"pr\": 42}) ",
        |a| &a.input_data,
    ),
];

/// A field block is a border, one line of text and a border. Always — a
/// squeezed box loses its label or its text, so when the form is taller than
/// the area it scrolls instead, a whole field at a time.
const FIELD_HEIGHT: u16 = 3;

/// Which slice of [`FIELDS`] the area has room for, and where to put it.
#[derive(Debug, PartialEq, Eq)]
struct FormViewport {
    /// Rows left blank above and below the form, dropped as the area shrinks.
    margin: u16,
    /// Index of the first field drawn.
    first: usize,
    /// How many fields are drawn, always at least one.
    count: usize,
    /// Whether the pinned help row at the bottom still fits.
    show_help: bool,
}

impl FormViewport {
    /// True while some field is off-screen in either direction.
    fn scrolls(&self) -> bool {
        self.first > 0 || self.first + self.count < FIELDS.len()
    }
}

/// Fit the form into `height` rows, keeping the active field on screen. The
/// scroll offset is derived from the cursor rather than stored: the active
/// field is the only thing that moves the view, so there is no state to keep
/// in sync with a resize.
fn form_viewport(height: u16, active: usize) -> FormViewport {
    let full = FIELD_HEIGHT * (FIELDS.len() as u16 + 1);
    let margin = match height {
        h if h >= full + 4 => 2,
        h if h >= full + 2 => 1,
        _ => 0,
    };
    let inner = height.saturating_sub(margin * 2);

    // The help row is the last thing to go: below two blocks' worth of rows
    // there is no room for both it and a field, and the field wins.
    let show_help = inner >= FIELD_HEIGHT * 2;
    let for_fields = if show_help {
        inner - FIELD_HEIGHT
    } else {
        inner
    };
    let count = ((for_fields / FIELD_HEIGHT) as usize).clamp(1, FIELDS.len());
    let first = active.saturating_sub(count - 1).min(FIELDS.len() - count);

    FormViewport {
        margin,
        first,
        count,
        show_help,
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

    let active = app.input_field;
    let active_index = FIELDS
        .iter()
        .position(|(field, _, _)| *field == active)
        .unwrap_or(0);
    let view = form_viewport(area.height, active_index);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + view.margin,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(view.margin * 2),
    };
    let field_area = |slot: usize| Rect {
        x: inner.x,
        y: inner.y + slot as u16 * FIELD_HEIGHT,
        width: inner.width,
        height: FIELD_HEIGHT,
    };

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

    for (slot, (field, label, input)) in FIELDS[view.first..view.first + view.count]
        .iter()
        .copied()
        .enumerate()
    {
        f.render_widget(
            Paragraph::new(input(app).value())
                .style(Style::default().fg(Color::White))
                .block(field_block(label, active == field)),
            field_area(slot),
        );
    }

    // A refused save replaces the hint: the row says why nothing was written
    // rather than the form appearing to ignore Enter.
    let trailing = match &app.form_error {
        // The same colour onboarding reports its own failure in.
        Some(message) => Span::styled(
            message.clone(),
            Style::default().fg(theme::theme().duration_high).bold(),
        ),
        None => Span::styled(
            "Need ≥2 of: Start, End, Duration".to_string(),
            Style::default().fg(theme::border()),
        ),
    };
    // Scrolled, the form no longer shows how far through it the cursor is, so
    // the title says so.
    let title = if view.scrolls() {
        format!("{}({}/{}) ", form_title, active_index + 1, FIELDS.len())
    } else {
        form_title
    };
    let help = Paragraph::new(Line::from(vec![
        Span::styled("Tab/↑↓", Style::default().fg(theme::accent())),
        Span::styled(": switch field | ", Style::default().fg(theme::inactive())),
        Span::styled("Enter", Style::default().fg(theme::accent())),
        Span::styled(": save | ", Style::default().fg(theme::inactive())),
        Span::styled("Esc", Style::default().fg(theme::accent())),
        Span::styled(": cancel  ", Style::default().fg(theme::inactive())),
        trailing,
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(title, Style::default().fg(theme::highlight()))),
    );
    // Straight under the last drawn field, as it sits in a form that fits.
    if view.show_help {
        f.render_widget(help, field_area(view.count));
    }

    // The viewport always keeps the active field on screen, so the cursor is
    // always in one of the drawn slots.
    if let Some((_, _, input)) = FIELDS.get(active_index).copied() {
        let slot = field_area(active_index - view.first);
        let width = Line::from(input(app).before_cursor()).width() as u16;
        f.set_cursor_position((slot.x + width + 1, slot.y + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The height that fits every field, the help row and the full margin.
    fn roomy() -> u16 {
        FIELD_HEIGHT * (FIELDS.len() as u16 + 1) + 4
    }

    #[test]
    fn a_tall_area_shows_the_whole_form_unscrolled() {
        let view = form_viewport(roomy(), 0);
        assert_eq!(
            view,
            FormViewport {
                margin: 2,
                first: 0,
                count: FIELDS.len(),
                show_help: true,
            }
        );
        assert!(!view.scrolls());

        // Extra rows are slack, not more fields.
        assert_eq!(form_viewport(roomy() + 20, 6), view);
    }

    #[test]
    fn the_margin_gives_way_before_any_field_does() {
        for (height, margin) in [(roomy(), 2), (roomy() - 2, 1), (roomy() - 4, 0)] {
            let view = form_viewport(height, 0);
            assert_eq!(view.margin, margin, "at {height} rows");
            assert_eq!(view.count, FIELDS.len(), "at {height} rows");
            assert!(!view.scrolls(), "at {height} rows");
        }
    }

    #[test]
    fn a_short_area_scrolls_the_active_field_into_view() {
        // Room for the help row and three fields, nothing more.
        let height = FIELD_HEIGHT * 4;

        // The cursor near the top keeps the form at the top.
        for active in 0..3 {
            let view = form_viewport(height, active);
            assert_eq!((view.first, view.count), (0, 3), "field {active}");
            assert!(view.scrolls());
        }

        // Past the last visible slot the view follows it down, one field at a
        // time, and stops at the end of the table.
        assert_eq!(form_viewport(height, 3).first, 1);
        assert_eq!(form_viewport(height, 4).first, 2);
        assert_eq!(
            form_viewport(height, FIELDS.len() - 1).first,
            FIELDS.len() - 3
        );
    }

    #[test]
    fn the_help_row_is_dropped_before_the_last_field() {
        let two = form_viewport(FIELD_HEIGHT * 2, 6);
        assert_eq!((two.count, two.show_help), (1, true));

        let one = form_viewport(FIELD_HEIGHT, 6);
        assert_eq!((one.count, one.show_help), (1, false));
        assert_eq!(one.first, FIELDS.len() - 1, "the active field is drawn");

        // Even with no room at all the slice stays non-empty rather than
        // panicking on an empty range.
        let none = form_viewport(0, 0);
        assert_eq!((none.first, none.count), (0, 1));
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
                InputField::Data,
            ]
        );
    }
}
