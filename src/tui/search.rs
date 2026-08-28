use super::App;
use super::cache::FilterKey;
use super::types::{InputMode, SortOrder};
use chrono::Duration;
use std::collections::HashMap;

impl App {
    /// The rows the entries table shows, in sort order. Cached against
    /// [`FilterKey`], so the many calls a single frame makes cost one walk.
    pub(crate) fn filtered_entries(&self) -> Vec<&crate::tracker::TimeEntry> {
        self.with_filtered_indices(|indices| {
            indices
                .iter()
                .filter_map(|i| self.data.entries.get(*i))
                .collect()
        })
    }

    /// How many rows [`filtered_entries`](Self::filtered_entries) would return,
    /// without materialising them.
    pub(crate) fn filtered_len(&self) -> usize {
        self.with_filtered_indices(<[usize]>::len)
    }

    /// Run `f` over the cached row indices, recomputing them first if any input
    /// the [`FilterKey`] covers has moved since they were built.
    fn with_filtered_indices<T>(&self, f: impl FnOnce(&[usize]) -> T) -> T {
        let key = FilterKey::of(self);
        let stale = match &*self.filtered_cache.borrow() {
            Some((cached, _)) => *cached != key,
            None => true,
        };
        if stale {
            let indices = self.compute_filtered_indices();
            *self.filtered_cache.borrow_mut() = Some((key, indices));
        }
        let cache = self.filtered_cache.borrow();
        let (_, indices) = cache.as_ref().expect("filled just above");
        f(indices)
    }

    /// The uncached walk — scope, sort, then the filters — resolved to indices
    /// into `data.entries` so the cache holds no borrow of `data`.
    fn compute_filtered_indices(&self) -> Vec<usize> {
        let mut entries = self.scope_entries();

        match self.sort_order {
            SortOrder::NewestFirst => entries.sort_by_key(|e| std::cmp::Reverse(e.start_time)),
            SortOrder::OldestFirst => entries.sort_by_key(|e| e.start_time),
        }

        // OR within a pane's includes, AND across the two; exclusions veto first.
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|e| self.project_filter.allows(|v| e.is_project(v)))
            .filter(|e| self.tag_filter.allows(|v| e.has_tag(v)))
            .collect();

        let entries = if self.search_term.is_empty() {
            entries
        } else {
            let search_lower = self.search_term.value().to_lowercase();
            entries
                .into_iter()
                .filter(|e| e.matches_search(&search_lower))
                .collect()
        };

        let position: HashMap<u64, usize> = self
            .data
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, i))
            .collect();
        entries
            .iter()
            .filter_map(|e| position.get(&e.id).copied())
            .collect()
    }

    pub(crate) fn filtered_total(&self) -> Duration {
        self.filtered_entries()
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    pub(crate) fn is_searching(&self) -> bool {
        !self.search_term.is_empty() || self.input_mode == InputMode::Searching
    }

    /// Whether a pane selection is narrowing the view.
    pub(crate) fn is_filtering(&self) -> bool {
        !self.project_filter.is_empty() || !self.tag_filter.is_empty()
    }

    /// Whether the footer's figure is narrowed — pane selection or search term.
    /// The Summary marker reads the same predicate, so the two cannot disagree.
    pub(crate) fn total_is_filtered(&self) -> bool {
        !self.search_term.is_empty() || self.is_filtering()
    }

    pub(crate) fn clear_filters(&mut self) {
        self.project_filter.clear();
        self.tag_filter.clear();
        self.table_state.select(Some(0));
    }

    pub(crate) fn start_search(&mut self) {
        self.input_mode = InputMode::Searching;
        self.search_term.cursor_to_end();
    }

    pub(crate) fn clear_search(&mut self) {
        self.search_term.clear();
        self.input_mode = InputMode::Normal;
        self.table_state.select(Some(0));
    }

    pub(crate) fn handle_search_char(&mut self, c: char) {
        self.search_term.insert(c);
        self.table_state.select(Some(0));
    }

    pub(crate) fn handle_search_backspace(&mut self) {
        self.search_term.backspace();
        self.table_state.select(Some(0));
    }

    pub(crate) fn confirm_search(&mut self) {
        self.input_mode = InputMode::Normal;
    }
}
