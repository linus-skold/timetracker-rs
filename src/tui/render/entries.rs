use super::columns::EntryColumn;
use super::legend::content_legend;
use super::overlay::CURSOR_MARKER;
use super::view_header::{content_title, render_view_selector};
use crate::tracker::TimeData;
use crate::tui::panes::Polarity;
use crate::tui::rows::{GroupHeader, Member, VisibleRow};
use crate::tui::summary::project_key;
use crate::tui::types::Focus;
use crate::tui::{App, theme};
use chrono::{Duration, Local, NaiveDate};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};
use std::collections::HashMap;

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

/// One configured column's cell for an entry row. `description` already
/// carries the member tree connector, if any.
fn entry_cell(
    column: EntryColumn,
    entry: &crate::tracker::TimeEntry,
    description: &str,
    dur_color: Color,
) -> Cell<'static> {
    match column {
        EntryColumn::Date => {
            Cell::from(entry.format_date()).style(Style::default().fg(theme::title()))
        }
        EntryColumn::Start => {
            Cell::from(entry.format_start_time()).style(Style::default().fg(theme::accent()))
        }
        EntryColumn::End => {
            Cell::from(entry.format_end_time()).style(Style::default().fg(theme::inactive()))
        }
        EntryColumn::Description => Cell::from(description.to_string()),
        EntryColumn::Project => {
            Cell::from(project_key(entry).to_string()).style(Style::default().fg(theme::accent()))
        }
        EntryColumn::Tags => {
            Cell::from(entry.format_tags()).style(Style::default().fg(theme::highlight()))
        }
        EntryColumn::Duration => {
            Cell::from(entry.format_duration()).style(Style::default().fg(dur_color))
        }
    }
}

/// One entry's row. A member row hangs off its group header's tree connector
/// and carries the tint rather than the stripe.
fn entry_row(
    columns: &[EntryColumn],
    entry: &crate::tracker::TimeEntry,
    stripe: bool,
    member: Option<&Member>,
) -> Row<'static> {
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

    let row_style = match (member, stripe) {
        (Some(_), _) => Style::default().bg(theme::member_bg()),
        (None, true) => Style::default().bg(Color::Rgb(35, 35, 35)),
        (None, false) => Style::default(),
    };
    // The connector opens in the column the group header's chevron sits in, so
    // its stroke runs unbroken from the header down to the last member.
    let description = match member {
        Some(member) if member.last => format!("\u{2514}\u{2500}\u{2500} {}", entry.description),
        Some(_) => format!("\u{251c}\u{2500}\u{2500} {}", entry.description),
        None => entry.description.clone(),
    };

    let mut cells: Vec<Cell<'static>> = columns
        .iter()
        .map(|c| entry_cell(*c, entry, &description, dur_color))
        .collect();
    cells.push(Cell::from(entry.status_icon()).style(status_style));

    Row::new(cells).style(row_style)
}

/// The live sum of the entries at `members`. Summed per frame and never
/// cached: a running member's duration moves with the clock.
fn members_total(entries: &[crate::tracker::TimeEntry], members: &[usize]) -> Duration {
    members
        .iter()
        .filter_map(|index| entries.get(*index))
        .fold(Duration::zero(), |acc, entry| acc + entry.duration())
}

/// The day banner's cell for one configured column: the weekday, the long
/// date and the day total each sit in their matching column; every other
/// column is blank.
fn day_header_cell(
    column: EntryColumn,
    weekday: &str,
    date_str: &str,
    total_str: &str,
) -> Cell<'static> {
    match column {
        EntryColumn::Date => Cell::from(weekday.to_string()).style(
            Style::default()
                .fg(theme::highlight())
                .add_modifier(Modifier::BOLD),
        ),
        EntryColumn::Description => {
            Cell::from(date_str.to_string()).style(Style::default().fg(theme::title()))
        }
        EntryColumn::Duration => Cell::from(total_str.to_string()).style(
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        EntryColumn::Start | EntryColumn::End | EntryColumn::Project | EntryColumn::Tags => {
            Cell::from("")
        }
    }
}

fn day_header_row(columns: &[EntryColumn], date: NaiveDate, total: Duration) -> Row<'static> {
    let weekday = format!("\n{}", date.format("%A"));
    let date_str = format!("\n{}", date.format("%B %d, %Y"));
    let total_str = format!("\n{}", crate::duration::format(total));

    let mut cells: Vec<Cell<'static>> = columns
        .iter()
        .map(|c| day_header_cell(*c, &weekday, &date_str, &total_str))
        .collect();
    cells.push(Cell::from(""));

    Row::new(cells)
        .height(2)
        .style(Style::default().bg(theme::day_header_bg()))
}

/// The group header's cell for one configured column: its span in
/// Date/Start/End, its label in Description, its tag in Tags and its summed
/// duration in Duration and the shared project in Project.
fn group_header_cell(
    column: EntryColumn,
    header: &GroupHeader,
    label: &str,
    end: &str,
    total_str: &str,
    dur_color: Color,
) -> Cell<'static> {
    match column {
        EntryColumn::Date => Cell::from(header.start.format("%Y-%m-%d").to_string())
            .style(Style::default().fg(theme::title())),
        EntryColumn::Start => Cell::from(header.start.format("%H:%M").to_string())
            .style(Style::default().fg(theme::accent())),
        EntryColumn::End => {
            Cell::from(end.to_string()).style(Style::default().fg(theme::inactive()))
        }
        // The count leads: this column is 19 cells at 80 columns, so whatever
        // comes second is what the clip takes.
        EntryColumn::Description => {
            Cell::from(label.to_string()).style(Style::default().add_modifier(Modifier::ITALIC))
        }
        // Stored tags carry no `#`; the display prefix comes from `format_tags`.
        EntryColumn::Tags => Cell::from(crate::tracker::format_tags(std::slice::from_ref(
            &header.tag,
        )))
        .style(Style::default().fg(theme::highlight())),
        EntryColumn::Duration => {
            Cell::from(total_str.to_string()).style(Style::default().fg(dur_color))
        }
        EntryColumn::Project => Cell::from(header.project.clone().unwrap_or_default()).style(
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::ITALIC),
        ),
    }
}

/// One issue's collapsed row: its span, its member count and id behind a
/// chevron, and the members' summed duration.
fn group_header_row(
    columns: &[EntryColumn],
    header: &GroupHeader,
    entries: &[crate::tracker::TimeEntry],
) -> Row<'static> {
    let total = members_total(entries, &header.members);
    let dur_color = theme::duration_color(
        total.num_hours(),
        theme::theme().entry_duration_high_h,
        theme::theme().entry_duration_med_h,
    );
    let chevron = if header.expanded {
        "\u{25be}"
    } else {
        "\u{25b8}"
    };
    let end = header
        .end
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default();
    let icon = if header.end.is_none() {
        crate::icons::active()
    } else {
        ""
    };
    let label = format!(
        "{chevron} {} entries - {}",
        header.members.len(),
        header.tag
    );
    let total_str = crate::duration::format(total);

    let mut cells: Vec<Cell<'static>> = columns
        .iter()
        .map(|c| group_header_cell(*c, header, &label, &end, &total_str, dur_color))
        .collect();
    cells.push(Cell::from(icon).style(Style::default().fg(theme::active())));

    Row::new(cells).style(Style::default().bg(theme::group_header_bg()))
}

pub(super) fn render_entries_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = app
        .entry_columns
        .iter()
        .map(|c| c.label())
        .chain(std::iter::once(""))
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

    // Day totals are summed here rather than carried on the row, so a running
    // entry's minutes keep moving under a cached row model.
    let mut day_totals: HashMap<NaiveDate, Duration> = HashMap::new();
    for entry in app.filtered_entries() {
        *day_totals
            .entry(entry.start_time.date_naive())
            .or_insert_with(Duration::zero) += entry.duration();
    }

    // One walk over the row model builds the visual rows and, beside them, the
    // map from the cursor's selectable index to the visual index it draws at.
    let model = app.rows();
    let mut rows: Vec<Row> = Vec::with_capacity(model.len());
    let mut visual_of_selectable: Vec<usize> = Vec::with_capacity(model.len());
    let mut stripe = false;
    for row in &model {
        match row {
            VisibleRow::DayHeader { date } => {
                // The stripe alternation restarts inside each day partition.
                stripe = false;
                let total = day_totals.get(date).copied().unwrap_or_else(Duration::zero);
                rows.push(day_header_row(&app.entry_columns, *date, total));
            }
            VisibleRow::GroupHeader(header) => {
                visual_of_selectable.push(rows.len());
                rows.push(group_header_row(
                    &app.entry_columns,
                    header,
                    &app.data.entries,
                ));
            }
            VisibleRow::Entry { index, member } => {
                if let Some(entry) = app.data.entries.get(*index) {
                    visual_of_selectable.push(rows.len());
                    rows.push(entry_row(
                        &app.entry_columns,
                        entry,
                        stripe,
                        member.as_ref(),
                    ));
                    // A member carries no stripe, so an expansion leaves the
                    // top-level alternation around it as it was.
                    if member.is_none() {
                        stripe = !stripe;
                    }
                }
            }
        }
    }
    let visual_selected = app
        .table_state
        .selected()
        .and_then(|idx| visual_of_selectable.get(idx).copied());

    // `(tt)` and `#impl`, the CLI's own sigils, so the title needs no legend;
    // an excluded value carries a `-` prefix.
    let base_title = content_title(app, "Entries", app.sort_order.label());
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
        format!(
            "{} [filtered: {}] ",
            base_title.trim_end(),
            values.join(" ")
        )
    } else {
        base_title
    };

    let focused = app.focus == Focus::Table;
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused {
            theme::accent()
        } else {
            theme::border()
        }))
        .title(Span::styled(
            title,
            Style::default().fg(if focused {
                theme::highlight()
            } else {
                theme::title()
            }),
        ));
    if let Some(keys) = content_legend(app, area.width.saturating_sub(2)) {
        block = block.title_bottom(keys.left_aligned());
    }

    // `Fill(1)` and `Min(12)` share what the fixed columns leave, so both grow with
    // the terminal. The fixed widths are exactly what they render, with no padding.
    let widths: Vec<Constraint> = app
        .entry_columns
        .iter()
        .map(|c| c.constraint())
        .chain(std::iter::once(Constraint::Length(3)))
        .collect();
    let table = Table::new(rows, widths)
        .header(header_row)
        .row_highlight_style(Style::default().bg(theme::selected_bg()))
        .highlight_symbol(CURSOR_MARKER);

    let inner = block.inner(area);
    f.render_widget(block, area);
    let table_area = render_view_selector(f, app, inner);
    let mut render_state = TableState::default().with_selected(visual_selected);
    f.render_stateful_widget(table, table_area, &mut render_state);
}
