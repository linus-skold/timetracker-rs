//! Keys for the derived-view caches.
//!
//! [`App::filtered_entries`](super::App::filtered_entries) and
//! [`App::pane_values`](super::App::pane_values) are recomputed only when the
//! state they read has actually moved. A cached value is reused while the *key*
//! it was computed under still compares equal, and that key holds **every**
//! input the view reads — so validity is decided by construction rather than by
//! remembering to call an `invalidate()` at each mutation site. A new mutator,
//! or a test writing `app.view_mode` directly, is covered for free.
//!
//! Adding an input to either derived view means adding it here too; forgetting
//! is the one way to get a stale cache.

use chrono::NaiveDate;

use super::App;
use super::panes::PaneFilter;
use super::types::{SortOrder, ViewMode};

/// Every input to [`App::scope_entries`](super::App::scope_entries), and so to
/// [`App::pane_values`](super::App::pane_values), which reads no filters.
#[derive(Clone, PartialEq)]
pub(crate) struct ScopeKey {
    /// Stands in for `data` itself, which is far too big to compare per call.
    /// See [`App::set_data`](super::App::set_data).
    revision: u64,
    view_mode: ViewMode,
    selected_date: NaiveDate,
}

impl ScopeKey {
    pub(crate) fn of(app: &App) -> Self {
        Self {
            revision: app.data_revision,
            view_mode: app.view_mode,
            selected_date: app.selected_date,
        }
    }
}

/// Every input to [`App::filtered_entries`](super::App::filtered_entries): the
/// scope, plus the sort order and the two pane filters and the search term.
#[derive(Clone, PartialEq)]
pub(crate) struct FilterKey {
    scope: ScopeKey,
    sort_order: SortOrder,
    project_filter: PaneFilter,
    tag_filter: PaneFilter,
    search: String,
}

impl FilterKey {
    pub(crate) fn of(app: &App) -> Self {
        Self {
            scope: ScopeKey::of(app),
            sort_order: app.sort_order,
            project_filter: app.project_filter.clone(),
            tag_filter: app.tag_filter.clone(),
            // Only the text matters; the cursor does not filter anything.
            search: app.search_term.value().to_string(),
        }
    }
}
