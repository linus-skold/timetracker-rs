use super::App;
use super::text_input::TextInput;
use super::types::{InputField, InputMode, ViewMode};
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate};

/// A submittable form resolved into what the store takes: description, tags,
/// start, the end an open entry does not have, and the custom JSON.
struct EntryFields {
    description: String,
    tags: Vec<String>,
    start_time: DateTime<Local>,
    end_time: Option<DateTime<Local>>,
    /// `None` for a blank Data field, which clears the entry's data on an edit.
    data: Option<serde_json::Value>,
}

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
        self.clear_form_fields();
    }

    pub(crate) fn cancel_adding(&mut self) {
        self.input_mode = InputMode::Normal;
        self.clear_form_fields();
        self.editing_entry_id = None;
    }

    fn clear_form_fields(&mut self) {
        self.input_description.clear();
        self.input_project.clear();
        self.input_tags.clear();
        self.input_start_time.clear();
        self.input_end_time.clear();
        self.input_duration.clear();
        self.input_data.clear();
        self.form_error = None;
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
                        crate::entry_data::to_edit_string(entry.data.as_ref()),
                    )
                })
            })
        };

        if let Some((id, description, project, tags, start_time, end_time, duration, data)) =
            entry_data
        {
            self.editing_entry_id = Some(id);
            self.input_data.set_from(&data);
            self.form_error = None;
            self.input_description.set_from(&description);
            self.input_project.set_from(&project);
            self.input_tags.set_from(&tags);
            self.input_start_time.set_from(&start_time);
            self.input_end_time.set_from(&end_time.unwrap_or_default());
            self.input_duration.set_from(&duration.unwrap_or_default());
            self.input_mode = InputMode::EditingEntry;
            self.input_field = InputField::Description;
        }
    }

    /// Validates the current form input and resolves it into entry fields.
    /// `Ok(None)` is "not submittable yet" — an empty description or
    /// unresolvable start/end times, both of which the user is still typing
    /// their way out of. `Err` is a value that will never resolve however much
    /// more is typed: today only invalid JSON in the Data field.
    fn build_entry_fields(&self) -> Result<Option<EntryFields>, String> {
        if self.input_description.is_empty() {
            return Ok(None);
        }
        // Parsed before the times, so a bad Data field is reported even while the
        // times are still half-typed.
        let data = crate::entry_data::parse(self.input_data.value())?;
        let Some((start_time, end_time)) = self.resolve_times() else {
            return Ok(None);
        };
        Ok(Some(EntryFields {
            description: self.input_description.value().to_string(),
            tags: self.parse_tags(),
            start_time,
            end_time,
            data,
        }))
    }

    /// The fields to save, or `None` when there is nothing to save yet. A
    /// rejected value is left on screen with `form_error` set, so nothing is
    /// written and the form stays open on what the user typed.
    fn fields_to_save(&mut self) -> Option<EntryFields> {
        match self.build_entry_fields() {
            Ok(fields) => fields,
            Err(message) => {
                self.form_error = Some(message);
                None
            }
        }
    }

    pub(crate) fn submit_entry(&mut self) -> Result<()> {
        let Some(fields) = self.fields_to_save() else {
            return Ok(());
        };
        let EntryFields {
            description,
            tags,
            start_time,
            end_time,
            data,
        } = fields;
        let project = self.parse_project();
        // Added against the freshly loaded store, so the id is the current `next_id`.
        self.mutate_store(|store| {
            let id = store
                .add_entry(description, project, tags, start_time, end_time)
                .id;
            store.set_entry_data(id, data);
        })?;
        self.cancel_adding();
        Ok(())
    }

    pub(crate) fn submit_edit(&mut self) -> Result<()> {
        let Some(entry_id) = self.editing_entry_id else {
            return Ok(());
        };
        let Some(fields) = self.fields_to_save() else {
            return Ok(());
        };
        let EntryFields {
            description,
            tags,
            start_time,
            end_time,
            data,
        } = fields;
        let project = self.parse_project();
        // Updating an id that is no longer in the store returns false, not an error.
        self.mutate_store(|store| {
            // A blank Data field clears the entry's data, so an edit can remove it.
            store.set_entry_data(entry_id, data);
            store.update_entry(entry_id, description, project, tags, start_time, end_time)
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
            InputField::EndTime => InputField::Data,
            InputField::Data => InputField::Description,
        };
        self.field_mut().cursor_to_end();
    }

    pub(crate) fn prev_input_field(&mut self) {
        let leaving = self.input_field;
        self.apply_time_calculations(leaving);
        self.input_field = match self.input_field {
            InputField::Description => InputField::Data,
            InputField::Project => InputField::Description,
            InputField::Tags => InputField::Project,
            InputField::Duration => InputField::Tags,
            InputField::StartTime => InputField::Duration,
            InputField::EndTime => InputField::StartTime,
            InputField::Data => InputField::EndTime,
        };
        self.field_mut().cursor_to_end();
    }

    pub(crate) fn handle_input_char(&mut self, c: char) {
        // Typing is the fix for whatever the last save was refused over.
        self.form_error = None;
        self.field_mut().insert(c);
    }

    pub(crate) fn handle_input_backspace(&mut self) {
        self.form_error = None;
        self.field_mut().backspace();
    }

    // ── Cursor movement ───────────────────────────────────────────────────────

    /// The one place the `InputField` -> field mapping lives.
    pub(crate) fn field_mut(&mut self) -> &mut TextInput {
        match self.input_field {
            InputField::Description => &mut self.input_description,
            InputField::Project => &mut self.input_project,
            InputField::Tags => &mut self.input_tags,
            InputField::StartTime => &mut self.input_start_time,
            InputField::EndTime => &mut self.input_end_time,
            InputField::Duration => &mut self.input_duration,
            InputField::Data => &mut self.input_data,
        }
    }

    /// The active input in any mode — form field or search bar.
    pub(crate) fn active_input(&mut self) -> &mut TextInput {
        match self.input_mode {
            InputMode::Searching => &mut self.search_term,
            _ => self.field_mut(),
        }
    }

    pub(crate) fn move_cursor_left(&mut self) {
        self.active_input().left();
    }

    pub(crate) fn move_cursor_right(&mut self) {
        self.active_input().right();
    }

    pub(crate) fn move_cursor_word_left(&mut self) {
        self.active_input().word_left();
    }

    pub(crate) fn move_cursor_word_right(&mut self) {
        self.active_input().word_right();
    }

    // ── Time resolution ──────────────────────────────────────────────────────

    /// Resolve start/end from the three fields, in priority order: Start+Duration,
    /// Start+End, End+Duration, Duration only (ends now), Start only (still active).
    pub(crate) fn resolve_times(&self) -> Option<(DateTime<Local>, Option<DateTime<Local>>)> {
        let start = if !self.input_start_time.is_empty() {
            self.parse_time_str(self.input_start_time.value())
        } else {
            None
        };
        let end = if !self.input_end_time.is_empty() {
            self.parse_time_str(self.input_end_time.value())
        } else {
            None
        };
        // A half-typed or unparseable duration is "not yet resolvable", the same
        // as a zero-length one — never an error while the user is still typing.
        let dur =
            crate::duration::parse(self.input_duration.value()).filter(|d| d.num_seconds() > 0);

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
        let start_str = self.input_start_time.value().to_string();
        let end_str = self.input_end_time.value().to_string();
        let dur_str = self.input_duration.value().to_string();

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
        let dur = crate::duration::parse(&dur_str).filter(|d| d.num_seconds() > 0);

        match leaving_field {
            InputField::StartTime => {
                if let (Some(s), Some(d)) = (start, dur) {
                    self.input_end_time
                        .set_from(&(s + d).format("%Y-%m-%d %H:%M").to_string());
                } else if let (Some(s), Some(e), None) = (start, end, dur) {
                    let diff = e.signed_duration_since(s);
                    if diff.num_seconds() > 0 {
                        self.input_duration.set_from(&crate::duration::format(diff));
                    }
                }
            }
            InputField::EndTime => {
                if let (Some(_s), Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time
                        .set_from(&(e - d).format("%Y-%m-%d %H:%M").to_string());
                } else if let (Some(s), Some(e), None) = (start, end, dur) {
                    let diff = e.signed_duration_since(s);
                    if diff.num_seconds() > 0 {
                        self.input_duration.set_from(&crate::duration::format(diff));
                    }
                } else if let (None, Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time
                        .set_from(&(e - d).format("%Y-%m-%d %H:%M").to_string());
                }
            }
            InputField::Duration => {
                if let (Some(s), Some(d)) = (start, dur) {
                    self.input_end_time
                        .set_from(&(s + d).format("%Y-%m-%d %H:%M").to_string());
                } else if let (None, Some(e), Some(d)) = (start, end, dur) {
                    self.input_start_time
                        .set_from(&(e - d).format("%Y-%m-%d %H:%M").to_string());
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
        let project = self.input_project.value().trim();
        if project.is_empty() {
            None
        } else {
            Some(project.to_string())
        }
    }

    fn parse_tags(&self) -> Vec<String> {
        self.input_tags
            .value()
            .split_whitespace()
            .map(|s| s.trim_start_matches('#').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}
