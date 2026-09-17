//! Per-project totals and the state behind the collapsible `Summary`
//! surface. Display-only; `render/surfaces.rs` draws it.
//!
//! **Two modes, picked by `summary_follows_filters`:** scope-only, the
//! default, folds `scope_entries()`, so a filter leaves it alone. Follow
//! mode folds `filtered_entries()` — both panes and the `/` search term.

use std::collections::HashMap;

use chrono::{Datelike, Duration, Local, Months, NaiveDate, NaiveDateTime, Timelike};

use super::App;
use super::panes::surface_count;
use super::types::{Focus, ViewMode};
use crate::tracker::{TimeData, TimeEntry};

/// Most project rows the surface shows before the rest live in the border count.
const MAX_VISIBLE_PROJECTS: usize = 6;

/// The rule and the total row under the project rows.
pub(crate) const SUMMARY_TOTAL_LINES: u16 = 2;

/// The scope-only statement on the title bar, after the scope word.
const ALL_PROJECTS: &str = "all projects";

/// The follow-mode statement in its place.
const FILTERED: &str = "filtered";

/// The label for entries with no project; counted so the rows sum to the scope.
pub(crate) const NO_PROJECT: &str = "(no project)";

/// The period one heat-strip cell covers, chosen by the view mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grain {
    Hour,
    Day,
    Week,
    Month,
}

impl Grain {
    pub(crate) fn for_view(view: ViewMode) -> Self {
        match view {
            ViewMode::Day => Grain::Hour,
            ViewMode::Week | ViewMode::Month => Grain::Day,
            ViewMode::All => Grain::Week,
            ViewMode::Year => Grain::Month,
        }
    }
}

/// Per-project time buckets over one period, oldest bucket first.
pub(crate) struct BucketGrid {
    pub(crate) grain: Grain,
    /// The start of bucket 0; each later bucket follows it by one grain.
    pub(crate) anchor: NaiveDateTime,
    /// One bucket list per row it was folded for, index for index.
    pub(crate) rows: Vec<Vec<Duration>>,
}

impl BucketGrid {
    /// When bucket `index` begins.
    pub(crate) fn start(&self, index: usize) -> NaiveDateTime {
        let index = index as i64;
        match self.grain {
            Grain::Hour => self.anchor + Duration::hours(index),
            Grain::Day => self.anchor + Duration::days(index),
            Grain::Week => self.anchor + Duration::days(index * 7),
            Grain::Month => self
                .anchor
                .checked_add_months(Months::new(index as u32))
                .unwrap_or(self.anchor),
        }
    }

    /// Buckets per row; every row carries the same count.
    pub(crate) fn len(&self) -> usize {
        self.rows.first().map(Vec::len).unwrap_or(0)
    }
}

/// Columns the gutter takes for a clock label.
pub(crate) const HEAT_GUTTER: usize = 4;

/// Columns the gutter takes for a project name, which it truncates.
pub(crate) const PROJECT_GUTTER: usize = 12;

/// Columns the gutter takes for a year band: the year plus one space.
pub(crate) const YEAR_GUTTER: usize = 5;

/// One block of the content grid: its rows over its columns.
pub(crate) struct HeatBand {
    /// Named once, in the gutter of the tick row: the year band's year.
    pub(crate) title: Option<String>,
    /// One label per row, drawn in the gutter.
    pub(crate) row_labels: Vec<String>,
    /// One tick per column, drawn over the column it names.
    pub(crate) column_ticks: Vec<Option<String>>,
    /// Row-major; `None` is a cell outside the band's own period.
    pub(crate) cells: Vec<Vec<Option<Duration>>>,
    /// What one cell covers; its shade is how full of this it is.
    pub(crate) cell_span: Duration,
    /// Where this moment falls. A row of its own only where the rows are
    /// time; the Day view's rows are projects and mark the column alone.
    pub(crate) now_row: Option<usize>,
    pub(crate) now_column: Option<usize>,
}

impl HeatBand {
    pub(crate) fn rows(&self) -> usize {
        self.cells.len()
    }

    pub(crate) fn columns(&self) -> usize {
        self.cells.first().map(Vec::len).unwrap_or(0)
    }
}

/// The content pane's heat: one band per period drawn, newest band first.
pub(crate) struct HeatGrid {
    pub(crate) bands: Vec<HeatBand>,
    /// The noun the block title counts.
    pub(crate) unit: &'static str,
    /// Columns the row labels take.
    pub(crate) gutter: usize,
    /// The time the cells hold, so the title counts what the grid paints.
    pub(crate) total: Duration,
    /// The title counts columns rather than cells, where the rows are
    /// projects and several may hold the same hour.
    pub(crate) counts_columns: bool,
}

impl HeatGrid {
    /// What the title counts: the columns holding time where the rows share a
    /// clock, else every cell holding time.
    pub(crate) fn active(&self) -> usize {
        self.bands
            .iter()
            .map(|band| {
                if self.counts_columns {
                    (0..band.columns())
                        .filter(|column| {
                            band.cells
                                .iter()
                                .any(|row| row[*column].is_some_and(|held| !held.is_zero()))
                        })
                        .count()
                } else {
                    band.cells
                        .iter()
                        .flatten()
                        .filter(|cell| cell.is_some_and(|held| !held.is_zero()))
                        .count()
                }
            })
            .sum()
    }

    /// Every cell's time, over every band.
    fn painted(&self) -> Duration {
        self.bands
            .iter()
            .flat_map(|band| band.cells.iter().flatten())
            .flatten()
            .fold(Duration::zero(), |acc, held| acc + *held)
    }
}

/// What the cell opening at `start` is called on an axis of `count` cells of
/// `span` each, or `None` where no tick belongs. The span picks the
/// vocabulary — a clock hour, a weekday, a date, a month — so every grid and
/// every strip name their cells alike.
pub(crate) fn axis_tick(start: NaiveDateTime, span: Duration, count: usize) -> Option<String> {
    /// Hours between ticks, so a day reads `00 06 12 18`.
    const TICK_HOURS: u32 = 6;
    /// Day cells a week holds; more than this is a month, not a week.
    const WEEK_DAYS: u32 = 7;

    let day = Duration::days(1);
    if span < day {
        return (start.minute() == 0 && start.hour().is_multiple_of(TICK_HOURS))
            .then(|| format!("{:02}", start.hour()));
    }
    if span == day {
        // A month of weekday names orients nobody, so a day axis longer than
        // a week takes the date of every seventh cell and nothing between.
        return if count <= WEEK_DAYS as usize {
            Some(start.format("%a").to_string())
        } else {
            (start.day() % WEEK_DAYS == 1).then(|| start.day().to_string())
        };
    }
    if span <= day * 7 {
        // The week opening a month carries its name.
        return (start.day() <= 7).then(|| start.format("%b").to_string());
    }
    Some(start.format("%b").to_string())
}

/// The cells of a run of `count` spans from `origin` that `[from, to)`
/// touches, so a fold walks the entry's own stretch and no further.
fn touched(
    origin: NaiveDateTime,
    span: Duration,
    count: usize,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> std::ops::Range<usize> {
    let seconds = span.num_seconds().max(1);
    let cell = |at: NaiveDateTime, round_up: i64| {
        (((at - origin).num_seconds() + round_up).div_euclid(seconds)).clamp(0, count as i64)
            as usize
    };
    let first = cell(from, 0);
    first..cell(to, seconds - 1).max(first)
}

/// Time each cell holds, where cell `(row, column)` opens at
/// `origin + column_span * column + cell_span * row`.
fn time_cells(
    entries: &[&TimeEntry],
    origin: NaiveDateTime,
    columns: usize,
    column_span: Duration,
    rows: usize,
    cell_span: Duration,
) -> Vec<Vec<Option<Duration>>> {
    let mut cells = vec![vec![Some(Duration::zero()); columns]; rows];
    for entry in entries {
        let (start, end) = entry_span(entry);
        for column in touched(origin, column_span, columns, start, end) {
            let opens = origin + column_span * column as i32;
            for row in touched(opens, cell_span, rows, start, end) {
                let from = opens + cell_span * row as i32;
                let part = overlap(entry, from, from + cell_span);
                if let Some(cell) = cells[row][column].as_mut() {
                    *cell += part;
                }
            }
        }
    }
    cells
}

/// One row: a project, its time in the scope, its entry count and its share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectTotal {
    /// The name as stored, or [`NO_PROJECT`].
    pub(crate) project: String,
    /// Raw; the caller applies `duration::format`.
    pub(crate) total: Duration,
    /// `human + agent` is always `total`, so the columns cannot disagree.
    pub(crate) human: Duration,
    pub(crate) agent: Duration,
    pub(crate) entries: usize,
    /// Percent of the folded total, rounded, and **not** fudged to sum to 100.
    pub(crate) share: u16,
}

impl App {
    /// The entry set both the rows and the heat strips fold.
    fn summary_entries(&self) -> Vec<&TimeEntry> {
        if self.summary_follows_filters {
            self.filtered_entries()
        } else {
            self.scope_entries()
        }
    }

    /// Per-project totals, largest first. Folds
    /// [`filtered_entries`](Self::filtered_entries) while
    /// `summary_follows_filters` is on — the panes and the `/` search term —
    /// else [`scope_entries`](Self::scope_entries). Nothing folded gives an
    /// empty list.
    pub(crate) fn project_summary(&self) -> Vec<ProjectTotal> {
        let entries = self.summary_entries();
        // (total, human, agent, entries)
        let mut totals: HashMap<&str, (Duration, Duration, Duration, usize)> = HashMap::new();
        for entry in &entries {
            let row = totals.entry(project_key(entry)).or_insert((
                Duration::zero(),
                Duration::zero(),
                Duration::zero(),
                0,
            ));
            let duration = entry.duration();
            row.0 += duration;
            if entry.is_agent() {
                row.2 += duration;
            } else {
                row.1 += duration;
            }
            row.3 += 1;
        }

        let folded_total: i64 = totals.values().map(|row| row.0.num_seconds()).sum();
        let mut rows: Vec<ProjectTotal> = totals
            .into_iter()
            .map(|(project, (total, human, agent, entries))| ProjectTotal {
                project: project.to_string(),
                total,
                human,
                agent,
                entries,
                share: share_of(total, folded_total),
            })
            .collect();
        // Ties broken by name, so the order is stable rather than the HashMap's.
        rows.sort_by(|a, b| {
            b.total
                .cmp(&a.total)
                .then_with(|| a.project.cmp(&b.project))
        });
        rows
    }

    /// The grain of the current view plus one bucket list per row of `rows`,
    /// index for index, oldest bucket first. Every entry's whole duration goes
    /// to the bucket holding its `start_time`, so a row's buckets sum to its
    /// [`ProjectTotal::total`]. Sheds nothing: the
    /// renderer owns the width and must take its maximum over what it draws.
    pub(crate) fn project_buckets(&self, rows: &[ProjectTotal]) -> BucketGrid {
        let grain = Grain::for_view(self.view_mode);
        let entries = self.summary_entries();
        let Some((anchor, count)) = bucket_range(self.view_mode, &entries, self.selected_date)
        else {
            return BucketGrid {
                grain,
                anchor: self.selected_date.and_hms_opt(0, 0, 0).unwrap(),
                rows: vec![Vec::new(); rows.len()],
            };
        };

        let mut buckets = vec![vec![Duration::zero(); count]; rows.len()];
        let row_of: HashMap<&str, usize> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.project.as_str(), index))
            .collect();
        for entry in &entries {
            let Some(&row) = row_of.get(project_key(entry)) else {
                continue;
            };
            let slot = bucket_index(grain, anchor, entry.start_time.naive_local());
            if let Ok(slot) = usize::try_from(slot)
                && let Some(cell) = buckets[row].get_mut(slot)
            {
                *cell += entry.duration();
            }
        }
        BucketGrid {
            grain,
            anchor,
            rows: buckets,
        }
    }

    /// The open view's period as a grid of heat cells, at the resolution the
    /// inner box has room for. **It folds `filtered_entries()`, never
    /// `summary_entries()`:** the heat and the entry list must not disagree
    /// about what is on screen.
    pub(crate) fn view_heat_grid(&self, inner_width: u16, inner_height: u16) -> HeatGrid {
        let entries = self.filtered_entries();
        let mut grid = match self.view_mode {
            ViewMode::Day => self.day_heat_grid(&entries, inner_width),
            ViewMode::Week | ViewMode::Month => self.block_heat_grid(&entries, inner_height),
            ViewMode::Year | ViewMode::All => self.day_cell_heat_grid(&entries),
        };
        // The title counts what the grid paints, so the two cannot disagree
        // about an entry that runs past the period.
        grid.total = grid.painted();
        grid
    }

    /// One row per project, largest total first, over the slots of the day.
    fn day_heat_grid(&self, entries: &[&TimeEntry], inner_width: u16) -> HeatGrid {
        /// Columns of a quarter hour each, the finest the day is drawn at.
        const QUARTERS: usize = 96;
        /// Columns one quarter hour takes before the day is drawn that fine.
        const LEAST_CELL: usize = 2;

        let available = (inner_width as usize).saturating_sub(PROJECT_GUTTER);
        let columns = if available >= QUARTERS * LEAST_CELL {
            QUARTERS
        } else {
            24
        };
        let cell_span = Duration::minutes(24 * 60 / columns as i64);
        let opens = midnight(self.selected_date);

        let folded: std::collections::HashSet<&str> =
            entries.iter().map(|entry| project_key(entry)).collect();
        // The rows keep `project_summary` order; a filtered-out project has none.
        let row_labels: Vec<String> = self
            .project_summary()
            .into_iter()
            .map(|row| row.project)
            .filter(|project| folded.contains(project.as_str()))
            .collect();
        let row_of: HashMap<&str, usize> = row_labels
            .iter()
            .enumerate()
            .map(|(index, project)| (project.as_str(), index))
            .collect();

        let mut rows: Vec<Vec<&TimeEntry>> = vec![Vec::new(); row_labels.len()];
        for entry in entries {
            if let Some(&row) = row_of.get(project_key(entry)) {
                rows[row].push(entry);
            }
        }
        let cells = rows
            .iter()
            .flat_map(|row| time_cells(row, opens, columns, cell_span, 1, cell_span))
            .collect();

        let column_ticks = (0..columns)
            .map(|column| axis_tick(opens + cell_span * column as i32, cell_span, columns))
            .collect();

        HeatGrid {
            bands: vec![HeatBand {
                title: None,
                row_labels,
                column_ticks,
                cells,
                cell_span,
                now_row: None,
                now_column: column_now(opens, cell_span, columns),
            }],
            unit: if columns == QUARTERS {
                "quarter"
            } else {
                "hour"
            },
            gutter: PROJECT_GUTTER,
            total: Duration::zero(),
            counts_columns: true,
        }
    }

    /// One column per day of the week or the month, one row per block of the
    /// day, midnight at the top.
    fn block_heat_grid(&self, entries: &[&TimeEntry], inner_height: u16) -> HeatGrid {
        /// Hours a row covers, finest first; the day itself is the last resort.
        const BLOCK_HOURS: [i64; 4] = [2, 4, 6, 24];

        let (opens, columns) = period_bounds(self.view_mode, self.selected_date)
            .unwrap_or_else(|| (midnight(self.selected_date), 7));
        let hours = BLOCK_HOURS
            .into_iter()
            .find(|hours| 24 / hours < i64::from(inner_height))
            .unwrap_or(24);
        let rows = (24 / hours) as usize;
        let cell_span = Duration::hours(hours);
        let column_span = Duration::days(1);

        let cells = time_cells(entries, opens, columns, column_span, rows, cell_span);
        let column_ticks = (0..columns)
            .map(|column| axis_tick(opens + column_span * column as i32, column_span, columns))
            .collect();
        let row_labels = (0..rows)
            .map(|row| format!("{:02}", row as i64 * hours))
            .collect();

        let now_column = column_now(opens, column_span, columns);
        HeatGrid {
            bands: vec![HeatBand {
                title: None,
                row_labels,
                column_ticks,
                cells,
                cell_span,
                now_row: now_column.map(|_| {
                    (i64::from(Local::now().naive_local().hour()) / hours).min(rows as i64 - 1)
                        as usize
                }),
                now_column,
            }],
            unit: "block",
            gutter: HEAT_GUTTER,
            total: Duration::zero(),
            counts_columns: false,
        }
    }

    /// Day-sized cells: one column per week, one row per weekday. The Year is
    /// one band, `All` one band per year that holds an entry, newest first.
    fn day_cell_heat_grid(&self, entries: &[&TimeEntry]) -> HeatGrid {
        // An entry crossing New Year belongs to both bands it runs through.
        let mut by_year: HashMap<i32, Vec<&TimeEntry>> = HashMap::new();
        for entry in entries {
            let (start, end) = entry_span(entry);
            for year in start.year()..=end.year() {
                by_year.entry(year).or_default().push(entry);
            }
        }
        let years: Vec<i32> = if self.view_mode == ViewMode::Year {
            vec![self.selected_date.year()]
        } else {
            let mut years: Vec<i32> = by_year.keys().copied().collect();
            years.sort_unstable_by(|a, b| b.cmp(a));
            years
        };
        HeatGrid {
            bands: years
                .into_iter()
                .map(|year| {
                    let held = by_year.get(&year).map(Vec::as_slice).unwrap_or_default();
                    year_band(held, year)
                })
                .collect(),
            unit: "day",
            gutter: YEAR_GUTTER,
            total: Duration::zero(),
            counts_columns: false,
        }
    }

    /// Height including borders. Collapsed, the box is the total row alone;
    /// expanded, the header and the capped rows sit over the rule and the
    /// total. The strips ride the rows they belong to, so they cost no line.
    pub(crate) fn summary_surface_height(&self) -> u16 {
        if !self.show_summary {
            return 3;
        }
        let rows = self.project_summary().len();
        // The empty box says why it is empty, heads no columns and sums nothing.
        let header = u16::from(rows > 0);
        let total = if rows > 0 { SUMMARY_TOTAL_LINES } else { 0 };
        2 + header + total + rows.clamp(1, MAX_VISIBLE_PROJECTS) as u16
    }

    /// The title bar's right half: `day · all projects`, or `day · filtered`
    /// while following, plus `· 6/9` when rows are off screen and `· split`
    /// while the split is on. The word says the mode; only its colour follows
    /// the filter state.
    pub(crate) fn summary_marker(&self, rows: &[ProjectTotal], visible_rows: usize) -> String {
        let mode = if self.summary_follows_filters {
            FILTERED
        } else {
            ALL_PROJECTS
        };
        let mut marker = format!("{} · {mode}", self.view_mode.label());
        if let Some(count) = summary_count(rows, visible_rows) {
            marker.push_str(" · ");
            marker.push_str(&count);
        }
        if self.summary_split {
            marker.push_str(" · split");
        }
        marker
    }

    /// Focus never reads as resting on a hidden Summary.
    pub(crate) fn summary_is_focused(&self) -> bool {
        self.focus == Focus::Summary && self.show_summary
    }

    /// `v`: show or hide the human / agent columns.
    pub(crate) fn toggle_summary_split(&mut self) {
        self.summary_split = !self.summary_split;
        self.persist_layout();
    }

    /// `M`: draw the content area as a heatmap or as the entry list.
    pub(crate) fn toggle_heat_view(&mut self) {
        self.heat_view = !self.heat_view;
        self.heat_scroll = 0;
        self.persist_layout();
    }

    /// `j`/`k` in a heat grid that overflows its box: the Day view's project
    /// rows and `All`'s year bands. Reports whether the key was claimed, so a
    /// grid with room for everything leaves it to the entries table.
    pub(crate) fn scroll_heat(&mut self, down: bool) -> bool {
        if !self.heat_view || self.heat_scroll_max == 0 {
            return false;
        }
        self.heat_scroll = if down {
            (self.heat_scroll + 1).min(self.heat_scroll_max)
        } else {
            self.heat_scroll.saturating_sub(1)
        };
        true
    }

    /// `m`: show or hide the Summary's per-project heat strips.
    pub(crate) fn toggle_summary_heat(&mut self) {
        self.summary_heat = !self.summary_heat;
        self.persist_layout();
    }

    /// `f`: fold the filtered entries or the whole scope.
    pub(crate) fn toggle_summary_follows_filters(&mut self) {
        self.summary_follows_filters = !self.summary_follows_filters;
        self.persist_layout();
    }

    /// `Shift-S`: show or hide the surface; opening it focuses it.
    pub(crate) fn toggle_summary(&mut self) {
        self.show_summary = !self.show_summary;
        if self.show_summary {
            self.focus = Focus::Summary;
        } else if self.focus == Focus::Summary {
            self.focus = self.focus_after_hiding();
        }
        self.persist_layout();
    }
}

/// One row over **every** project row, on screen or not; `share` is unused.
pub(crate) fn summary_total(rows: &[ProjectTotal]) -> ProjectTotal {
    rows.iter().fold(
        ProjectTotal {
            project: "total".to_string(),
            total: Duration::zero(),
            human: Duration::zero(),
            agent: Duration::zero(),
            entries: 0,
            share: 0,
        },
        |mut acc, row| {
            acc.total += row.total;
            acc.human += row.human;
            acc.agent += row.agent;
            acc.entries += row.entries;
            acc
        },
    )
}

/// The leading rows the box has room for. Takes what `project_summary` folded,
/// so one frame folds the scope once.
pub(crate) fn visible_project_summary(
    rows: &[ProjectTotal],
    visible_rows: usize,
) -> &[ProjectTotal] {
    &rows[..visible_rows.min(rows.len())]
}

/// How wide each cell is and how many of the newest buckets are drawn, for a
/// strip with `available` columns. The cells stretch to the columns the row
/// really has; once one column each is too many, the oldest buckets shed, as
/// `render_year_heatmap` sheds its oldest weeks. Only the remainder of the even
/// division is left over, so the strip is never a whole cell short.
pub(crate) fn strip_cells(available: usize, buckets: usize) -> (usize, usize) {
    if buckets == 0 {
        return (1, 0);
    }
    let cell_width = (available / buckets).max(1);
    (cell_width, (available / cell_width).min(buckets))
}

/// `shown/total` once more projects exist than fit, else `None`.
pub(crate) fn summary_count(rows: &[ProjectTotal], visible_rows: usize) -> Option<String> {
    if rows.len() <= visible_rows {
        return None;
    }
    surface_count(None, rows.len(), visible_rows)
}

/// The summary row an entry belongs to: its trimmed project, or [`NO_PROJECT`].
/// Empty-after-trim counts as absent, as the form and `pane_values` do.
fn project_key(entry: &TimeEntry) -> &str {
    let project = entry.project.as_deref().map(str::trim).unwrap_or("");
    if project.is_empty() {
        NO_PROJECT
    } else {
        project
    }
}

/// The first bucket's start and how many buckets follow it, or `None` when
/// nothing is folded. The **view** says which period the strip covers, so the
/// strip reads against the period rather than against itself. `All` has no
/// bounded period and spans the entries instead.
fn bucket_range(
    view: ViewMode,
    entries: &[&TimeEntry],
    selected: NaiveDate,
) -> Option<(NaiveDateTime, usize)> {
    if entries.is_empty() {
        return None;
    }
    if let Some(bounds) = period_bounds(view, selected) {
        return Some(bounds);
    }
    let earliest = entries.iter().map(|e| e.start_time.naive_local()).min()?;
    let latest = entries.iter().map(|e| e.start_time.naive_local()).max()?;
    let anchor = midnight(TimeData::week_start(earliest.date()));
    let count = (bucket_index(Grain::for_view(view), anchor, latest) + 1).max(1) as usize;
    Some((anchor, count))
}

/// The period a view with bounded one covers: the 24 hours of the selected
/// day, the 7 days of its week, the days of its month, the 12 months of its
/// year. `All` has none.
pub(crate) fn period_bounds(view: ViewMode, selected: NaiveDate) -> Option<(NaiveDateTime, usize)> {
    let first_of_month = NaiveDate::from_ymd_opt(selected.year(), selected.month(), 1)?;
    Some(match view {
        ViewMode::Day => (midnight(selected), 24),
        ViewMode::Week => (midnight(TimeData::week_start(selected)), 7),
        ViewMode::Month => (midnight(first_of_month), days_in_month(first_of_month)),
        ViewMode::Year => (
            midnight(NaiveDate::from_ymd_opt(selected.year(), 1, 1)?),
            12,
        ),
        ViewMode::All => return None,
    })
}

pub(crate) fn midnight(date: NaiveDate) -> NaiveDateTime {
    date.and_hms_opt(0, 0, 0).unwrap()
}

/// The days of the month `first` opens, counted off the calendar. `first` must
/// be the first of its month.
pub(crate) fn days_in_month(first: NaiveDate) -> usize {
    first
        .checked_add_months(Months::new(1))
        .map(|next| (next - first).num_days() as usize)
        .unwrap_or(31)
}

/// How many grains `start` sits past `anchor`. Reads the start alone, never the
/// span, so an entry is never shared between the buckets it runs through.
fn bucket_index(grain: Grain, anchor: NaiveDateTime, start: NaiveDateTime) -> i64 {
    match grain {
        Grain::Hour => (floor_hour(start) - anchor).num_hours(),
        Grain::Day => (start.date() - anchor.date()).num_days(),
        Grain::Week => (start.date() - anchor.date()).num_days().div_euclid(7),
        Grain::Month => {
            (i64::from(start.year()) - i64::from(anchor.year())) * 12 + i64::from(start.month())
                - i64::from(anchor.month())
        }
    }
}

/// One year as weeks across and weekdays down, as a contribution graph draws
/// it. A day outside the year is blank rather than empty.
fn year_band(entries: &[&TimeEntry], year: i32) -> HeatBand {
    const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

    let opens = TimeData::week_start(NaiveDate::from_ymd_opt(year, 1, 1).unwrap());
    let closes =
        TimeData::week_start(NaiveDate::from_ymd_opt(year, 12, 31).unwrap()) + Duration::days(6);
    let columns = ((closes - opens).num_days() / 7 + 1) as usize;
    let day = Duration::days(1);
    let week = Duration::days(7);
    let date_of = |row: usize, column: usize| opens + Duration::days((column * 7 + row) as i64);

    let mut cells = time_cells(entries, midnight(opens), columns, week, 7, day);
    for (row, line) in cells.iter_mut().enumerate() {
        for (column, cell) in line.iter_mut().enumerate() {
            if date_of(row, column).year() != year {
                *cell = None;
            }
        }
    }

    let today = Local::now().date_naive();
    let now = (today.year() == year).then(|| {
        let offset = (today - opens).num_days();
        ((offset % 7) as usize, (offset / 7) as usize)
    });
    HeatBand {
        title: Some(year.to_string()),
        row_labels: WEEKDAYS.iter().map(|day| day.to_string()).collect(),
        column_ticks: (0..columns)
            .map(|column| {
                let monday = date_of(0, column);
                (monday.year() == year)
                    .then(|| axis_tick(midnight(monday), week, columns))
                    .flatten()
            })
            .collect(),
        cells,
        cell_span: day,
        now_row: now.map(|(row, _)| row),
        now_column: now.map(|(_, column)| column),
    }
}

/// Which column holds this moment, or `None` when the period is not now.
fn column_now(opens: NaiveDateTime, column_span: Duration, columns: usize) -> Option<usize> {
    let now = Local::now().naive_local();
    (0..columns).find(|column| {
        let from = opens + column_span * *column as i32;
        from <= now && now < from + column_span
    })
}

/// The entry's wall-clock span, both ends local. A running entry ends now, as
/// `TimeEntry::duration()` reads it. Never add `duration()` to the start: the
/// two differ by an hour across a daylight-saving step.
pub(crate) fn entry_span(entry: &TimeEntry) -> (NaiveDateTime, NaiveDateTime) {
    (
        entry.start_time.naive_local(),
        entry.end_time.unwrap_or_else(Local::now).naive_local(),
    )
}

/// How much of `entry` falls inside the half-open window `[from, to)`, zero
/// when the two are disjoint. Idle stretches stay counted.
pub(crate) fn overlap(entry: &TimeEntry, from: NaiveDateTime, to: NaiveDateTime) -> Duration {
    let (start, end) = entry_span(entry);
    let first = start.max(from);
    let last = end.min(to);
    if last > first {
        last - first
    } else {
        Duration::zero()
    }
}

fn floor_hour(at: NaiveDateTime) -> NaiveDateTime {
    at.date().and_hms_opt(at.hour(), 0, 0).unwrap()
}

/// `part` as a whole-percent share of `whole`, in seconds; a zero `whole` is 0%.
fn share_of(part: Duration, whole: i64) -> u16 {
    if whole <= 0 {
        return 0;
    }
    let part = part.num_seconds().max(0);
    // Round half up in integers, so the same input always gives the same point.
    (((part * 200) / whole + 1) / 2) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn every_view_mode_maps_to_its_own_grain() {
        assert_eq!(Grain::for_view(ViewMode::Day), Grain::Hour);
        assert_eq!(Grain::for_view(ViewMode::Week), Grain::Day);
        assert_eq!(Grain::for_view(ViewMode::All), Grain::Week);
        assert_eq!(Grain::for_view(ViewMode::Month), Grain::Day);
        assert_eq!(Grain::for_view(ViewMode::Year), Grain::Month);
    }

    /// The index keys on the start alone, so a span never reaches the next bucket.
    #[test]
    fn a_bucket_index_counts_grains_from_the_anchor_to_the_start() {
        let anchor = at(2026, 1, 1, 0, 0);
        assert_eq!(
            bucket_index(Grain::Hour, anchor, at(2026, 1, 1, 23, 30)),
            23
        );
        assert_eq!(bucket_index(Grain::Hour, anchor, at(2026, 1, 2, 0, 30)), 24);
        assert_eq!(bucket_index(Grain::Day, anchor, at(2026, 1, 2, 0, 30)), 1);
        assert_eq!(bucket_index(Grain::Week, anchor, at(2026, 1, 15, 9, 0)), 2);
        assert_eq!(
            bucket_index(Grain::Month, anchor, at(2026, 12, 31, 9, 0)),
            11
        );
        assert_eq!(bucket_index(Grain::Month, anchor, at(2027, 2, 1, 9, 0)), 13);
    }

    fn entry_at(start: NaiveDateTime) -> TimeEntry {
        TimeEntry {
            id: 0,
            description: "seed".to_string(),
            project: Some("tt".to_string()),
            tags: Vec::new(),
            start_time: start.and_local_timezone(chrono::Local).unwrap(),
            end_time: None,
            idle: Vec::new(),
            data: None,
        }
    }

    /// The month's own length, taken from the calendar rather than a table.
    #[test]
    fn a_month_view_buckets_the_days_of_the_selected_month() {
        let seed = entry_at(at(2026, 2, 10, 9, 0));
        let entries = vec![&seed];

        let (anchor, count) = bucket_range(
            ViewMode::Month,
            &entries,
            NaiveDate::from_ymd_opt(2026, 2, 10).unwrap(),
        )
        .unwrap();
        assert_eq!(anchor, at(2026, 2, 1, 0, 0), "bucket 0 opens the month");
        assert_eq!(count, 28, "February 2026 has 28 days");

        let (anchor, count) = bucket_range(
            ViewMode::Month,
            &entries,
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        )
        .unwrap();
        assert_eq!(anchor, at(2026, 1, 1, 0, 0));
        assert_eq!(count, 31, "January has 31 days");
    }

    /// The Week view keeps its own seven days while Month shares its grain.
    #[test]
    fn a_week_view_still_buckets_seven_days_from_its_monday() {
        let seed = entry_at(at(2026, 2, 10, 9, 0));
        let entries = vec![&seed];

        let (anchor, count) = bucket_range(
            ViewMode::Week,
            &entries,
            NaiveDate::from_ymd_opt(2026, 2, 10).unwrap(),
        )
        .unwrap();
        assert_eq!(anchor, at(2026, 2, 9, 0, 0), "the week opens on Monday");
        assert_eq!(count, 7);
    }

    /// The cells fill the row: what is left over is under one cell's worth,
    /// and a row too narrow for the buckets sheds instead of shrinking further.
    #[test]
    fn strip_cells_fill_the_room_and_then_shed() {
        for buckets in [1usize, 7, 12, 24, 53] {
            for available in 0..200usize {
                let (cell_width, cells) = strip_cells(available, buckets);
                assert!(cell_width >= 1, "a cell is never nothing");
                assert!(cells <= buckets, "more cells drawn than buckets held");
                assert!(cells * cell_width <= available, "the strip overran");
                if available >= buckets {
                    assert_eq!(cells, buckets, "every bucket fits and must be drawn");
                    assert!(
                        available - cells * cell_width < buckets,
                        "{available} columns over {buckets} buckets left a whole cell unused"
                    );
                } else {
                    assert_eq!(cell_width, 1, "a shed strip keeps the narrowest cell");
                    assert_eq!(cells, available, "a shed strip fills what is left");
                }
            }
        }
        assert_eq!(strip_cells(40, 0), (1, 0), "nothing folded draws nothing");
    }
    #[test]
    fn an_entry_splits_over_the_windows_it_crosses() {
        let seed = {
            let mut entry = entry_at(at(2026, 1, 1, 9, 30));
            entry.end_time = Some(
                at(2026, 1, 1, 12, 15)
                    .and_local_timezone(chrono::Local)
                    .unwrap(),
            );
            entry
        };
        let window = |hour: u32| {
            (
                at(2026, 1, 1, hour, 0),
                at(2026, 1, 1, hour, 0) + Duration::hours(1),
            )
        };

        let parts: Vec<i64> = (9..13)
            .map(|hour| {
                let (from, to) = window(hour);
                overlap(&seed, from, to).num_minutes()
            })
            .collect();
        assert_eq!(parts, vec![30, 60, 60, 15]);
        assert_eq!(
            parts.iter().sum::<i64>(),
            seed.duration().num_minutes(),
            "the windows lost time the entry held"
        );

        let (from, to) = window(8);
        assert_eq!(overlap(&seed, from, to), Duration::zero(), "before it");
        let (from, to) = window(13);
        assert_eq!(overlap(&seed, from, to), Duration::zero(), "after it");
        assert_eq!(
            overlap(&seed, at(2026, 1, 1, 10, 0), at(2026, 1, 1, 11, 0)),
            Duration::hours(1),
            "a window inside the entry is full"
        );
    }

    fn spanning(id: u64, project: &str, start: NaiveDateTime, minutes: i64) -> TimeEntry {
        let start = start.and_local_timezone(chrono::Local).unwrap();
        TimeEntry {
            id,
            description: "seed".to_string(),
            project: Some(project.to_string()),
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
        app
    }

    fn band_total(band: &HeatBand) -> Duration {
        band.cells
            .iter()
            .flatten()
            .flatten()
            .fold(Duration::zero(), |acc, held| acc + *held)
    }

    /// The day is projects down and the time of day across, as fine as the
    /// box is wide.
    #[test]
    fn a_day_grid_is_one_row_per_project_over_the_time_of_day() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-day");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let app = app_for(
            ViewMode::Day,
            day,
            vec![
                spanning(0, "alpha", at(2026, 1, 7, 9, 0), 60),
                spanning(1, "beta", at(2026, 1, 7, 10, 0), 120),
            ],
        );

        let wide = app.view_heat_grid(204, 20);
        assert_eq!(wide.bands[0].columns(), 96, "a wide box takes quarters");
        assert_eq!(wide.unit, "quarter");
        // 198 columns less the gutter leave 186 for 96 quarters: under two
        // each, so the day stays hourly and the hours stretch instead.
        assert_eq!(app.view_heat_grid(200, 20).bands[0].columns(), 24);

        let grid = app.view_heat_grid(80, 20);
        let band = &grid.bands[0];
        assert_eq!(band.columns(), 24, "80 columns is an hourly day");
        assert_eq!(grid.unit, "hour");
        assert_eq!(
            band.row_labels,
            vec!["beta".to_string(), "alpha".to_string()],
            "the rows are not in project_summary order"
        );
        assert_eq!(band.cells[1][9], Some(Duration::hours(1)), "alpha at 09");
        assert_eq!(band.cells[1][10], Some(Duration::zero()));
        assert_eq!(band.cells[0][10], Some(Duration::hours(1)), "beta at 10");
        assert_eq!(band_total(band), grid.total, "the band lost folded time");
    }

    /// A short box coarsens its rows instead of hiding a part of the day.
    #[test]
    fn week_rows_coarsen_until_they_fit_the_box() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-week");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let app = app_for(
            ViewMode::Week,
            day,
            vec![spanning(0, "alpha", at(2026, 1, 7, 9, 0), 60)],
        );

        for (height, rows) in [(20, 12), (13, 12), (12, 6), (7, 6), (6, 4), (4, 1)] {
            let grid = app.view_heat_grid(80, height);
            assert_eq!(
                grid.bands[0].rows(),
                rows,
                "{height} inner rows gave the wrong block size"
            );
        }

        let grid = app.view_heat_grid(80, 20);
        let band = &grid.bands[0];
        assert_eq!(band.columns(), 7, "the week is seven days across");
        assert_eq!(band.row_labels[0], "00", "midnight is the top row");
        assert_eq!(band.row_labels[11], "22");
        assert_eq!(band.cell_span, Duration::hours(2));
        assert_eq!(
            band.column_ticks[0].as_deref(),
            Some("Mon"),
            "the week opens on Monday"
        );
        // Wednesday 09:00, in the 08-10 block.
        assert_eq!(band.cells[4][2], Some(Duration::hours(1)));
    }

    /// A month is its own length across, ticked every seventh day.
    #[test]
    fn a_month_grid_has_one_column_per_day_of_the_month() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-month");
        let day = NaiveDate::from_ymd_opt(2026, 2, 10).unwrap();
        let app = app_for(
            ViewMode::Month,
            day,
            vec![spanning(0, "alpha", at(2026, 2, 10, 9, 0), 30)],
        );

        let grid = app.view_heat_grid(80, 20);
        let band = &grid.bands[0];
        assert_eq!(band.columns(), 28, "February 2026 has 28 days");
        assert_eq!(band.column_ticks[0].as_deref(), Some("1"));
        assert_eq!(band.column_ticks[7].as_deref(), Some("8"));
        assert_eq!(band.column_ticks[1], None, "every day was ticked");
        assert_eq!(band.cells[4][9], Some(Duration::minutes(30)));
        assert_eq!(band_total(band), grid.total);
    }

    /// A cell takes the part of the entry that ran inside it, so a night
    /// shift paints both of the days it touches.
    #[test]
    fn an_entry_crossing_midnight_paints_both_days() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-midnight");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let app = app_for(
            ViewMode::Week,
            day,
            vec![spanning(0, "alpha", at(2026, 1, 7, 23, 0), 120)],
        );

        let grid = app.view_heat_grid(80, 20);
        let band = &grid.bands[0];
        assert_eq!(band.cells[11][2], Some(Duration::hours(1)), "Wednesday 23");
        assert_eq!(band.cells[0][3], Some(Duration::hours(1)), "Thursday 00");
        assert_eq!(band_total(band), grid.total, "the crossing lost time");
        assert_eq!(grid.active(), 2, "two cells hold the shift");
    }

    /// A filter narrows the grid as it narrows the list under it.
    #[test]
    fn a_project_filter_narrows_the_grid() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-filter");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let mut app = app_for(
            ViewMode::Day,
            day,
            vec![
                spanning(0, "alpha", at(2026, 1, 7, 9, 0), 60),
                spanning(1, "beta", at(2026, 1, 7, 10, 0), 120),
            ],
        );
        app.project_filter.cycle("beta", true);

        let grid = app.view_heat_grid(80, 20);
        assert_eq!(grid.bands[0].row_labels, vec!["beta".to_string()]);
        assert_eq!(grid.total, Duration::hours(2));
    }

    /// The year is weeks across and weekdays down, and the days of its
    /// neighbours stay blank.
    #[test]
    fn a_year_grid_is_seven_weekday_rows_over_its_weeks() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-year");
        let day = NaiveDate::from_ymd_opt(2026, 3, 4).unwrap();
        let app = app_for(
            ViewMode::Year,
            day,
            vec![spanning(0, "alpha", at(2026, 3, 4, 9, 0), 120)],
        );

        let grid = app.view_heat_grid(120, 20);
        assert_eq!(grid.unit, "day");
        let band = &grid.bands[0];
        assert_eq!(band.rows(), 7);
        assert_eq!(band.row_labels[0], "Mon");
        assert_eq!(band.cell_span, Duration::days(1));
        assert_eq!(
            band.title.as_deref(),
            Some("2026"),
            "the Year view names its band too"
        );
        assert_eq!(band.columns(), 53, "2026 spans 53 grid weeks");
        // 2026 opens on a Thursday, so its first whole grid week is January's.
        assert_eq!(band.column_ticks[0], None);
        assert_eq!(band.column_ticks[1].as_deref(), Some("Jan"));
        // 2026-03-04 is a Wednesday, in the ninth grid week.
        assert_eq!(band.cells[2][9], Some(Duration::hours(2)));
        assert_eq!(band.cells[0][0], None, "December is not part of 2026");
    }

    /// Every heat surface folds the filtered entries, the year grid included.
    #[test]
    fn a_project_filter_narrows_the_year_grid() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-year-filter");
        let day = NaiveDate::from_ymd_opt(2026, 3, 4).unwrap();
        let mut app = app_for(
            ViewMode::Year,
            day,
            vec![
                spanning(0, "alpha", at(2026, 3, 4, 9, 0), 120),
                spanning(1, "beta", at(2026, 3, 5, 9, 0), 60),
            ],
        );
        assert_eq!(app.view_heat_grid(120, 20).total, Duration::hours(3));

        app.project_filter.cycle("alpha", true);
        let grid = app.view_heat_grid(120, 20);
        assert_eq!(grid.total, Duration::hours(2));
        assert_eq!(grid.bands[0].cells[3][9], Some(Duration::zero()), "beta");
    }

    /// `All` stacks a band per year that holds time, newest at the top.
    #[test]
    fn the_all_view_stacks_one_band_per_year_newest_first() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-all");
        let app = app_for(
            ViewMode::All,
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
            vec![
                spanning(0, "alpha", at(2024, 3, 4, 9, 0), 60),
                spanning(1, "alpha", at(2026, 3, 4, 9, 0), 60),
                spanning(2, "alpha", at(2022, 3, 4, 9, 0), 60),
            ],
        );

        let grid = app.view_heat_grid(120, 30);
        let titles: Vec<Option<&str>> = grid
            .bands
            .iter()
            .map(|band| band.title.as_deref())
            .collect();
        assert_eq!(titles, vec![Some("2026"), Some("2024"), Some("2022")]);
        assert_eq!(grid.total, Duration::hours(3));
        assert_eq!(grid.active(), 3, "one day of each year holds time");
    }

    /// The window from an entry's own start to its own end holds all of it,
    /// whatever the clock did in between.
    #[test]
    fn an_entry_fills_the_window_of_its_own_span() {
        let seed = spanning(0, "alpha", at(2026, 3, 29, 1, 0), 180);
        let (start, end) = entry_span(&seed);
        assert_eq!(overlap(&seed, start, end), end - start);

        let running = entry_at(at(2026, 3, 29, 1, 0));
        let (start, end) = entry_span(&running);
        assert_eq!(overlap(&running, start, end), end - start);
    }

    /// An entry running past the period's edge is painted up to the edge, and
    /// the total says so: the title cannot promise time the grid never drew.
    #[test]
    fn the_total_counts_the_time_the_grid_paints() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("heat-grid-edge");
        // Sunday, the last day of its week.
        let sunday = NaiveDate::from_ymd_opt(2026, 1, 11).unwrap();
        let app = app_for(
            ViewMode::Week,
            sunday,
            vec![spanning(0, "alpha", at(2026, 1, 11, 23, 0), 120)],
        );

        let grid = app.view_heat_grid(80, 20);
        let band = &grid.bands[0];
        assert_eq!(band.cells[11][6], Some(Duration::hours(1)), "Sunday 23");
        assert_eq!(grid.total, Duration::hours(1), "the total outran the box");
        assert_eq!(band_total(band), grid.total);
        assert_eq!(grid.active(), 1);
    }

    /// One tick per cell would be a wall of numbers; a clock axis names every
    /// sixth hour, whatever the cells are worth.
    #[test]
    fn a_clock_axis_names_every_sixth_hour() {
        let hourly: Vec<Option<String>> = (0..24)
            .map(|hour| {
                axis_tick(
                    at(2026, 1, 1, 0, 0) + Duration::hours(hour),
                    Duration::hours(1),
                    24,
                )
            })
            .collect();
        assert_eq!(hourly[0].as_deref(), Some("00"));
        assert_eq!(hourly[6].as_deref(), Some("06"));
        assert_eq!(hourly[7], None);

        let quarters: Vec<Option<String>> = (0..96)
            .map(|cell| {
                axis_tick(
                    at(2026, 1, 1, 0, 0) + Duration::minutes(15 * cell),
                    Duration::minutes(15),
                    96,
                )
            })
            .collect();
        assert_eq!(quarters[24].as_deref(), Some("06"), "06:00 is unnamed");
        assert_eq!(quarters[25], None, "a quarter past six took a tick");
    }

    /// Seven day cells are a week, and a week names its weekdays.
    #[test]
    fn a_week_long_day_axis_keeps_its_weekday_names() {
        let names: Option<Vec<String>> = (0..7)
            .map(|day| {
                axis_tick(
                    at(2026, 1, 1, 0, 0) + Duration::days(day),
                    Duration::days(1),
                    7,
                )
            })
            .collect();
        assert_eq!(
            names.expect("every weekday is named"),
            vec!["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"],
        );
    }

    /// A month of weekday names orients nobody, so the long day axis takes
    /// the date of every seventh cell instead.
    #[test]
    fn a_day_axis_longer_than_a_week_is_labelled_every_seventh_date() {
        let labelled: Vec<(usize, String)> = (0..31)
            .filter_map(|day| {
                axis_tick(
                    at(2026, 1, 1, 0, 0) + Duration::days(day as i64),
                    Duration::days(1),
                    31,
                )
                .map(|text| (day, text))
            })
            .collect();
        assert_eq!(
            labelled,
            vec![
                (0, "1".to_string()),
                (7, "8".to_string()),
                (14, "15".to_string()),
                (21, "22".to_string()),
                (28, "29".to_string()),
            ]
        );
    }

    /// A week cell names the month it opens; a month cell names its own.
    #[test]
    fn the_week_and_month_axes_name_their_months() {
        let weeks: Vec<Option<String>> = (0..10)
            .map(|week| {
                axis_tick(
                    at(2026, 1, 1, 0, 0) + Duration::days(week * 7),
                    Duration::days(7),
                    10,
                )
            })
            .collect();
        assert_eq!(weeks[0].as_deref(), Some("Jan"), "the week opening a month");
        assert_eq!(weeks[1], None);

        let months: Vec<Option<String>> = (0..12)
            .map(|month| {
                let start = at(2026, 1, 1, 0, 0)
                    .checked_add_months(Months::new(month))
                    .unwrap();
                let next = at(2026, 1, 1, 0, 0)
                    .checked_add_months(Months::new(month + 1))
                    .unwrap();
                axis_tick(start, next - start, 12)
            })
            .collect();
        assert_eq!(months[0].as_deref(), Some("Jan"));
        assert_eq!(months[11].as_deref(), Some("Dec"));
    }
}
