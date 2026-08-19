use chrono::Duration;
use super::App;
use super::types::{InputMode, SortOrder};

impl App {
    pub(crate) fn filtered_entries(&self) -> Vec<&crate::tracker::TimeEntry> {
        let mut entries = self.scope_entries();

        match self.sort_order {
            SortOrder::NewestFirst => entries.sort_by(|a, b| b.start_time.cmp(&a.start_time)),
            SortOrder::OldestFirst => entries.sort_by(|a, b| a.start_time.cmp(&b.start_time)),
        }

        // OR within a pane, AND across the two; an empty set is not a filter.
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|e| {
                self.selected_projects.is_empty() || e.has_any_project(&self.selected_projects)
            })
            .filter(|e| self.selected_tags.is_empty() || e.has_any_tag(&self.selected_tags))
            .collect();

        if self.search_term.is_empty() {
            entries
        } else {
            let search_lower = self.search_term.to_lowercase();
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
        !self.selected_projects.is_empty() || !self.selected_tags.is_empty()
    }

    /// Whether the footer's figure is narrowed — pane selection or search term.
    /// The Summary marker reads the same predicate, so the two cannot disagree.
    pub(crate) fn total_is_filtered(&self) -> bool {
        !self.search_term.is_empty() || self.is_filtering()
    }

    pub(crate) fn clear_filters(&mut self) {
        self.selected_projects.clear();
        self.selected_tags.clear();
        self.table_state.select(Some(0));
    }

    pub(crate) fn start_search(&mut self) {
        self.input_mode = InputMode::Searching;
        self.cursor_pos = self.search_term.chars().count();
    }

    pub(crate) fn clear_search(&mut self) {
        self.search_term.clear();
        self.input_mode = InputMode::Normal;
        self.cursor_pos = 0;
        self.table_state.select(Some(0));
    }

    pub(crate) fn handle_search_char(&mut self, c: char) {
        let pos = self.cursor_pos;
        let byte_idx = self.search_term.char_indices().nth(pos).map(|(i, _)| i).unwrap_or(self.search_term.len());
        self.search_term.insert(byte_idx, c);
        self.cursor_pos += 1;
        self.table_state.select(Some(0));
    }

    pub(crate) fn handle_search_backspace(&mut self) {
        let pos = self.cursor_pos;
        let actual_pos = pos.min(self.search_term.chars().count());
        if actual_pos == 0 { return; }
        let byte_start = self.search_term.char_indices().nth(actual_pos - 1).map(|(i, _)| i).unwrap_or(self.search_term.len());
        let byte_end = self.search_term.char_indices().nth(actual_pos).map(|(i, _)| i).unwrap_or(self.search_term.len());
        self.search_term.drain(byte_start..byte_end);
        self.cursor_pos = actual_pos - 1;
        self.table_state.select(Some(0));
    }

    pub(crate) fn confirm_search(&mut self) {
        self.input_mode = InputMode::Normal;
    }
}
