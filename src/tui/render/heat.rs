//! What every heat surface draws alike: the tick axis over its cells, the
//! `Less … More` ramps, and the content pane's own two-dimensional grid.

use super::legend::content_legend;
use crate::tui::summary::{BucketGrid, HeatBand, HeatGrid, axis_tick, strip_cells};
use crate::tui::types::ViewMode;
use crate::tui::{App, theme};
use chrono::Duration;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

/// Today's cell marker, as the year grid draws it too.
pub(super) const TODAY_MARKER: &str = "\u{25cf}";

/// What a heat block calls itself: its total and how many `unit`s made it.
pub(super) fn heat_block_title(total: Duration, active: usize, unit: &str) -> String {
    format!(
        " {} tracked over {} active {}{} ",
        crate::duration::format(total),
        active,
        unit,
        if active == 1 { "" } else { "s" }
    )
}

/// The open view's period as a grid of heat cells filling the content area: a
/// gutter of row labels, a tick axis per band, and cells stretched to the room
/// the box has in both axes. A cell is shaded by how full its own span is, so
/// the shading does not re-scale as the user pages through periods.
pub(super) fn render_heat_grid(f: &mut Frame, app: &mut App, area: Rect) {
    let inner_width = area.width.saturating_sub(2);
    let inner_height = area.height.saturating_sub(2);
    let grid = app.view_heat_grid(inner_width, inner_height);
    // The draw owns the clamp: it alone knows the room and the layout. `j`
    // and `k` read the limit back off `App`.
    app.heat_scroll_max = scroll_max(&grid, inner_height, app.view_mode);
    app.heat_scroll = app.heat_scroll.min(app.heat_scroll_max);
    let app = &*app;
    let day_sized = grid
        .bands
        .first()
        .is_some_and(|band| band.cell_span >= Duration::days(1));

    let keys = content_legend(app, inner_width);
    let keys_width = keys.as_ref().map(|line| line.width()).unwrap_or(0) as u16;
    let ramp = if day_sized {
        day_heat_legend(keys_width, inner_width)
    } else {
        relative_heat_legend(keys_width, inner_width)
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::border()))
        .title(Span::styled(
            heat_block_title(grid.total, grid.active(), grid.unit),
            Style::default().fg(theme::title()),
        ));
    if let Some(keys) = keys {
        block = block.title_bottom(keys.left_aligned());
    }
    if let Some(ramp) = ramp {
        block = block.title_bottom(ramp.right_aligned());
    }

    let available = (inner_width as usize).saturating_sub(grid.gutter);
    let (drawn, first_row) = scrolled(app, &grid);
    let rows_total: usize = drawn
        .iter()
        .map(|band| band.rows())
        .sum::<usize>()
        .saturating_sub(first_row);
    if rows_total == 0 || available == 0 {
        f.render_widget(Paragraph::new(Vec::<Line>::new()).block(block), area);
        return;
    }
    // Every band keeps its axis line and a blank line before the next;
    // the rows share what is left.
    let fixed = drawn.len() + drawn.len().saturating_sub(1);
    let cell_height = ((inner_height as usize).saturating_sub(fixed) / rows_total).max(1);
    let dim = Style::default().fg(theme::inactive());

    let room = inner_height as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(room);
    'bands: for band in drawn {
        if !lines.is_empty() {
            if lines.len() == room {
                break;
            }
            lines.push(Line::default());
        }
        let (cell_width, columns) = strip_cells(available, band.columns());
        // Too narrow a box sheds the oldest columns, as every other strip
        // does: the newest weeks and the marker on today must survive.
        let first = band.columns() - columns;
        lines.push(Line::from(Span::styled(
            format!(
                "{}{}",
                tick_gutter(band.title.as_deref(), grid.gutter),
                axis_line(&band.column_ticks[first..], cell_width)
            ),
            dim,
        )));
        for (row, cells) in band.cells.iter().enumerate().skip(first_row) {
            for line in 0..cell_height {
                if lines.len() == room {
                    break 'bands;
                }
                // The label names its row once, so a tall row does not stack it.
                let named = line == cell_height / 2;
                let label = if named {
                    gutter_label(&band.row_labels[row], grid.gutter)
                } else {
                    " ".repeat(grid.gutter)
                };
                let mut spans = vec![Span::styled(label, dim)];
                for (column, cell) in cells.iter().enumerate().skip(first) {
                    let Some(held) = cell else {
                        spans.push(Span::raw(" ".repeat(cell_width)));
                        continue;
                    };
                    let style = Style::default().bg(if day_sized {
                        theme::heat_color(held.num_hours())
                    } else {
                        theme::heat_shade(held.num_seconds(), band.cell_span.num_seconds())
                    });
                    if band.now_column == Some(column) && marks_now(band, row, named) {
                        let pad = " ".repeat(cell_width.saturating_sub(1));
                        spans.push(Span::styled(
                            format!("{TODAY_MARKER}{pad}"),
                            style.fg(theme::highlight()).bold(),
                        ));
                    } else {
                        spans.push(Span::styled(" ".repeat(cell_width), style));
                    }
                }
                lines.push(Line::from(spans));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// How far the offset may go: the rows or bands the box has no room for, at
/// one line each. Only `All` scrolls its bands, only the Day view its rows.
fn scroll_max(grid: &HeatGrid, inner_height: u16, view: ViewMode) -> usize {
    let room = inner_height as usize;
    match view {
        // A band is its axis and rows, plus a blank line before the next.
        ViewMode::All => {
            let band_lines = grid.bands.first().map_or(1, |band| 2 + band.rows());
            grid.bands
                .len()
                .saturating_sub(((room + 1) / band_lines).max(1))
        }
        // The axis costs a line; the rows take the rest, one each.
        ViewMode::Day => grid
            .bands
            .first()
            .map_or(0, HeatBand::rows)
            .saturating_sub(room.saturating_sub(1)),
        _ => 0,
    }
}

/// The bands to draw and the first row of each, at the clamped `heat_scroll`.
fn scrolled<'a>(app: &App, grid: &'a HeatGrid) -> (&'a [HeatBand], usize) {
    if app.view_mode == ViewMode::All {
        return (&grid.bands[app.heat_scroll..], 0);
    }
    (&grid.bands, app.heat_scroll)
}

/// Whether the marker belongs on this line of `row`. A band whose rows are
/// time names its own row; one whose rows are projects marks its middle.
fn marks_now(band: &HeatBand, row: usize, named: bool) -> bool {
    named
        && match band.now_row {
            Some(now) => now == row,
            None => row == band.rows() / 2,
        }
}

/// A row label with one blank column each side inside `width`.
fn gutter_label(text: &str, width: usize) -> String {
    let inner = width.saturating_sub(1);
    let label: String = text.chars().take(inner.saturating_sub(1)).collect();
    format!(" {label:<inner$}")
}

/// The tick row's gutter: the band's year left-aligned, or blank.
fn tick_gutter(title: Option<&str>, width: usize) -> String {
    match title {
        Some(text) => format!("{text:<width$}"),
        None => " ".repeat(width),
    }
}

/// The axis that heads a strip of `cells` buckets, the first of them `first`:
/// one tick over the cell it belongs to, as the yearly overview heads its grid
/// with month names.
pub(super) fn heat_axis(
    grid: &BucketGrid,
    first: usize,
    cells: usize,
    cell_width: usize,
) -> String {
    let labels: Vec<Option<String>> = (0..cells)
        .map(|cell| {
            let start = grid.start(first + cell);
            axis_tick(start, grid.start(first + cell + 1) - start, grid.len())
        })
        .collect();
    axis_line(&labels, cell_width)
}

/// One tick per labelled cell, each written over the cell it belongs to. A
/// tick is cut one column short of the next one, so a dense axis abbreviates
/// instead of running its labels together; the last may run to the end.
pub(super) fn axis_line(labels: &[Option<String>], cell_width: usize) -> String {
    let ticks: Vec<(usize, &String)> = labels
        .iter()
        .enumerate()
        .filter_map(|(cell, label)| label.as_ref().map(|text| (cell * cell_width, text)))
        .collect();
    let mut axis = vec![' '; labels.len() * cell_width];
    for (index, (column, text)) in ticks.iter().enumerate() {
        let room = ticks
            .get(index + 1)
            .map(|(next, _)| (next - column).saturating_sub(1).max(1))
            .unwrap_or_else(|| axis.len().saturating_sub(*column));
        for (offset, symbol) in text.chars().take(room).enumerate() {
            axis[column + offset] = symbol;
        }
    }
    axis.into_iter().collect()
}

/// The day-total ramp, `Less` to `More`, over `theme::heat_color`. `None` when
/// it would run into the key legend sharing its border, as its relative
/// sibling is.
pub(super) fn day_heat_legend(keys_width: u16, inner_width: u16) -> Option<Line<'static>> {
    let t = theme::theme();
    let mut spans = vec![Span::styled(
        " Less ",
        Style::default().fg(theme::inactive()),
    )];
    for hours in [
        0,
        1,
        t.day_duration_med_h / 2,
        t.day_duration_med_h,
        t.day_duration_high_h,
    ] {
        spans.push(Span::styled(
            "  ",
            Style::default().bg(theme::heat_color(hours)),
        ));
    }
    spans.push(Span::styled(
        " More ",
        Style::default().fg(theme::inactive()),
    ));
    let line = Line::from(spans);
    (line.width() as u16 + keys_width <= inner_width).then_some(line)
}

/// The relative ramp, `Less` to `More`, over `theme::heat_shade` — the one the
/// grains too small for a day threshold read against. `None` when it would run
/// into the key legend sharing its border, whichever side each sits on.
pub(super) fn relative_heat_legend(keys_width: u16, inner_width: u16) -> Option<Line<'static>> {
    // Part and maximum per swatch, one per quarter of the ramp. An empty
    // bucket is never drawn, so the ramp does not offer a swatch for one.
    const SWATCHES: [(i64, i64); 4] = [(1, 4), (1, 2), (3, 4), (1, 1)];

    let mut spans = vec![Span::styled(
        " Less ",
        Style::default().fg(theme::inactive()),
    )];
    spans.extend(SWATCHES.iter().map(|(part, max)| {
        Span::styled("  ", Style::default().bg(theme::heat_shade(*part, *max)))
    }));
    spans.push(Span::styled(
        " More ",
        Style::default().fg(theme::inactive()),
    ));
    let line = Line::from(spans);
    (line.width() as u16 + keys_width <= inner_width).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::storage::{env_guard, env_sandbox as sandbox};
    use crate::tracker::{TimeData, TimeEntry};
    use crate::tui::types::ViewMode;
    use chrono::{Local, NaiveDate};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn at_noon(year: i32, month: u32, day: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    /// An entry of `minutes` opening at `at`.
    fn entry(id: u64, at: chrono::NaiveDateTime, minutes: i64) -> TimeEntry {
        let start = at.and_local_timezone(Local).unwrap();
        TimeEntry {
            id,
            description: "seed".to_string(),
            project: Some("tt".to_string()),
            tags: Vec::new(),
            start_time: start,
            end_time: Some(start + Duration::minutes(minutes)),
            idle: Vec::new(),
            data: None,
        }
    }

    fn app_for(view: ViewMode, on: NaiveDate, entries: Vec<TimeEntry>) -> App {
        let next_id = entries.len() as u64;
        crate::storage::save_data(&TimeData {
            entries,
            next_id,
            schema_version: 1,
        })
        .unwrap();
        let mut app = App::new().unwrap();
        app.view_mode = view;
        app.selected_date = on;
        app.heat_view = true;
        app
    }

    /// Every inner line of the drawn block, as `(text, painted columns)`.
    fn drawn(app: &mut App, width: u16, height: u16) -> Vec<(String, Vec<u16>)> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render_heat_grid(f, app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (1..height.saturating_sub(1))
            .map(|y| {
                let text: String = (1..width - 1).map(|x| buffer[(x, y)].symbol()).collect();
                let painted: Vec<u16> = (1..width - 1)
                    .filter(|x| buffer[(*x, y)].bg != Color::Reset)
                    .collect();
                (text.trim_end().to_string(), painted)
            })
            .collect()
    }

    /// `All` stacks a band per year; the scroll offset drops the ones above.
    #[test]
    fn the_all_grid_scrolls_past_its_first_year_band() {
        let _guard = env_guard();
        sandbox("heat-grid-all-scroll");
        let mut app = app_for(
            ViewMode::All,
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
            vec![
                entry(0, at_noon(2026, 3, 4), 60),
                entry(1, at_noon(2024, 3, 4), 60),
            ],
        );

        let named = |app: &mut App| {
            drawn(app, 80, 12)
                .into_iter()
                .filter(|(text, _)| text.starts_with("2026 ") || text.starts_with("2024 "))
                .map(|(text, _)| text[..4].to_string())
                .collect::<Vec<String>>()
        };
        assert_eq!(named(&mut app)[0], "2026", "the newest year is not on top");
        assert_eq!(app.heat_scroll_max, 1, "the draw did not set the limit");

        app.heat_scroll = 1;
        assert_eq!(
            named(&mut app),
            vec!["2024".to_string()],
            "the offset did not move the bands"
        );

        app.heat_scroll = 9;
        assert_eq!(
            named(&mut app),
            vec!["2024".to_string()],
            "the last band scrolled off the box"
        );
    }

    /// Every band's tick row opens with its own year, and every band's rows
    /// are named Mon..Sun, not just the first.
    #[test]
    fn every_year_band_gutter_names_its_year_and_its_weekdays() {
        let _guard = env_guard();
        sandbox("heat-grid-year-gutter");
        let mut app = app_for(
            ViewMode::All,
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
            vec![
                entry(0, at_noon(2026, 3, 4), 60),
                entry(1, at_noon(2024, 3, 4), 60),
            ],
        );

        let lines = drawn(&mut app, 80, 20);
        let ticks: Vec<&str> = lines
            .iter()
            .map(|(text, _)| text.as_str())
            .filter(|text| text.contains("Jan"))
            .collect();
        assert_eq!(ticks.len(), 2, "expected one tick row per band: {ticks:?}");
        assert!(
            ticks[0].starts_with("2026 "),
            "no year before Jan: {}",
            ticks[0]
        );
        assert!(
            ticks[1].starts_with("2024 "),
            "no year before Jan: {}",
            ticks[1]
        );
        let second = lines
            .iter()
            .position(|(text, _)| text.starts_with("2024 "))
            .unwrap();
        assert!(
            lines[second - 1].0.trim().is_empty(),
            "no blank line before the second band: {}",
            lines[second - 1].0
        );

        const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let weekdays: Vec<&str> = lines
            .iter()
            .map(|(text, _)| text[..5.min(text.len())].trim())
            .filter(|text| WEEKDAYS.contains(text))
            .collect();
        assert_eq!(
            weekdays,
            WEEKDAYS
                .iter()
                .chain(WEEKDAYS.iter())
                .copied()
                .collect::<Vec<_>>(),
            "every band's rows are not Mon..Sun: {weekdays:?}"
        );
    }

    /// A box too narrow for the year sheds its oldest weeks, so the marker on
    /// today is still on screen.
    #[test]
    fn a_narrow_year_grid_sheds_its_oldest_weeks() {
        let _guard = env_guard();
        sandbox("heat-grid-year-narrow");
        let today = Local::now().date_naive();
        let mut app = app_for(
            ViewMode::Year,
            today,
            vec![entry(0, today.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let wide = drawn(&mut app, 120, 14);
        assert!(
            wide.iter().any(|(text, _)| text.contains(TODAY_MARKER)),
            "the wide grid lost today"
        );
        let narrow = drawn(&mut app, 30, 14);
        assert!(
            narrow.iter().any(|(text, _)| text.contains(TODAY_MARKER)),
            "the shed grid dropped the newest weeks instead of the oldest"
        );
    }

    /// One hour worked by three projects is one active hour, not three.
    #[test]
    fn the_day_title_counts_the_hours_not_the_project_cells() {
        let _guard = env_guard();
        sandbox("heat-grid-day-active");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let mut app = app_for(
            ViewMode::Day,
            day,
            vec![
                entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 20),
                entry(1, day.and_hms_opt(9, 20, 0).unwrap(), 20),
                entry(2, day.and_hms_opt(9, 40, 0).unwrap(), 20),
            ],
        );
        app.data.entries[1].project = Some("beta".to_string());
        app.data.entries[2].project = Some("gamma".to_string());

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|f| render_heat_grid(f, &mut app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let title: String = (0..80).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(title.contains("1 active hour"), "{title}");
    }

    /// The draw works out how far the grid may scroll and pulls an offset
    /// left over from a taller box back with it.
    #[test]
    fn the_draw_sets_the_scroll_limit_and_clamps_the_offset() {
        let _guard = env_guard();
        sandbox("heat-grid-limit");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let mut app = app_for(
            ViewMode::Day,
            day,
            (0..8)
                .map(|id| entry(id, day.and_hms_opt(8 + id as u32, 0, 0).unwrap(), 30))
                .collect(),
        );
        for (index, held) in app.data.entries.iter_mut().enumerate() {
            held.project = Some(format!("p{index}"));
        }
        app.heat_scroll = 6;

        // Eight inner lines: the axis, then seven of the eight rows.
        drawn(&mut app, 80, 10);
        assert_eq!(app.heat_scroll_max, 1);
        assert_eq!(app.heat_scroll, 1, "the offset outlived the taller box");

        // A box with room for every row scrolls nowhere.
        drawn(&mut app, 80, 20);
        assert_eq!(app.heat_scroll_max, 0);
        assert_eq!(app.heat_scroll, 0);
    }

    /// The gutter names each project once and the cells fill what is left.
    #[test]
    fn a_day_grid_draws_a_project_row_over_the_hours_of_the_day() {
        let _guard = env_guard();
        sandbox("heat-grid-day-render");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let lines = drawn(
            &mut app_for(
                ViewMode::Day,
                day,
                vec![entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60)],
            ),
            80,
            12,
        );

        let axis = &lines[0].0;
        assert!(axis.starts_with(&" ".repeat(12)), "no gutter: {axis}");
        for hour in ["00", "06", "12", "18"] {
            assert!(axis.contains(hour), "no `{hour}` on the axis: {axis}");
        }
        assert!(
            lines.iter().any(|(text, _)| text.trim() == "tt"),
            "the gutter did not name the project"
        );
        let painted: Vec<usize> = lines
            .iter()
            .map(|(_, cells)| cells.len())
            .filter(|count| *count > 0)
            .collect();
        // 78 inner columns less the 12-column gutter: 66 over 24 hours.
        assert!(
            painted.iter().all(|count| *count == 24 * 2),
            "a cell was clipped or dropped: {painted:?}"
        );
        // Ten inner lines less the axis: the one row takes the other nine.
        assert_eq!(painted.len(), 9, "the row did not fill the box");
    }

    /// A cell shades by how full its own span is, not by the busiest cell, so
    /// two half-full hours read alike whatever else the day holds.
    #[test]
    fn a_cell_shades_by_the_fill_of_its_own_span() {
        let _guard = env_guard();
        sandbox("heat-grid-fill");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let mut app = app_for(
            ViewMode::Day,
            day,
            vec![
                entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60),
                entry(1, day.and_hms_opt(11, 0, 0).unwrap(), 30),
            ],
        );

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|f| render_heat_grid(f, &mut app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let cells: Vec<Color> = (1..79).map(|x| buffer[(x, 2)].bg).collect();
        let gutter = 12;
        assert_eq!(
            cells[gutter + 9 * 2],
            theme::heat_shade(1, 1),
            "a full hour is not full"
        );
        assert_eq!(
            cells[gutter + 11 * 2],
            theme::heat_shade(1, 2),
            "half an hour is not half"
        );
    }

    /// A short box coarsens the rows; nothing of the week is hidden.
    #[test]
    fn a_week_grid_is_blocks_of_the_day_over_the_weekdays() {
        let _guard = env_guard();
        sandbox("heat-grid-week-render");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let mut app = app_for(
            ViewMode::Week,
            day,
            vec![entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let lines = drawn(&mut app, 80, 16);
        assert!(
            lines[0].0.contains("Mon"),
            "no weekday axis: {}",
            lines[0].0
        );
        assert!(lines[0].0.contains("Sun"));
        let labels: Vec<&str> = lines
            .iter()
            .map(|(text, _)| text.trim())
            .filter(|text| !text.is_empty())
            .collect();
        assert!(labels.contains(&"00"), "midnight is unlabelled: {labels:?}");
        assert!(labels.contains(&"22"), "the last block is unlabelled");
        let painted = lines.iter().filter(|(_, cells)| !cells.is_empty()).count();
        assert_eq!(painted, 12, "the twelve blocks were not all drawn");
    }

    /// 80 columns is the narrow case every view must still fit.
    #[test]
    fn a_month_grid_keeps_every_day_at_eighty_columns() {
        let _guard = env_guard();
        sandbox("heat-grid-month-render");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let mut app = app_for(
            ViewMode::Month,
            day,
            vec![entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let lines = drawn(&mut app, 80, 16);
        let widest = lines
            .iter()
            .map(|(_, cells)| cells.len())
            .max()
            .unwrap_or(0);
        // 74 columns over January's 31 days: two each, none shed.
        assert_eq!(widest, 62, "a day of the month was dropped");
        assert!(
            lines.iter().all(|(text, _)| text.chars().count() <= 78),
            "a line ran past the inner width"
        );
    }

    /// The block title counts what the entry list under it counts.
    #[test]
    fn the_block_title_totals_the_filtered_entries() {
        let _guard = env_guard();
        sandbox("heat-grid-title");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let mut app = app_for(
            ViewMode::Day,
            day,
            vec![
                entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60),
                entry(1, day.and_hms_opt(11, 0, 0).unwrap(), 30),
            ],
        );

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|f| render_heat_grid(f, &mut app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let title: String = (0..80).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(
            title.contains(&crate::duration::format(app.filtered_total())),
            "the title disagrees with the list: {title}"
        );
        assert!(title.contains("2 active hours"), "{title}");
    }

    fn labels(texts: &[Option<&str>]) -> Vec<Option<String>> {
        texts.iter().map(|text| text.map(str::to_string)).collect()
    }

    /// A crowded tick gives up its tail; the last one keeps the whole row.
    #[test]
    fn a_tick_is_cut_one_column_short_of_its_neighbour() {
        let dense = axis_line(&labels(&[Some("Jan"), Some("Feb"), None, None]), 2);
        assert_eq!(dense, "J Feb   ", "`Jan` kept a column its neighbour needs");

        let sparse = axis_line(&labels(&[Some("Jan"), None, None, Some("Feb"), None]), 2);
        assert_eq!(
            sparse, "Jan   Feb ",
            "a tick with room keeps its whole name"
        );
    }

    /// The row's width binds the last tick as a neighbour would.
    #[test]
    fn the_last_tick_is_cut_at_the_end_of_the_row() {
        assert_eq!(axis_line(&labels(&[None, Some("Feb")]), 2), "  Fe");
    }

    /// Nothing labelled is a blank axis of the row's own width.
    #[test]
    fn an_unlabelled_axis_is_blank_and_as_wide_as_the_cells() {
        assert_eq!(axis_line(&labels(&[None, None, None]), 3), " ".repeat(9));
        assert_eq!(axis_line(&[], 3), "");
    }
}
