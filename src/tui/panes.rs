//! The collapsible Projects / Tags pane surface. Each pane offers the distinct
//! values of the current view scope *before* any filter, with their match counts.

use std::collections::HashMap;

use super::App;
use super::types::{Focus, Pane};
use crate::tracker::TimeEntry;

/// Most value rows a pane shows before it starts scrolling.
const MAX_VISIBLE_VALUES: usize = 6;

/// The number a collapsible surface puts on its top border, or `None` when it has
/// nothing to say. With a cursor (`position` is `Some`) it is about *place*: hidden
/// while every row fits, `position/total` once the list scrolls. Without one it is
/// about *existence*: a bare `total` while all fit, `shown/total` once they do not.
pub(crate) fn surface_count(
    position: Option<usize>,
    total: usize,
    visible_rows: usize,
) -> Option<String> {
    match (position, total <= visible_rows) {
        (_, true) if total == 0 => None,
        (Some(_), true) => None,
        (Some(position), false) => Some(format!("{}/{}", position + 1, total)),
        (None, true) => Some(total.to_string()),
        (None, false) => Some(format!("{}/{}", visible_rows, total)),
    }
}

impl App {
    /// The current view scope, before any filter. `filtered_entries` starts here.
    pub(crate) fn scope_entries(&self) -> Vec<&TimeEntry> {
        use super::types::ViewMode;
        use crate::tracker::TimeData;
        match self.view_mode {
            ViewMode::All => self.data.entries.iter().collect(),
            ViewMode::Day => self.data.entries_for_date(self.selected_date),
            ViewMode::Week => {
                let week_start = TimeData::week_start(self.selected_date);
                self.data.entries_for_week(week_start)
            }
        }
    }

    pub(crate) fn pane_is_visible(&self, pane: Pane) -> bool {
        match pane {
            Pane::Projects => self.show_projects,
            Pane::Tags => self.show_tags,
        }
    }

    pub(crate) fn visible_panes(&self) -> Vec<Pane> {
        [Pane::Projects, Pane::Tags]
            .into_iter()
            .filter(|pane| self.pane_is_visible(*pane))
            .collect()
    }

    /// Distinct values with match counts, most used first, ties broken by name. No
    /// project contributes no row; a tag repeated within one entry counts once.
    pub(crate) fn pane_values(&self, pane: Pane) -> Vec<(String, usize)> {
        let entries = self.scope_entries();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entry in &entries {
            match pane {
                Pane::Projects => {
                    let project = entry.project.as_deref().map(str::trim).unwrap_or("");
                    if !project.is_empty() {
                        *counts.entry(project).or_default() += 1;
                    }
                }
                Pane::Tags => {
                    let mut seen: Vec<&str> = Vec::new();
                    for tag in &entry.tags {
                        let tag = tag.trim();
                        if tag.is_empty() || seen.contains(&tag) {
                            continue;
                        }
                        seen.push(tag);
                        *counts.entry(tag).or_default() += 1;
                    }
                }
            }
        }

        let mut values: Vec<(String, usize)> = counts
            .into_iter()
            .map(|(value, count)| (value.to_string(), count))
            .collect();
        values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        values
    }

    /// Height including borders, or 0 when both are hidden so the layout drops it.
    pub(crate) fn pane_surface_height(&self) -> u16 {
        let panes = self.visible_panes();
        if panes.is_empty() {
            return 0;
        }
        let longest = panes
            .iter()
            .map(|pane| self.pane_values(*pane).len())
            .max()
            .unwrap_or(0);
        2 + longest.clamp(1, MAX_VISIBLE_VALUES) as u16
    }

    pub(crate) fn pane_cursor(&self, pane: Pane) -> usize {
        let cursor = match pane {
            Pane::Projects => self.project_cursor,
            Pane::Tags => self.tag_cursor,
        };
        let len = self.pane_values(pane).len();
        cursor.min(len.saturating_sub(1))
    }

    /// `position/total` once the values do not all fit in `visible_rows`.
    pub(crate) fn pane_scroll_indicator(&self, pane: Pane, visible_rows: usize) -> Option<String> {
        surface_count(
            Some(self.pane_cursor(pane)),
            self.pane_values(pane).len(),
            visible_rows,
        )
    }

    pub(crate) fn focused_pane(&self) -> Option<Pane> {
        match self.focus {
            Focus::Pane(pane) if self.pane_is_visible(pane) => Some(pane),
            _ => None,
        }
    }

    /// Show or hide a pane. Opening one focuses it; hiding the focused one falls
    /// back to the other pane, or to the table.
    pub(crate) fn toggle_pane(&mut self, pane: Pane) {
        match pane {
            Pane::Projects => self.show_projects = !self.show_projects,
            Pane::Tags => self.show_tags = !self.show_tags,
        }
        if self.pane_is_visible(pane) {
            self.focus = Focus::Pane(pane);
        } else if self.focus == Focus::Pane(pane) {
            self.focus = self
                .visible_panes()
                .first()
                .copied()
                .map_or(Focus::Table, Focus::Pane);
        }
    }

    /// `Tab`: entries table → each visible pane, left to right → back.
    pub(crate) fn cycle_focus(&mut self) {
        self.shift_focus(1);
    }

    /// `Shift-Tab`: the exact inverse of [`cycle_focus`](Self::cycle_focus).
    pub(crate) fn cycle_focus_back(&mut self) {
        self.shift_focus(-1);
    }

    /// Walk the ring — table then visible panes, left to right — by `delta`, wrapping.
    /// A focus off the ring reads as the table, so both directions recover.
    fn shift_focus(&mut self, delta: isize) {
        let mut order = vec![Focus::Table];
        order.extend(self.visible_panes().into_iter().map(Focus::Pane));
        let current = order.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let len = order.len() as isize;
        self.focus = order[(current + delta).rem_euclid(len) as usize];
    }

    pub(crate) fn pane_selection(&self, pane: Pane) -> &[String] {
        match pane {
            Pane::Projects => &self.selected_projects,
            Pane::Tags => &self.selected_tags,
        }
    }

    /// Exact match: both sides come from [`pane_values`](Self::pane_values), though
    /// the filter predicates themselves stay case-insensitive.
    pub(crate) fn pane_value_is_selected(&self, pane: Pane, value: &str) -> bool {
        self.pane_selection(pane).iter().any(|v| v == value)
    }

    /// `Enter` in the focused pane: toggle the value under its cursor. Returns false
    /// when no pane has focus, so `Enter` falls through to the detail popover.
    pub(crate) fn toggle_pane_value(&mut self) -> bool {
        let Some(pane) = self.focused_pane() else {
            return false;
        };
        let cursor = self.pane_cursor(pane);
        let Some((value, _)) = self.pane_values(pane).into_iter().nth(cursor) else {
            return true;
        };
        let selection = match pane {
            Pane::Projects => &mut self.selected_projects,
            Pane::Tags => &mut self.selected_tags,
        };
        match selection.iter().position(|v| v == &value) {
            Some(pos) => {
                selection.remove(pos);
            }
            None => selection.push(value),
        }
        // The row that was selected is very unlikely to still be the same row.
        self.table_state.select(Some(0));
        true
    }

    /// `j` in the focused pane, or false so the caller moves the table instead.
    pub(crate) fn pane_next(&mut self) -> bool {
        self.move_pane_cursor(1)
    }

    pub(crate) fn pane_previous(&mut self) -> bool {
        self.move_pane_cursor(-1)
    }

    fn move_pane_cursor(&mut self, delta: isize) -> bool {
        let Some(pane) = self.focused_pane() else {
            return false;
        };
        let len = self.pane_values(pane).len();
        if len == 0 {
            return true;
        }
        let current = self.pane_cursor(pane) as isize;
        let next = (current + delta).rem_euclid(len as isize) as usize;
        match pane {
            Pane::Projects => self.project_cursor = next,
            Pane::Tags => self.tag_cursor = next,
        }
        true
    }
}
