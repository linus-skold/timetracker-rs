use super::overlay::CURSOR_MARKER;
use crate::tracker::TimeData;
use crate::tui::panes::Polarity;
use crate::tui::types::ViewMode;
use crate::tui::{App, theme};
use chrono::{Datelike, Duration, Local, NaiveDate};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use std::collections::HashMap;

/// A GitHub-style yearly contribution heatmap: one column per week, one row
/// per weekday, each cell shaded by `theme::heat_color` for that day's total.
pub(super) fn render_overview(f: &mut Frame, app: &App, area: Rect) {
    const GUTTER: usize = 4; // weekday label width, e.g. "Mon "
    const WEEKDAY_LABELS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

    let year = app.selected_date.year();
    let breakdown = app.data.year_breakdown(year);
    let today = Local::now().date_naive();

    let grid_start = TimeData::week_start(NaiveDate::from_ymd_opt(year, 1, 1).unwrap());
    let grid_end =
        TimeData::week_start(NaiveDate::from_ymd_opt(year, 12, 31).unwrap()) + Duration::days(6);
    let total_weeks = ((grid_end - grid_start).num_days() / 7 + 1) as usize;

    // Show only as many of the most recent weeks as fit the available width,
    // rather than silently clipping mid-cell on a narrow terminal.
    let available_weeks = ((area.width as usize).saturating_sub(GUTTER) / 2).max(1);
    let visible_weeks = total_weeks.min(available_weeks);
    let first_week = total_weeks - visible_weeks;

    // Month header: a fixed-width char grid, so a 3-char abbreviation can
    // overlap slightly into the next column, same as it does on github.com.
    let mut header_chars = vec![' '; GUTTER + visible_weeks * 2];
    for w in 0..visible_weeks {
        let monday = grid_start + Duration::days(((first_week + w) * 7) as i64);
        if monday.year() == year && monday.day() <= 7 {
            let col = GUTTER + w * 2;
            for (i, ch) in monday.format("%b").to_string().chars().enumerate() {
                if let Some(slot) = header_chars.get_mut(col + i) {
                    *slot = ch;
                }
            }
        }
    }
    let mut lines = vec![Line::from(Span::styled(
        header_chars.into_iter().collect::<String>(),
        Style::default().fg(theme::inactive()),
    ))];

    for (row, label) in WEEKDAY_LABELS.iter().enumerate() {
        let mut spans = vec![Span::styled(
            format!("{:<width$}", label, width = GUTTER),
            Style::default().fg(theme::inactive()),
        )];
        for w in 0..visible_weeks {
            let date = grid_start + Duration::days(((first_week + w) * 7 + row) as i64);
            if date.year() != year {
                spans.push(Span::raw("  "));
                continue;
            }
            let hours = breakdown.get(&date).map(|d| d.num_hours()).unwrap_or(0);
            let cell_style = Style::default().bg(theme::heat_color(hours));
            if date == today {
                spans.push(Span::styled(" ●", cell_style.fg(theme::highlight()).bold()));
            } else {
                spans.push(Span::styled("  ", cell_style));
            }
        }
        lines.push(Line::from(spans));
    }

    // Legend: "Less" -> five progressively hotter swatches -> "More".
    let t = theme::theme();
    let mut legend_spans = vec![
        Span::raw(" ".repeat(GUTTER)),
        Span::styled("Less ", Style::default().fg(theme::inactive())),
    ];
    for hours in [
        0,
        1,
        t.day_duration_med_h / 2,
        t.day_duration_med_h,
        t.day_duration_high_h,
    ] {
        legend_spans.push(Span::styled(
            "  ",
            Style::default().bg(theme::heat_color(hours)),
        ));
    }
    legend_spans.push(Span::styled(
        " More",
        Style::default().fg(theme::inactive()),
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(legend_spans));

    let total = breakdown.values().fold(Duration::zero(), |acc, d| acc + *d);
    let active_days = breakdown.len();
    let title = format!(
        " {} tracked over {} active day{} ",
        crate::duration::format(total),
        active_days,
        if active_days == 1 { "" } else { "s" }
    );

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(title, Style::default().fg(theme::title()))),
    );
    f.render_widget(paragraph, area);
}

pub(super) fn render_weekly_breakdown(f: &mut Frame, app: &App, area: Rect) {
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

            let dur_color = theme::duration_color(
                hours,
                theme::theme().day_duration_high_h,
                theme::theme().day_duration_med_h,
            );

            let (day_style, date_style) = if is_today {
                (
                    Style::default().fg(theme::highlight()).bold(),
                    Style::default().fg(theme::highlight()),
                )
            } else {
                (
                    Style::default().fg(theme::accent()),
                    Style::default().fg(theme::title()),
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
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(
                " Daily Totals ",
                Style::default().fg(theme::title()),
            )),
    );
    f.render_widget(table, area);
}

fn entry_row(entry: &crate::tracker::TimeEntry, stripe: bool) -> Row<'_> {
    let hours = entry.duration().num_hours();
    let dur_color = theme::duration_color(
        hours,
        theme::theme().entry_duration_high_h,
        theme::theme().entry_duration_med_h,
    );

    let status_style = if entry.is_active() {
        Style::default().fg(theme::active())
    } else {
        Style::default().fg(theme::inactive())
    };

    let row_style = if stripe {
        Style::default().bg(Color::Rgb(35, 35, 35))
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(entry.format_date()).style(Style::default().fg(theme::title())),
        Cell::from(entry.format_start_time()).style(Style::default().fg(theme::accent())),
        Cell::from(entry.format_end_time()).style(Style::default().fg(theme::inactive())),
        Cell::from(entry.description.clone()),
        Cell::from(entry.format_tags()).style(Style::default().fg(theme::highlight())),
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
                .fg(theme::highlight())
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
        Cell::from(""),
        Cell::from(date_str).style(Style::default().fg(theme::title())),
        Cell::from(""),
        Cell::from(total_str).style(
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
    ])
    .height(2)
    .style(Style::default().bg(theme::DAY_HEADER_BG))
}

pub(super) fn render_entries_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = [
        "Date",
        "Start",
        "End",
        "Description",
        "Tags",
        "Duration",
        "",
    ]
    .into_iter()
    .map(|h| {
        Cell::from(h).style(
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD),
        )
    });
    let header_row = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme::header_bg()));

    let entries = app.filtered_entries();

    let (rows, visual_selected): (Vec<Row>, Option<usize>) = if app.view_mode == ViewMode::Week {
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

    // `(tt)` and `#impl`, the CLI's own sigils, so the title needs no legend;
    // an excluded value carries a `-` prefix.
    let title = if app.is_filtering() {
        let negate = |p: Polarity| if p == Polarity::Exclude { "-" } else { "" };
        let values: Vec<String> = app
            .project_filter
            .values()
            .map(|(v, p)| format!("{}({})", negate(p), v))
            .chain(
                app.tag_filter
                    .values()
                    .map(|(v, p)| format!("{}#{}", negate(p), v)),
            )
            .collect();
        format!(" Entries [filtered: {}] ", values.join(" "))
    } else {
        " Entries ".to_string()
    };

    // `Fill(1)` and `Min(12)` share what the fixed columns leave, so both grow with
    // the terminal. The fixed widths are exactly what they render, with no padding.
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(12),
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(3),
        ],
    )
    .header(header_row)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(title, Style::default().fg(theme::title()))),
    )
    .row_highlight_style(Style::default().bg(theme::selected_bg()))
    .highlight_symbol(CURSOR_MARKER);

    let mut render_state = TableState::default().with_selected(visual_selected);
    f.render_stateful_widget(table, area, &mut render_state);
}
