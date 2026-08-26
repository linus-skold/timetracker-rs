use super::App;
use super::types::{InputField, InputMode, ViewMode};
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate};

/// A submittable form resolved into the fields `add_entry` takes: description,
/// tags, start, and the end an open entry does not have.
type EntryFields = (
    String,
    Vec<String>,
    DateTime<Local>,
    Option<DateTime<Local>>,
);

impl App {
    pub(crate) fn start_adding(&mut self) {
        // In week view, snap selected_date to the day under the cursor.
        if self.view_mode == ViewMode::Week
            && let Some(date) = self.date_under_cursor()
        {
            self.selected_date = date;
        }
        self.input_mode = InputMode::AddingEntry;
        self.input_field = InputField::Description;
        self.input_description.clear();
        self.input_project.clear();
        self.input_tags.clear();
        self.input_start_time.clear();
        self.input_end_time.clear();
        self.input_duration.clear();
        self.cursor_pos = 0;
    }

    pub(crate) fn cancel_adding(&mut self) {
        self.input_mode = InputMode::Normal;
        self.input_description.clear();
        self.input_project.clear();
        self.input_tags.clear();
        self.input_start_time.clear();
        self.input_end_time.clear();
        self.input_duration.clear();
        self.editing_entry_id = None;
        self.cursor_pos = 0;
    }

    pub(crate) fn start_editing(&mut self) {
        let entry_data = {
            let filtered = self.filtered_entries();
            self.table_state.selected().and_then(|idx| {
                filtered.get(idx).map(|entry| {
                    (
                        entry.id,
                        entry.description.clone(),
                        entry.project.clone().unwrap_or_default(),
                        entry.tags.join(" "),
                        entry.start_time.format("%Y-%m-%d %H:%M").to_string(),
                        entry
                            .end_time
                            .map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                        entry.end_time.map(|_| entry.format_duration()),
                    )
                })
            })
        };

        if let Some((id, description, project, tags, start_time, end_time, duration)) = entry_data {
            self.editing_entry_id = Some(id);
            self.input_description = description;
            self.input_project = project;
            self.input_tags = tags;
            self.input_start_time = start_time;
            self.input_end_time = end_time.unwrap_or_default();
            self.input_duration = duration.unwrap_or_default();
            self.input_mode = InputMode::EditingEntry;
            self.input_field = InputField::Description;
            self.cursor_pos = self.input_description.chars().count();
        }
    }

    /// Validates the current form input and resolves it into entry fields.
    /// Returns `None` if the form isn't currently submittable (empty
    /// description, unresolvable start/end times).
    fn build_entry_fields(&self) -> Option<EntryFields> {
        if self.input_description.is_empty() {
            return None;
        }
        let (start_time, end_time) = self.resolve_times()?;
        Some((
            self.input_description.clone(),
            self.parse_tags(),
            start_time,
            end_time,
        ))
    }

    pub(crate) fn submit_entry(&mut self) -> Result<()> {
        let Some((description, tags, start_time, end_time)) = self.build_entry_fields() else {
            return Ok(());
        };
        let project = self.parse_project();
        // Added against the freshly loaded store, so the id is the current `next_id`.
        self.mutate_store(|data| {
            data.add_entry(description, project, tags, start_time, end_time);
        })?;
        self.cancel_adding();
        Ok(())
    }

    pub(crate) fn submit_edit(&mut self) -> Result<()> {
        let Some(entry_id) = self.editing_entry_id else {
            return Ok(());
        };
        let Some((description, tags, start_time, end_time)) = self.build_entry_fields() else {
            return Ok(());
        };
        let project = self.parse_project();
        // Updating an id that is no longer in the store returns false, not an error.
        self.mutate_store(|data| {
            data.update_entry(entry_id, description, project, tags, start_time, end_time)
        })?;
        self.cancel_adding();
        Ok(())
    }

    pub(crate) fn next_input_field(&mut self) {
        let leaving = self.input_field;
        self.apply_time_calculations(leaving);
        self.input_field = match self.input_field {
            InputField::Description => InputField::Project,
            InputField::Project => InputField::Tags,
            InputField::Tags => InputField::Duration,
            InputField::Duration => InputField::StartTime,
            InputField::StartTime => InputField::EndTime,
            InputField::EndTime => InputField::Description,
        };
        self.cursor_pos = self.active_field_input().chars().count();
    }

    pub(crate) fn prev_input_field(&mut self) {
        let leaving = self.input_field;
        self.apply_time_calculations(leaving);
        self.input_field = match self.input_field {
            InputField::Description => InputField::EndTime,
            InputField::Project => InputField::Description,
            InputField::Tags => InputField::Project,
            InputField::Duration => InputField::Tags,
            InputField::StartTime => InputField::Duration,
            InputField::EndTime => InputField::StartTime,
        };
        self.cursor_pos = self.active_field_input().chars().count();
    }

    pub(crate) fn handle_input_char(&mut self, c: char) {
        let pos = self.cursor_pos;
        let s = match self.input_field {
            InputField::Description => &mut self.input_description,
            InputField::Project => &mut self.input_project,
            InputField::Tags => &mut self.input_tags,
            InputField::StartTime => &mut self.input_start_time,
            InputField::EndTime => &mut self.input_end_time,
            InputField::Duration => &mut self.input_duration,
        };
        let byte_idx = s.char_indices().nth(pos).map(|(i, _)| i).unwrap_or(s.len());
        s.insert(byte_idx, c);
        self.cursor_pos += 1;
    }

    pub(crate) fn handle_input_backspace(&mut self) {
        let pos = self.cursor_pos;
        let s = match self.input_field {
            InputField::Description => &mut self.input_description,
            InputField::Project => &mut self.input_project,
            InputField::Tags => &mut self.input_tags,
            InputField::StartTime => &mut self.input_start_time,
            InputField::EndTime => &mut self.input_end_time,
            InputField::Duration => &mut self.input_duration,
        };
        let actual_pos = pos.min(s.chars().count());
        if actual_pos == 0 {
            return;
        }
        let byte_start = s
            .char_indices()
            .nth(actual_pos - 1)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        let byte_end = s
            .char_indices()
            .nth(actual_pos)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        s.drain(byte_start..byte_end);
        self.cursor_pos = actual_pos - 1;
    }

    // ── Cursor movement ───────────────────────────────────────────────────────

    /// Returns the text of the currently active form field (not search).
    pub(crate) fn active_field_input(&self) -> &str {
        match self.input_field {
            InputField::Description => &self.input_description,
            InputField::Project => &self.input_project,
            InputField::Tags => &self.input_tags,
            InputField::StartTime => &self.input_start_time,
            InputField::EndTime => &self.input_end_time,
            InputField::Duration => &self.input_duration,
        }
    }

    /// The active input's text in any mode — form field or search bar.
    fn active_input(&self) -> &str {
        match self.input_mode {
            super::types::InputMode::Searching => &self.search_term,
            _ => self.active_field_input(),
        }
    }

    pub(crate) fn move_cursor_left(&mut self) {
        let clamped = self.cursor_pos.min(self.active_input().chars().count());
        self.cursor_pos = clamped.saturating_sub(1);
    }

    pub(crate) fn move_cursor_right(&mut self) {
        let len = self.active_input().chars().count();
        let clamped = self.cursor_pos.min(len);
        self.cursor_pos = (clamped + 1).min(len);
    }

    /// Jump left past whitespace/punctuation, then past the preceding word.
    pub(crate) fn move_cursor_word_left(&mut self) {
        let input = self.active_input().to_string();
        let chars: Vec<char> = input.chars().collect();
        let mut pos = self.cursor_pos.min(chars.len());
        while pos > 0 && !chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        while pos > 0 && chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        self.cursor_pos = pos;
    }

    /// Jump right past the current word, then past any trailing whitespace/punctuation.
    pub(crate) fn move_cursor_word_right(&mut self) {
        let input = self.active_input().to_string();
        let chars: Vec<char> = input.chars().collect();
        let len = chars.len();
        let mut pos = self.cursor_pos.min(len);
        while pos < len && chars[pos].is_alphanumeric() {
            pos += 1;
        }
        while pos < len && !chars[pos].is_alphanumeric() {
            pos += 1;
        }
        self.cursor_pos = pos;
    }

    // ── Time resolution ──────────────────────────────────────────────────────

    /// Resolve start/end from the three fields, in priority order: Start+Duration,
    /// Start+End, End+Duration, Duration only (ends now), Start only (still active).
    pub(crate) fn resolve_times(&self) -> Option<(DateTime<Local>, Option<DateTime<Local>>)> {
        let start = if !self.input_start_time.is_empty() {
            self.parse_time_str(&self.input_start_time)
        } else {
            None
        };
        let end = if !self.input_end_time.is_empty() {
            self.parse_time_str(&self.input_end_time)
        } else {
            None
        };
        let dur = if !self.input_duration.is_empty() {
            let d = crate::duration::parse(&self.input_duration);
            if d.num_seconds() > 0 { Some(d) } else { None }
        } else {
            None
        };

        match (start, end, dur) {
            (Some(s), _, Some(d)) => Some((s, Some(s + d))),
            (Some(s), Some(e), None) => Some((s, Some(e))),
            (None, Some(e), Some(d)) => Some((e - d, Some(e))),
            (None, None, Some(d)) => {
                // Anchored to selected_date, so a past day's entry lands on that day.
                let now_time = Local::now().time();
                let end = self
                    .selected_date
                    .and_time(now_time)
                    .and_local_timezone(Local)
                    .single()
                    .unwrap_or_else(Local::now);
                Some((end - d, Some(end)))
            }
            (Some(s), None, None) => Some((s, None)),
            _ => None,
        }
    }

    /// Tabbing off Start / End / Duration derives whichever of the other two is still
    /// blank, preferring to adjust the field the user did not just leave.
    pub(crate) fn apply_time_calculations(&mut self, leaving_field: InputField) {
        let start_str = self.input_start_time.clone();
        let end_str = self.input_end_time.clone();
        let dur_str = self.input_duration.clone();

        let start = if !start_str.is_empty() {
            self.parse_time_str(&start_str)
        } else {
            None
        };
        let end = if !end_str.is_empty() {
            self.parse_time_str(&end_str)
        } else {
            None
        };
        let dur = if !dur_str.is_empty() {
            let d = crate::duration::parse(&dur_str);
            if d.num_seconds() > 0 { Some(d) } else { None }
        } else {
            None
        };

        match leaving_field {
            InputField::StartTime => {
                if let (Some(s), Some(d)) = (start, dur) {
                    self.input_end_time = (s + d).format("%Y-%m-%d %H:%M").to_string();
                } else if let (Some(s), Some(e), None) = (start, end, dur) {
                    let diff = e.signed_duration_since(s);
                    if diff.num_seconds() > 0 {
                        self.input_duration = crate::duration::format(diff);
                    }
                }
            }
            InputField::EndTime => {
                if let (Some(_s), Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time = (e - d).format("%Y-%m-%d %H:%M").to_string();
                } else if let (Some(s), Some(e), None) = (start, end, dur) {
                    let diff = e.signed_duration_since(s);
                    if diff.num_seconds() > 0 {
                        self.input_duration = crate::duration::format(diff);
                    }
                } else if let (None, Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time = (e - d).format("%Y-%m-%d %H:%M").to_string();
                }
            }
            InputField::Duration => {
                if let (Some(s), Some(d)) = (start, dur) {
                    self.input_end_time = (s + d).format("%Y-%m-%d %H:%M").to_string();
                } else if let (None, Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time = (e - d).format("%Y-%m-%d %H:%M").to_string();
                }
            }
            _ => {}
        }
    }

    // ── Time parsing ─────────────────────────────────────────────────────────

    pub(crate) fn parse_time_str(&self, input: &str) -> Option<DateTime<Local>> {
        use chrono::Datelike;

        let input = input.trim();
        let current_year = Local::now().year();

        let (naive_date, time_input) = if let Some(space_idx) = input.find(' ') {
            let date_part = &input[..space_idx];
            let time_part = input[space_idx + 1..].trim();
            match Self::parse_date_part(date_part, current_year) {
                Some(d) => (Some(d), time_part),
                None => (Some(self.selected_date), input),
            }
        } else {
            (Some(self.selected_date), input)
        };

        let date = naive_date?;
        let time = Self::parse_time_part(time_input)?;
        date.and_time(time).and_local_timezone(Local).single()
    }

    /// Parse a date-only string. Supported formats: `DD/MM`, `MM-DD`, `YYYY-MM-DD`.
    fn parse_date_part(s: &str, current_year: i32) -> Option<NaiveDate> {
        // DD/MM
        if s.contains('/') {
            let mut parts = s.splitn(2, '/');
            if let (Some(d), Some(m)) = (parts.next(), parts.next())
                && let (Ok(day), Ok(month)) = (d.parse::<u32>(), m.parse::<u32>())
            {
                return NaiveDate::from_ymd_opt(current_year, month, day);
            }
        }
        // YYYY-MM-DD
        if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Some(nd);
        }
        // MM-DD - assumes current year
        if let Ok(nd) = NaiveDate::parse_from_str(&format!("{}-{}", current_year, s), "%Y-%m-%d") {
            return Some(nd);
        }
        None
    }

    /// Parse a time: 24- or 12-hour, `:` or `.` separated, minutes default to `00`.
    fn parse_time_part(s: &str) -> Option<chrono::NaiveTime> {
        use chrono::NaiveTime;

        let s = s.trim().to_lowercase();
        let (is_12h, is_pm, rest) = if s.ends_with("pm") {
            (true, true, s[..s.len() - 2].trim().to_string())
        } else if s.ends_with("am") {
            (true, false, s[..s.len() - 2].trim().to_string())
        } else {
            (false, false, s.clone())
        };

        let rest = rest.replace('.', ":");
        let (hour, minute) = if let Some(colon_pos) = rest.find(':') {
            let h: u32 = rest[..colon_pos].trim().parse().ok()?;
            let m: u32 = rest[colon_pos + 1..].trim().parse().ok()?;
            if m > 59 {
                return None;
            }
            (h, m)
        } else {
            let h: u32 = rest.trim().parse().ok()?;
            (h, 0)
        };

        let hour_24 = if is_12h {
            if hour == 0 || hour > 12 {
                return None;
            }
            match (is_pm, hour) {
                (false, 12) => 0,
                (true, 12) => 12,
                (false, h) => h,
                (true, h) => h + 12,
            }
        } else {
            hour
        };

        NaiveTime::from_hms_opt(hour_24, minute, 0)
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn date_under_cursor(&self) -> Option<chrono::NaiveDate> {
        let filtered = self.filtered_entries();
        self.table_state
            .selected()
            .and_then(|idx| filtered.get(idx))
            .map(|entry| entry.start_time.date_naive())
    }

    /// The project as typed, trimmed; an empty field means "no project".
    fn parse_project(&self) -> Option<String> {
        let project = self.input_project.trim();
        if project.is_empty() {
            None
        } else {
            Some(project.to_string())
        }
    }

    fn parse_tags(&self) -> Vec<String> {
        self.input_tags
            .split_whitespace()
            .map(|s| s.trim_start_matches('#').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}
