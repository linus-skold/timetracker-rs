use super::App;
use super::types::{InputMode, SortOrder};
use chrono::Duration;

impl App {
    pub(crate) fn filtered_entries(&self) -> Vec<&crate::tracker::TimeEntry> {
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

        if self.search_term.is_empty() {
            entries
        } else {
            let search_lower = self.search_term.value().to_lowercase();
            entries
                .into_iter()
                .filter(|e| e.matches_search(&search_lower))
                .collect()
        }
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
