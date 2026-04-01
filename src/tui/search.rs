use chrono::Duration;
use crate::tracker::TimeData;
use super::App;
use super::types::{InputMode, SortOrder, ViewMode};

impl App {
    pub(crate) fn filtered_entries(&self) -> Vec<&crate::tracker::TimeEntry> {
        let mut entries: Vec<_> = match self.view_mode {
            ViewMode::All => self.data.entries.iter().collect(),
            ViewMode::Day => self.data.entries_for_date(self.selected_date),
            ViewMode::Week => {
                let week_start = TimeData::week_start(self.selected_date);
                self.data.entries_for_week(week_start)
            }
        };

        match self.sort_order {
            SortOrder::NewestFirst => entries.sort_by(|a, b| b.start_time.cmp(&a.start_time)),
            SortOrder::OldestFirst => entries.sort_by(|a, b| a.start_time.cmp(&b.start_time)),
        }

        let entries = if self.tag_filter.is_empty() {
            entries
        } else {
            entries
                .into_iter()
                .filter(|e| e.has_any_tag(&self.tag_filter))
                .collect()
        };

        if self.search_term.is_empty() {
            entries
        } else {
            let search_lower = self.search_term.to_lowercase();
            entries
                .into_iter()
                .filter(|e| {
                    e.description.to_lowercase().contains(&search_lower)
                        || e.tags.iter().any(|t| t.to_lowercase().contains(&search_lower))
                })
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

    pub(crate) fn is_tag_filtering(&self) -> bool {
        !self.tag_filter.is_empty()
    }

    pub(crate) fn toggle_tag_filter(&mut self, tag: &str) {
        let tag = tag.to_string();
        if let Some(pos) = self.tag_filter.iter().position(|t| t == &tag) {
            self.tag_filter.remove(pos);
        } else {
            self.tag_filter.push(tag);
        }
        self.table_state.select(Some(0));
    }

    pub(crate) fn clear_tag_filter(&mut self) {
        self.tag_filter.clear();
        self.table_state.select(Some(0));
    }

    pub(crate) fn filter_by_selected_tags(&mut self) {
        let tags = {
            let filtered = self.filtered_entries();
            self.table_state.selected().and_then(|idx| {
                filtered.get(idx).map(|entry| entry.tags.clone())
            })
        };

        if let Some(tags) = tags {
            if tags.is_empty() {
                return;
            }
            if self.tag_filter == tags {
                self.clear_tag_filter();
            } else {
                self.tag_filter = tags;
                self.table_state.select(Some(0));
            }
        }
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
