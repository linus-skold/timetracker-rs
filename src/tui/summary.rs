//! Per-project totals for the current view scope, and the state behind the
//! collapsible `Summary` surface. Display-only; `render.rs` draws it.
//!
//! **This folds the scope, not the view:** [`App::project_summary`] reads
//! `scope_entries()`, never `filtered_entries()`, so a filter leaves it alone.

use std::collections::HashMap;

use chrono::Duration;

use super::App;
use super::panes::surface_count;

/// Most project rows the surface shows before the rest live in the border count.
const MAX_VISIBLE_PROJECTS: usize = 6;

/// The standing statement on the title bar, after the scope word.
const ALL_PROJECTS: &str = "all projects";

/// The label for entries with no project; counted so the rows sum to the scope.
pub(crate) const NO_PROJECT: &str = "(no project)";

/// One row: a project, its time in the scope, its entry count and its share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectTotal {
    /// The name as stored, or [`NO_PROJECT`].
    pub(crate) project: String,
    /// Raw; the caller applies `duration::format`.
    pub(crate) total: Duration,
    pub(crate) entries: usize,
    /// Percent of the scope total, rounded, and **not** fudged to sum to 100.
    pub(crate) share: u16,
}

impl App {
    /// Per-project totals for the current view scope, largest first. Folds
    /// [`scope_entries`](Self::scope_entries), so it is about the period rather than
    /// the rows on screen. An empty scope gives an empty list.
    pub(crate) fn project_summary(&self) -> Vec<ProjectTotal> {
        let entries = self.scope_entries();
        let mut totals: HashMap<&str, (Duration, usize)> = HashMap::new();
        for entry in &entries {
            // Empty-after-trim counts as absent, as the form and `pane_values` do.
            let project = entry.project.as_deref().map(str::trim).unwrap_or("");
            let key = if project.is_empty() {
                NO_PROJECT
            } else {
                project
            };
            let row = totals.entry(key).or_insert((Duration::zero(), 0));
            row.0 += entry.duration();
            row.1 += 1;
        }

        let scope_total: i64 = totals.values().map(|(d, _)| d.num_seconds()).sum();
        let mut rows: Vec<ProjectTotal> = totals
            .into_iter()
            .map(|(project, (total, entries))| ProjectTotal {
                project: project.to_string(),
                total,
                entries,
                share: share_of(total, scope_total),
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

    /// Height including borders, or 0 while hidden so the layout drops the row.
    pub(crate) fn summary_surface_height(&self) -> u16 {
        if !self.show_summary {
            return 0;
        }
        2 + self.project_summary().len().clamp(1, MAX_VISIBLE_PROJECTS) as u16
    }

    pub(crate) fn visible_project_summary(&self, visible_rows: usize) -> Vec<ProjectTotal> {
        let mut rows = self.project_summary();
        rows.truncate(visible_rows);
        rows
    }

    /// `shown/total` once more projects exist than fit, else `None`.
    pub(crate) fn summary_count(&self, visible_rows: usize) -> Option<String> {
        let total = self.project_summary().len();
        if total <= visible_rows {
            return None;
        }
        surface_count(None, total, visible_rows)
    }

    /// The title bar's right half: `day · all projects`, plus `· 6/9` when rows are
    /// off screen. Present in **both** filter states — only its colour changes.
    pub(crate) fn summary_marker(&self, visible_rows: usize) -> String {
        let mut marker = format!("{} · {}", self.view_mode.label(), ALL_PROJECTS);
        if let Some(count) = self.summary_count(visible_rows) {
            marker.push_str(" · ");
            marker.push_str(&count);
        }
        marker
    }

    /// `Shift-S`: show or hide the surface.
    pub(crate) fn toggle_summary(&mut self) {
        self.show_summary = !self.show_summary;
    }
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
