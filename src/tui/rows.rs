//! The entries table's row model: the filtered entries with every issue's
//! entries folded into one group header, expanded or not.

use super::App;
use super::cache::RowKey;
use super::types::ViewMode;
use chrono::{DateTime, Local, NaiveDate};
use std::collections::HashMap;

/// One line of the entries table. `Entry` holds an index into `data.entries`,
/// never a reference, so the row cache does not borrow `data`.
#[derive(Clone, PartialEq)]
pub(crate) enum VisibleRow {
    DayHeader {
        date: NaiveDate,
    },
    GroupHeader(GroupHeader),
    Entry {
        index: usize,
        /// Where this row sits in its group; `None` for a top-level row.
        member: Option<Member>,
    },
}

/// What the table cursor is anchored to while the store is reloaded. A header
/// anchors by tag, never by a member's id: resolving an id would prefer that
/// member's own row and slide the cursor off the header it was on.
pub(crate) enum CursorAnchor {
    Header { tag: String, date: NaiveDate },
    Entry(u64),
}

/// A member row's place under its group header.
#[derive(Clone, PartialEq)]
pub(crate) struct Member {
    /// The item tag of the group whose header sits above this row.
    pub(crate) tag: String,
    /// The group's last member, which closes the tree with `\u{2514}`.
    pub(crate) last: bool,
}

/// One issue's entries, summarised for the collapsed row.
#[derive(Clone, PartialEq)]
pub(crate) struct GroupHeader {
    /// The item tag as stored, carrying no `#`.
    pub(crate) tag: String,
    /// Indices into `data.entries`, in the order `filtered_entries` gave them.
    pub(crate) members: Vec<usize>,
    pub(crate) expanded: bool,
    /// The earliest member's start, and the latest member's end — `None` while
    /// any member is still running.
    pub(crate) start: DateTime<Local>,
    pub(crate) end: Option<DateTime<Local>>,
    /// The members' project when every member agrees; `None` otherwise.
    pub(crate) project: Option<String>,
}

impl VisibleRow {
    /// Whether the table cursor can land on this row. `table_state.selected()`
    /// counts these rows only, so a day header is never an index.
    fn is_selectable(&self) -> bool {
        !matches!(self, Self::DayHeader { .. })
    }
}

impl App {
    /// The rows the entries table draws, in sort order. Cached against
    /// [`RowKey`], so the many calls a single frame makes cost one walk.
    pub(crate) fn rows(&self) -> Vec<VisibleRow> {
        self.with_rows(<[VisibleRow]>::to_vec)
    }

    /// How many rows the cursor can land on: entries and group headers.
    pub(crate) fn selectable_len(&self) -> usize {
        self.with_rows(|rows| rows.iter().filter(|row| row.is_selectable()).count())
    }

    /// The row under the cursor, counting selectable rows only.
    pub(crate) fn selected_row(&self) -> Option<VisibleRow> {
        let idx = self.table_state.selected()?;
        self.with_rows(|rows| {
            rows.iter()
                .filter(|row| row.is_selectable())
                .nth(idx)
                .cloned()
        })
    }

    /// The selectable index showing `id`: its own row where it has one, else the
    /// header of the collapsed group that holds it. Its own row wins — an
    /// expanded member is on screen twice over, once as itself.
    pub(crate) fn selectable_index_of(&self, id: u64) -> Option<usize> {
        let is_id = |index: &usize| self.data.entries.get(*index).is_some_and(|e| e.id == id);
        self.with_rows(|rows| {
            let selectable = || rows.iter().filter(|row| row.is_selectable());
            selectable()
                .position(|row| matches!(row, VisibleRow::Entry { index, .. } if is_id(index)))
                .or_else(|| {
                    selectable().position(|row| {
                        matches!(row, VisibleRow::GroupHeader(header)
                            if header.members.iter().any(is_id))
                    })
                })
        })
    }

    /// What the cursor is on, in terms that survive a reload.
    pub(crate) fn cursor_anchor(&self) -> Option<CursorAnchor> {
        match self.selected_row()? {
            VisibleRow::Entry { index, .. } => self
                .data
                .entries
                .get(index)
                .map(|entry| CursorAnchor::Entry(entry.id)),
            VisibleRow::GroupHeader(header) => Some(CursorAnchor::Header {
                tag: header.tag,
                date: header.start.date_naive(),
            }),
            VisibleRow::DayHeader { .. } => None,
        }
    }

    /// Where `anchor` sits now. A header anchor prefers the header in the day
    /// segment it was in, then any header for the tag.
    pub(crate) fn selectable_index_for(&self, anchor: &CursorAnchor) -> Option<usize> {
        let (tag, date) = match anchor {
            CursorAnchor::Entry(id) => return self.selectable_index_of(*id),
            CursorAnchor::Header { tag, date } => (tag, date),
        };
        let is_header = |row: &VisibleRow, same_day: bool| match row {
            VisibleRow::GroupHeader(header) => {
                header.tag == *tag && (!same_day || header.start.date_naive() == *date)
            }
            _ => false,
        };
        self.with_rows(|rows| {
            let selectable = || rows.iter().filter(|row| row.is_selectable());
            selectable()
                .position(|row| is_header(row, true))
                .or_else(|| selectable().position(|row| is_header(row, false)))
        })
    }

    /// Flip the expanded state of the group the cursor is in; a no-op on a
    /// top-level entry row and on an empty list.
    ///
    /// On a header the selected index is **left alone** — expanding inserts
    /// rows after the header, so the cursor stays on it. On a member the group
    /// collapses and the cursor follows onto the header that replaces it.
    #[allow(
        clippy::collapsible_match,
        reason = "`remove` is the toggle's first half; a match guard would hide it"
    )]
    pub(crate) fn toggle_group_at_cursor(&mut self) {
        match self.selected_row() {
            Some(VisibleRow::GroupHeader(header)) => {
                if !self.expanded_issues.remove(&header.tag) {
                    self.expanded_issues.insert(header.tag);
                    self.reveal_group_members = true;
                }
            }
            Some(VisibleRow::Entry {
                index,
                member: Some(member),
            }) => {
                let id = self.data.entries.get(index).map(|entry| entry.id);
                self.expanded_issues.remove(&member.tag);
                // Collapsed, the id has no row of its own, so this resolves to
                // the header holding it — in Week view, the right day's.
                if let Some(id) = id {
                    self.select_by_id(id);
                }
            }
            _ => {}
        }
    }

    /// Run `f` over the cached rows, recomputing them first if any input the
    /// [`RowKey`] covers has moved since they were built.
    fn with_rows<T>(&self, f: impl FnOnce(&[VisibleRow]) -> T) -> T {
        let key = RowKey::of(self);
        let stale = match &*self.rows_cache.borrow() {
            Some((cached, _)) => *cached != key,
            None => true,
        };
        if stale {
            let rows = self.compute_rows();
            *self.rows_cache.borrow_mut() = Some((key, rows));
        }
        let cache = self.rows_cache.borrow();
        let (_, rows) = cache.as_ref().expect("filled just above");
        f(rows)
    }

    /// The uncached walk: day partitions in Week view, each partitioned again
    /// into issues, so a group header exists per issue *per day*.
    fn compute_rows(&self) -> Vec<VisibleRow> {
        let indices = self.filtered_indices();
        let mut rows = Vec::with_capacity(indices.len());

        if self.view_mode != ViewMode::Week {
            self.push_segment(&indices, &mut rows);
            return rows;
        }

        // Sorting is by start time, so a date's entries are one contiguous run.
        let mut segment: Vec<usize> = Vec::new();
        let mut current: Option<NaiveDate> = None;
        for index in indices {
            let Some(entry) = self.data.entries.get(index) else {
                continue;
            };
            let date = entry.start_time.date_naive();
            if current != Some(date) {
                self.push_segment(&segment, &mut rows);
                segment.clear();
                current = Some(date);
                rows.push(VisibleRow::DayHeader { date });
            }
            segment.push(index);
        }
        self.push_segment(&segment, &mut rows);
        rows
    }

    /// Append one day partition's rows: an issue with two or more entries as a
    /// group header, anything else as the plain entry row it is today.
    fn push_segment(&self, segment: &[usize], rows: &mut Vec<VisibleRow>) {
        enum Slot {
            Entry(usize),
            Group(usize),
        }
        let mut slots: Vec<Slot> = Vec::with_capacity(segment.len());
        let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();

        for index in segment {
            let Some(entry) = self.data.entries.get(*index) else {
                continue;
            };
            let Some(tag) = crate::report::classify(&entry.tags).0 else {
                slots.push(Slot::Entry(*index));
                continue;
            };

            match seen.get(tag) {
                Some(group) => groups[*group].1.push(*index),
                None => {
                    seen.insert(tag, groups.len());
                    slots.push(Slot::Group(groups.len()));
                    groups.push((tag, vec![*index]));
                }
            }
        }

        let top_level = |index: usize| VisibleRow::Entry {
            index,
            member: None,
        };
        for slot in slots {
            match slot {
                Slot::Entry(index) => rows.push(top_level(index)),
                Slot::Group(group) => {
                    let (tag, members) = &groups[group];
                    if members.len() < 2 {
                        rows.push(top_level(members[0]));
                        continue;
                    }
                    rows.push(VisibleRow::GroupHeader(self.summarise(tag, members)));
                    if self.expanded_issues.contains(*tag) {
                        let last = members.len() - 1;
                        rows.extend(members.iter().enumerate().map(|(nth, index)| {
                            VisibleRow::Entry {
                                index: *index,
                                member: Some(Member {
                                    tag: tag.to_string(),
                                    last: nth == last,
                                }),
                            }
                        }));
                    }
                }
            }
        }
    }

    /// The collapsed row's clock-free figures for `members`, which are never
    /// empty. **No duration here** — a running member's moves with the clock and
    /// [`RowKey`] has no clock input, so the sums are the renderer's per frame.
    fn summarise(&self, tag: &str, members: &[usize]) -> GroupHeader {
        let entries = || members.iter().filter_map(|i| self.data.entries.get(*i));
        let mut projects = entries().map(crate::tui::summary::project_key);
        let first = projects.next();
        let project = if projects.all(|p| Some(p) == first) {
            first.map(str::to_string)
        } else {
            None
        };
        GroupHeader {
            tag: tag.to_string(),
            project,
            members: members.to_vec(),
            expanded: self.expanded_issues.contains(tag),
            start: entries()
                .map(|e| e.start_time)
                .min()
                .unwrap_or_else(Local::now),
            end: if entries().any(|e| e.end_time.is_none()) {
                None
            } else {
                entries().filter_map(|e| e.end_time).max()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VisibleRow;
    use crate::storage;
    use crate::storage::env_guard;
    use crate::storage::env_sandbox as sandbox;
    use crate::tracker::{TimeData, TimeEntry};
    use crate::tui::App;
    use crate::tui::types::ViewMode;
    use chrono::{Duration, Local, NaiveDate};

    fn seed(entries: Vec<TimeEntry>) {
        let next_id = entries.len() as u64;
        storage::save_data(&TimeData {
            entries,
            next_id,
            schema_version: 1,
        })
        .unwrap();
    }

    /// A half-hour entry starting at `hour` on `date`.
    fn at(id: u64, description: &str, tags: &[&str], date: NaiveDate, hour: u32) -> TimeEntry {
        let start = date
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        TimeEntry {
            id,
            description: description.to_string(),
            project: Some("tt".to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: start,
            end_time: Some(start + chrono::Duration::minutes(30)),
            idle: Vec::new(),
            data: None,
        }
    }

    /// Each row as one line: `day <date> <total>`, `group <tag> <members>
    /// <total>`, or `entry <description>`, a member's behind its connector.
    fn shape(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .map(|row| match row {
                VisibleRow::DayHeader { date } => format!("day {date}"),
                VisibleRow::GroupHeader(header) => format!(
                    "group {} {} {}",
                    header.tag,
                    header.members.len(),
                    crate::duration::format(
                        header
                            .members
                            .iter()
                            .filter_map(|i| app.data.entries.get(*i))
                            .fold(Duration::zero(), |acc, e| acc + e.duration())
                    )
                ),
                VisibleRow::Entry { index, member } => format!(
                    "entry {}{}",
                    match member {
                        Some(m) if m.last => "\u{2514} ",
                        Some(_) => "\u{251c} ",
                        None => "",
                    },
                    app.data.entries[*index].description
                ),
            })
            .collect()
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    #[test]
    fn untagged_and_single_member_issues_stay_one_row_each() {
        let _guard = env_guard();
        sandbox("rows-singletons");
        seed(vec![
            at(0, "loose", &[], today(), 9),
            at(1, "only member", &["impl", "tt/8"], today(), 10),
            at(2, "other issue", &["tt/9"], today(), 11),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;

        assert_eq!(
            shape(&app),
            vec!["entry other issue", "entry only member", "entry loose"]
        );
        assert_eq!(app.rows().len(), app.filtered_entries().len());
    }

    #[test]
    fn a_group_of_missing_and_blank_projects_keys_its_header_as_no_project() {
        let _guard = env_guard();
        sandbox("rows-no-project-header");
        let mut none = at(0, "none", &["tt/8"], today(), 9);
        none.project = None;
        let mut blank = at(1, "blank", &["tt/8"], today(), 10);
        blank.project = Some("  ".to_string());
        seed(vec![none, blank]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;

        let header = app.rows().into_iter().find_map(|row| match row {
            VisibleRow::GroupHeader(header) => Some(header),
            _ => None,
        });
        assert_eq!(
            header.unwrap().project.as_deref(),
            Some(crate::tui::summary::NO_PROJECT)
        );
    }

    #[test]
    fn three_entries_on_one_issue_collapse_to_a_header_carrying_their_total() {
        let _guard = env_guard();
        sandbox("rows-collapse");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
            at(2, "round three", &["tt/8"], today(), 11),
            at(3, "loose", &[], today(), 12),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;

        assert_eq!(shape(&app), vec!["entry loose", "group tt/8 3 1h 30m"]);
    }

    #[test]
    fn an_expanded_issue_shows_its_header_and_then_its_members() {
        let _guard = env_guard();
        sandbox("rows-expand");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
            at(2, "round three", &["tt/8"], today(), 11),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.expanded_issues.insert("tt/8".to_string());

        assert_eq!(
            shape(&app),
            vec![
                "group tt/8 3 1h 30m",
                "entry \u{251c} round three",
                "entry \u{251c} round two",
                "entry \u{2514} round one",
            ]
        );
    }

    /// One issue over two days: a header per day, each under its day header.
    #[test]
    fn week_view_nests_a_group_header_inside_every_day_partition() {
        let _guard = env_guard();
        sandbox("rows-week");
        let week_start = TimeData::week_start(today());
        let day_two = week_start + chrono::Duration::days(1);
        seed(vec![
            at(0, "mon one", &["tt/8"], week_start, 9),
            at(1, "mon two", &["tt/8"], week_start, 10),
            at(2, "tue one", &["tt/8"], day_two, 9),
            at(3, "tue two", &["tt/8"], day_two, 10),
        ]);
        let mut app = App::new().unwrap();
        app.selected_date = week_start;
        app.view_mode = ViewMode::Week;

        assert_eq!(
            shape(&app),
            vec![
                format!("day {day_two}"),
                "group tt/8 2 1h 0m".to_string(),
                format!("day {week_start}"),
                "group tt/8 2 1h 0m".to_string(),
            ]
        );
    }

    /// Expanding inserts the members *after* the header, so the cursor keeps
    /// its index by itself.
    #[test]
    fn toggling_the_group_under_the_cursor_leaves_the_cursor_where_it_is() {
        let _guard = env_guard();
        sandbox("rows-toggle");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
            at(2, "round three", &["tt/8"], today(), 11),
            at(3, "loose", &[], today(), 12),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        // Newest first puts `loose` above the group.
        app.table_state.select(Some(1));
        assert_eq!(app.selectable_len(), 2);

        app.toggle_group_at_cursor();
        assert_eq!(app.table_state.selected(), Some(1));
        assert_eq!(app.selectable_len(), 5);
        assert!(matches!(
            app.selected_row(),
            Some(VisibleRow::GroupHeader(_))
        ));

        app.toggle_group_at_cursor();
        assert_eq!(app.table_state.selected(), Some(1));
        assert_eq!(app.selectable_len(), 2);
    }

    /// `g` from inside a group is the way back out of it, so it collapses the
    /// group and follows the cursor onto the header that replaces the members.
    #[test]
    fn toggling_on_a_member_collapses_its_group_and_lands_on_the_header() {
        let _guard = env_guard();
        sandbox("rows-toggle-member");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
            at(2, "round three", &["tt/8"], today(), 11),
            at(3, "loose", &[], today(), 12),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.expanded_issues.insert("tt/8".to_string());
        // Rows: loose, the header, then the three members newest first.
        app.table_state.select(Some(3));
        assert_eq!(
            app.selected_entry().map(|e| e.description.clone()),
            Some("round two".to_string())
        );

        app.toggle_group_at_cursor();

        assert!(app.expanded_issues.is_empty(), "the group stayed open");
        assert_eq!(app.table_state.selected(), Some(1));
        assert!(matches!(
            app.selected_row(),
            Some(VisibleRow::GroupHeader(_))
        ));
    }

    #[test]
    fn toggling_does_nothing_on_an_entry_row_or_an_empty_list() {
        let _guard = env_guard();
        sandbox("rows-toggle-inert");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
            at(2, "loose", &[], today(), 12),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.table_state.select(Some(0));
        app.toggle_group_at_cursor();
        assert!(app.expanded_issues.is_empty(), "an entry row toggled");

        seed(Vec::new());
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.toggle_group_at_cursor();
        assert!(app.expanded_issues.is_empty());
    }

    #[test]
    fn the_rows_are_rebuilt_when_the_expanded_set_moves() {
        let _guard = env_guard();
        sandbox("rows-cache-key");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;

        assert_eq!(shape(&app), vec!["group tt/8 2 1h 0m"]);
        app.expanded_issues.insert("tt/8".to_string());
        assert_eq!(
            shape(&app),
            vec![
                "group tt/8 2 1h 0m",
                "entry \u{251c} round two",
                "entry \u{2514} round one"
            ]
        );
        app.expanded_issues.remove("tt/8");
        assert_eq!(shape(&app), vec!["group tt/8 2 1h 0m"]);
    }
}
