//! The project header row of the Entries table.

use super::columns::EntryColumn;
use super::entries::members_total;
use crate::tui::rows::ProjectHeader;
use crate::tui::theme;
use ratatui::{
    prelude::*,
    widgets::{Cell, Row},
};

/// One project's row: a chevron and its name in Description, its summed
/// duration in Duration. Every other column is blank.
pub(super) fn project_header_row(
    columns: &[EntryColumn],
    header: &ProjectHeader,
    entries: &[crate::tracker::TimeEntry],
) -> Row<'static> {
    let chevron = if header.expanded {
        "\u{25be}"
    } else {
        "\u{25b8}"
    };
    let label = format!("{chevron} {}", header.project);
    let total = members_total(entries, &header.members);
    let total_str = crate::duration::format(total);
    let bold = |color: Color| Style::default().fg(color).add_modifier(Modifier::BOLD);

    let mut cells: Vec<Cell<'static>> = columns
        .iter()
        .map(|column| match column {
            EntryColumn::Description => Cell::from(label.clone()).style(bold(theme::accent())),
            EntryColumn::Duration => Cell::from(total_str.clone()).style(bold(theme::accent())),
            _ => Cell::from(""),
        })
        .collect();
    let icon = if header.end.is_none() {
        crate::icons::active()
    } else {
        ""
    };
    cells.push(Cell::from(icon).style(Style::default().fg(theme::active())));

    Row::new(cells).style(Style::default().bg(theme::group_header_bg()))
}
