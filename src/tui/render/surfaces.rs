use super::heat::{heat_axis, relative_heat_legend};
use super::legend::legend;
use super::overlay::CURSOR_MARKER;
use crate::tui::panes::Polarity;
use crate::tui::summary::{
    SUMMARY_TOTAL_LINES, strip_cells, summary_total, visible_project_summary,
};
use crate::tui::types::Pane;
use crate::tui::{App, theme};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

/// The `Marks` surface: `project/issue phase`, start time, elapsed, and the mark's
/// last heartbeat, newest first. The last-seen wording and the `[stale]` marker are
/// `marks::rows_at`'s; the columns are the panel's own. Liveness comes from `App.leases`.
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

    let leases = app.visible_leases();
    // One label column for the whole box, so the times read as columns.
    let label_width = leases
        .iter()
        .map(|lease| lease.mark.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(LABEL_WIDTH);

    let now = chrono::Local::now();

    let lines: Vec<Line> = if leases.is_empty() {
        vec![Line::from(Span::styled(
            " no phases in progress",
            Style::default().fg(theme::inactive()).italic(),
        ))]
    } else {
        leases
            .iter()
            .map(|lease| {
                let label = lease.mark.label();
                let pad = " ".repeat(label_width.saturating_sub(label.chars().count()));
                let stale = lease.is_expired_at(now, app.liveness_thresholds);
                Line::from(vec![
                    Span::styled(
                        format!(" {}{}", label, pad),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" {}", lease.mark.started_at()),
                        Style::default().fg(theme::inactive()),
                    ),
                    Span::styled(
                        format!("   ({})", lease.mark.elapsed()),
                        Style::default().fg(theme::highlight()),
                    ),
                    Span::styled(
                        format!("  last seen {}", lease.last_seen_at()),
                        Style::default().fg(theme::inactive()),
                    ),
                    Span::styled(
                        if stale { " [stale]" } else { "" }.to_string(),
                        Style::default().fg(theme::accent()),
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

/// The `Summary` surface: how the folded entries split across projects. The
/// title bar's marker names the mode and takes its colour from the same
/// predicate the footer's total uses.
pub(super) fn render_summary_surface(f: &mut Frame, app: &App, area: Rect) {
    /// The right-flushed number columns. Fixed, not content-derived, so a
    /// re-scope that widens one figure cannot shift them.
    const TOTAL_WIDTH: usize = 9;
    const HUMAN_WIDTH: usize = 9;
    const AGENT_WIDTH: usize = 9;
    // 6, not 5: `count` would touch `total`.
    const COUNT_WIDTH: usize = 6;
    const SHARE_WIDTH: usize = 6;
    /// The header over the project column, and the column's floor.
    const LABEL_HEADER: &str = "project";
    /// Fewest cells worth drawing: a shorter strip reads as noise, not a shape.
    const MIN_STRIP_CELLS: usize = 6;

    let focused = app.summary_is_focused();
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused {
            theme::accent()
        } else {
            theme::border()
        }))
        .title(Span::styled(
            " Summary (S) ",
            Style::default().fg(if focused {
                theme::highlight()
            } else {
                theme::title()
            }),
        ));
    let inner = block.inner(area);

    // One fold for the whole frame: the marker and the rows read the same list.
    let summary = app.project_summary();
    // Budget excludes the header and total lines; see `summary_surface_height`.
    let scoped = !summary.is_empty();
    let visible_rows = if scoped {
        (inner.height as usize).saturating_sub(1 + SUMMARY_TOTAL_LINES as usize)
    } else {
        inner.height as usize
    };

    let marker_style = Style::default().fg(if app.total_is_filtered() {
        theme::highlight()
    } else {
        theme::title()
    });
    block = block.title_top(
        Line::from(Span::styled(
            format!(" {} ", app.summary_marker(&summary, visible_rows)),
            marker_style,
        ))
        .right_aligned(),
    );

    // A key stays accented off-focus while its own mode is on, as `S` does.
    let mut keys = vec![("f", "filter", focused || app.summary_follows_filters)];
    if app.show_summary {
        keys.insert(0, ("v", "split", focused || app.summary_split));
        keys.push(("m", "heat", focused || app.summary_heat));
    }
    let keys = legend(&keys, inner.width);
    let keys_width = keys.as_ref().map(|line| line.width()).unwrap_or(0) as u16;
    if let Some(keys) = keys {
        block = block.title_bottom(keys.left_aligned());
    }

    // Both conditions: an empty day must not blame a filter nobody set.
    let empty_text = if app.summary_follows_filters && app.total_is_filtered() {
        " nothing matches the filter"
    } else {
        " nothing in scope"
    };
    let total_line = |label_width: usize| {
        let sum = summary_total(&summary);
        let mut spans = vec![
            Span::styled(
                format!(" {:<label_width$}", sum.project),
                Style::default().fg(theme::title()),
            ),
            Span::styled(
                format!("{:>TOTAL_WIDTH$}", crate::duration::format(sum.total)),
                Style::default().fg(Color::White).bold(),
            ),
        ];
        if app.summary_split {
            spans.push(Span::styled(
                format!("{:>HUMAN_WIDTH$}", crate::duration::format(sum.human)),
                Style::default().fg(theme::active()).bold(),
            ));
            spans.push(Span::styled(
                format!("{:>AGENT_WIDTH$}", crate::duration::format(sum.agent)),
                Style::default().fg(theme::highlight()).bold(),
            ));
        }
        spans.push(Span::styled(
            format!("{:>COUNT_WIDTH$}", sum.entries),
            Style::default().fg(theme::inactive()),
        ));
        Line::from(spans)
    };

    // Drawn on the border, so the strip costs no row of the box.
    let mut heat_legend: Option<Line> = None;

    let lines: Vec<Line> = if !app.show_summary {
        // The footer's old total, one line: label, sum, nothing else.
        let label = if app.summary_follows_filters && app.total_is_filtered() {
            " Filtered total: "
        } else {
            " Total: "
        };
        vec![Line::from(vec![
            Span::styled(label, Style::default().fg(theme::title())),
            Span::styled(
                crate::duration::format(summary_total(&summary).total),
                Style::default().fg(theme::highlight()).bold(),
            ),
        ])]
    } else if !scoped {
        vec![Line::from(Span::styled(
            empty_text,
            Style::default().fg(theme::inactive()).italic(),
        ))]
    } else {
        let rows = visible_project_summary(&summary, visible_rows);
        // One project column for the whole box, so the numbers read as columns.
        let label_width = rows
            .iter()
            .map(|row| row.project.chars().count())
            .max()
            .unwrap_or(0)
            .max(LABEL_HEADER.chars().count());

        // The rule under the numbers and the strip's budget read one width.
        let mut numbers_width = TOTAL_WIDTH + COUNT_WIDTH + SHARE_WIDTH;
        if app.summary_split {
            numbers_width += HUMAN_WIDTH + AGENT_WIDTH;
        }

        // The columns left of the border, right of the strip's own separator.
        // The model holds every bucket; the free width decides how many are
        // drawn and how wide.
        let free = (inner.width as usize)
            .saturating_sub(1 + label_width + numbers_width + strip_separator().width());
        let grid = app.project_buckets(rows);
        let (cell_width, cells) = strip_cells(free, grid.len());
        let first = grid.len() - cells;
        let strip = app.summary_heat && cells >= MIN_STRIP_CELLS;
        // One maximum over every cell drawn, so the rows stay comparable.
        let peak = grid
            .rows
            .iter()
            .flat_map(|row| row[first..].iter())
            .map(|cell| cell.num_seconds())
            .max()
            .unwrap_or(0);

        let mut header = format!(" {LABEL_HEADER:<label_width$}{:>TOTAL_WIDTH$}", "total");
        if app.summary_split {
            header.push_str(&format!("{:>HUMAN_WIDTH$}", "human"));
            header.push_str(&format!("{:>AGENT_WIDTH$}", "agent"));
        }
        header.push_str(&format!("{:>COUNT_WIDTH$}", "count"));
        header.push_str(&format!("{:>SHARE_WIDTH$}", "share"));

        let mut header_spans = vec![Span::styled(
            header,
            Style::default().fg(theme::inactive()).italic(),
        )];
        if strip {
            // The ticks head the strip as the column names head the numbers.
            header_spans.push(strip_separator());
            header_spans.push(Span::styled(
                heat_axis(&grid, first, cells, cell_width),
                Style::default().fg(theme::inactive()),
            ));
        }

        let mut lines = vec![Line::from(header_spans)];
        lines.extend(rows.iter().enumerate().map(|(index, row)| {
            let pad = " ".repeat(label_width.saturating_sub(row.project.chars().count()));
            let mut spans = vec![
                Span::styled(
                    format!(" {}{}", row.project, pad),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:>TOTAL_WIDTH$}", crate::duration::format(row.total)),
                    Style::default().fg(Color::White),
                ),
            ];
            if app.summary_split {
                spans.push(Span::styled(
                    format!("{:>HUMAN_WIDTH$}", crate::duration::format(row.human)),
                    Style::default().fg(theme::active()),
                ));
                spans.push(Span::styled(
                    format!("{:>AGENT_WIDTH$}", crate::duration::format(row.agent)),
                    Style::default().fg(theme::highlight()),
                ));
            }
            spans.push(Span::styled(
                format!("{:>COUNT_WIDTH$}", row.entries),
                Style::default().fg(theme::inactive()),
            ));
            spans.push(Span::styled(
                format!("{:>SHARE_WIDTH$}", format!("{}%", row.share)),
                Style::default().fg(theme::accent()),
            ));
            if strip {
                spans.push(strip_separator());
                spans.extend(grid.rows[index][first..].iter().map(|cell| {
                    let seconds = cell.num_seconds();
                    // An empty bucket keeps the surface behind it, so a strip
                    // reads as its own sparse timeline, not as a filled grid.
                    if seconds <= 0 {
                        return Span::raw(" ".repeat(cell_width));
                    }
                    Span::styled(
                        " ".repeat(cell_width),
                        Style::default().bg(theme::heat_shade(seconds, peak)),
                    )
                }));
            }
            Line::from(spans)
        }));

        if strip {
            heat_legend = relative_heat_legend(keys_width, inner.width);
        }

        // The rule sits under the number columns only, as a hand sum does.
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(label_width + 1)),
            Span::styled(
                "─".repeat(numbers_width),
                Style::default().fg(theme::border()),
            ),
        ]));
        lines.push(total_line(label_width));
        lines
    };

    if let Some(heat_legend) = heat_legend {
        block = block.title_bottom(heat_legend.right_aligned());
    }

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// What holds the strip off the `share` column: two blanks, a rule of the
/// border's own, two blanks. Drawn on the header and on every project row, so
/// the rule runs the height of the strip beside it.
fn strip_separator() -> Span<'static> {
    Span::styled("  │  ", Style::default().fg(theme::border()))
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

    // Ordered most useful first: the helper sheds from the end.
    if let Some(keys) = legend(
        &[
            ("Enter", "filter", focused),
            ("-", "back", focused),
            ("j/k", "move", focused),
        ],
        inner.width,
    ) {
        block = block.title_bottom(keys.left_aligned());
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
