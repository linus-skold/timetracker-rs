use super::overlay::CURSOR_MARKER;
use crate::tui::panes::Polarity;
use crate::tui::types::Pane;
use crate::tui::{App, theme};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

/// The `Marks` surface: `project/issue phase`, start time and elapsed, newest
/// first. Rows come from `crate::marks` so the CLI and the TUI cannot disagree;
/// elapsed is asked for per frame, so it counts up between directory reads.
pub(super) fn render_marks_surface(f: &mut Frame, app: &App, area: Rect) {
    /// Narrowest the label column gets, so short labels still line their times up.
    const LABEL_WIDTH: usize = 18;

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::border()))
        .title(Span::styled(
            " Agents (A) ",
            Style::default().fg(theme::title()),
        ));
    let inner = block.inner(area);

    // Driven off the rows this frame really has, not a constant.
    if let Some(count) = app.marks_count(inner.height as usize) {
        block = block.title_top(
            Line::from(Span::styled(
                format!(" {} ", count),
                Style::default().fg(theme::inactive()),
            ))
            .right_aligned(),
        );
    }

    let marks = app.visible_marks();
    // One label column for the whole box, so the times read as columns.
    let label_width = marks
        .iter()
        .map(|mark| mark.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(LABEL_WIDTH);

    let lines: Vec<Line> = if marks.is_empty() {
        vec![Line::from(Span::styled(
            " no phases in progress",
            Style::default().fg(theme::inactive()).italic(),
        ))]
    } else {
        marks
            .iter()
            .map(|mark| {
                let label = mark.label();
                let pad = " ".repeat(label_width.saturating_sub(label.chars().count()));
                Line::from(vec![
                    Span::styled(
                        format!(" {}{}", label, pad),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" {}", mark.started_at()),
                        Style::default().fg(theme::inactive()),
                    ),
                    Span::styled(
                        format!("   ({})", mark.elapsed()),
                        Style::default().fg(theme::highlight()),
                    ),
                ])
            })
            .collect()
    };

    let mut lines = lines;
    if !app.unaccounted.is_empty() {
        let header = match app.unaccounted_count() {
            Some(count) => format!(" ⚠ unaccounted activity ({count})"),
            None => " ⚠ unaccounted activity".to_string(),
        };
        lines.push(Line::from(Span::styled(
            header,
            Style::default().fg(theme::inactive()).italic(),
        )));
        for item in app.visible_unaccounted() {
            lines.push(Line::from(Span::styled(
                format!(" {}", item.describe()),
                Style::default().fg(theme::highlight()),
            )));
        }
    }

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The `Summary` surface: how the current scope split across projects. The title
/// bar's `all projects` marker keeps its words in both filter states and changes
/// only colour, off the same predicate the footer's total uses.
pub(super) fn render_summary_surface(f: &mut Frame, app: &App, area: Rect) {
    /// Narrowest the project column gets, so short names still line their totals up.
    const LABEL_WIDTH: usize = 14;
    /// The three right-flushed number columns. Fixed, not content-derived, so a
    /// re-scope that widens one figure cannot shift them.
    const TOTAL_WIDTH: usize = 8;
    const COUNT_WIDTH: usize = 5;
    const SHARE_WIDTH: usize = 6;

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::border()))
        .title(Span::styled(
            " Summary (S) ",
            Style::default().fg(theme::title()),
        ));
    let inner = block.inner(area);

    let marker_style = Style::default().fg(if app.total_is_filtered() {
        theme::highlight()
    } else {
        theme::title()
    });
    block = block.title_top(
        Line::from(Span::styled(
            format!(" {} ", app.summary_marker(inner.height as usize)),
            marker_style,
        ))
        .right_aligned(),
    );

    let rows = app.visible_project_summary(inner.height as usize);
    // One project column for the whole box, so the numbers read as columns.
    let label_width = rows
        .iter()
        .map(|row| row.project.chars().count())
        .max()
        .unwrap_or(0)
        .max(LABEL_WIDTH);

    let lines: Vec<Line> = if rows.is_empty() {
        vec![Line::from(Span::styled(
            " nothing in scope",
            Style::default().fg(theme::inactive()).italic(),
        ))]
    } else {
        rows.iter()
            .map(|row| {
                let pad = " ".repeat(label_width.saturating_sub(row.project.chars().count()));
                Line::from(vec![
                    Span::styled(
                        format!(" {}{}", row.project, pad),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!("{:>TOTAL_WIDTH$}", crate::duration::format(row.total)),
                        Style::default().fg(theme::highlight()),
                    ),
                    Span::styled(
                        format!("{:>COUNT_WIDTH$}", row.entries),
                        Style::default().fg(theme::inactive()),
                    ),
                    Span::styled(
                        format!("{:>SHARE_WIDTH$}", format!("{}%", row.share)),
                        Style::default().fg(theme::accent()),
                    ),
                ])
            })
            .collect()
    };

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The pane surface: both panes side by side, or the single open one full width.
pub(super) fn render_pane_surface(f: &mut Frame, app: &App, area: Rect) {
    let panes = app.visible_panes();
    let share = panes.len() as u32;
    let constraints: Vec<Constraint> = panes.iter().map(|_| Constraint::Ratio(1, share)).collect();
    let areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    for (pane, pane_area) in panes.iter().zip(areas.iter()) {
        render_pane(f, app, *pane, *pane_area);
    }
}

fn render_pane(f: &mut Frame, app: &App, pane: Pane, area: Rect) {
    let focused = app.focused_pane() == Some(pane);
    let values = app.pane_values(pane);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused {
            theme::accent()
        } else {
            theme::border()
        }))
        .title(Span::styled(
            pane.title(),
            Style::default().fg(if focused {
                theme::highlight()
            } else {
                theme::title()
            }),
        ));

    let inner = block.inner(area);

    // On the top border, which is dead space: costs no row and no inner column.
    if let Some(indicator) = app.pane_scroll_indicator(pane, inner.height as usize) {
        block = block.title_top(
            Line::from(Span::styled(
                format!(" {} ", indicator),
                Style::default().fg(theme::inactive()),
            ))
            .right_aligned(),
        );
    }

    // The marker is a gutter on every row, so it comes off the rows' layout width.
    let width = (inner.width as usize).saturating_sub(CURSOR_MARKER.len());
    let items: Vec<ListItem> = if values.is_empty() {
        vec![ListItem::new(Span::styled(
            " nothing in view",
            Style::default().fg(theme::inactive()).italic(),
        ))]
    } else {
        values
            .iter()
            .map(|(value, count)| {
                // The lead column carries the filter mark, so it costs no width.
                // Both marks are one ASCII column: `used` below counts on it.
                let state = app.pane_value_state(pane, value);
                let (mark, value_style) = match state {
                    Some(Polarity::Include) => ("•", Style::default().fg(theme::accent()).bold()),
                    Some(Polarity::Exclude) => ("-", Style::default().fg(theme::inactive()).bold()),
                    None => (" ", Style::default().fg(Color::White)),
                };
                let count = count.to_string();
                let used = 1 + value.chars().count() + count.chars().count() + 1;
                let gap = width.saturating_sub(used).max(1);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{}{}", mark, value), value_style),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(count, Style::default().fg(theme::highlight())),
                ]))
            })
            .collect()
    };

    // Shown with or without focus: it says where this pane's cursor will resume.
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(theme::selected_bg()))
        .highlight_symbol(CURSOR_MARKER);
    let mut state = ListState::default();
    if !values.is_empty() {
        state.select(Some(app.pane_cursor(pane)));
    }
    f.render_stateful_widget(list, area, &mut state);
}
