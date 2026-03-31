use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate};
use crate::storage::save_data;
use super::App;
use super::types::{InputField, InputMode};

impl App {
    pub(crate) fn start_adding(&mut self) {
        self.input_mode = InputMode::AddingEntry;
        self.input_field = InputField::Description;
        self.input_description.clear();
        self.input_tags.clear();
        self.input_start_time.clear();
        self.input_end_time.clear();
        self.input_duration.clear();
    }

    pub(crate) fn cancel_adding(&mut self) {
        self.input_mode = InputMode::Normal;
        self.input_description.clear();
        self.input_tags.clear();
        self.input_start_time.clear();
        self.input_end_time.clear();
        self.input_duration.clear();
        self.editing_entry_id = None;
    }

    pub(crate) fn start_editing(&mut self) {
        let entry_data = {
            let filtered = self.filtered_entries();
            self.table_state.selected().and_then(|idx| {
                filtered.get(idx).map(|entry| {
                    (
                        entry.id,
                        entry.description.clone(),
                        entry.tags.join(" "),
                        entry.start_time.format("%Y-%m-%d %H:%M").to_string(),
                        entry.end_time.map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                        entry.end_time.map(|_| entry.format_duration()),
                    )
                })
            })
        };

        if let Some((id, description, tags, start_time, end_time, duration)) = entry_data {
            self.editing_entry_id = Some(id);
            self.input_description = description;
            self.input_tags = tags;
            self.input_start_time = start_time;
            self.input_end_time = end_time.unwrap_or_default();
            self.input_duration = duration.unwrap_or_default();
            self.input_mode = InputMode::EditingEntry;
            self.input_field = InputField::Description;
        }
    }

    pub(crate) fn submit_entry(&mut self) -> Result<()> {
        if self.input_description.is_empty() {
            return Ok(());
        }
        let Some((start_time, end_time)) = self.resolve_times() else {
            return Ok(());
        };
        let tags = self.parse_tags();
        self.data.add_entry(self.input_description.clone(), tags, start_time, end_time);
        save_data(&self.data)?;
        self.cancel_adding();
        Ok(())
    }

    pub(crate) fn submit_edit(&mut self) -> Result<()> {
        let entry_id = match self.editing_entry_id {
            Some(id) => id,
            None => return Ok(()),
        };
        if self.input_description.is_empty() {
            return Ok(());
        }
        let Some((start_time, end_time)) = self.resolve_times() else {
            return Ok(());
        };
        let tags = self.parse_tags();
        self.data.update_entry(entry_id, self.input_description.clone(), tags, start_time, end_time);
        save_data(&self.data)?;
        self.cancel_adding();
        Ok(())
    }

    pub(crate) fn next_input_field(&mut self) {
        let leaving = self.input_field;
        self.apply_time_calculations(leaving);
        self.input_field = match self.input_field {
            InputField::Description => InputField::Tags,
            InputField::Tags => InputField::Duration,
            InputField::Duration => InputField::StartTime,
            InputField::StartTime => InputField::EndTime,
            InputField::EndTime => InputField::Description,
        };
    }

    pub(crate) fn prev_input_field(&mut self) {
        let leaving = self.input_field;
        self.apply_time_calculations(leaving);
        self.input_field = match self.input_field {
            InputField::Description => InputField::EndTime,
            InputField::Tags => InputField::Description,
            InputField::Duration => InputField::Tags,
            InputField::StartTime => InputField::Duration,
            InputField::EndTime => InputField::StartTime,
        };
    }

    pub(crate) fn handle_input_char(&mut self, c: char) {
        match self.input_field {
            InputField::Description => self.input_description.push(c),
            InputField::Tags => self.input_tags.push(c),
            InputField::StartTime => self.input_start_time.push(c),
            InputField::EndTime => self.input_end_time.push(c),
            InputField::Duration => self.input_duration.push(c),
        }
    }

    pub(crate) fn handle_input_backspace(&mut self) {
        match self.input_field {
            InputField::Description => { self.input_description.pop(); }
            InputField::Tags => { self.input_tags.pop(); }
            InputField::StartTime => { self.input_start_time.pop(); }
            InputField::EndTime => { self.input_end_time.pop(); }
            InputField::Duration => { self.input_duration.pop(); }
        }
    }

    // ── Time resolution ──────────────────────────────────────────────────────

    /// Resolve start/end times from the three input fields. Priority:
    /// - Start + Duration → end = start + duration
    /// - Start + End      → save both as-is
    /// - End + Duration   → start = end - duration
    /// - Duration only    → end = selected_date@now, start = end - duration
    /// - Start only       → active entry (no end time)
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
                // Anchor to selected_date at current wall-clock time so that
                // duration-only entries added while browsing a past day land
                // on that day rather than today.
                let now_time = Local::now().time();
                let end = self.selected_date
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

    /// Auto-fill missing time fields when the user tabs away from a field.
    ///
    /// - Leave StartTime:  if Dur set → End = Start + Dur; else if End set → Dur = End − Start
    /// - Leave EndTime:    if Start + Dur → adjust Start; else if Start → Dur = End − Start;
    ///                     else if Dur only → Start = End − Dur
    /// - Leave Duration:   if Start → End = Start + Dur; else if End → Start = End − Dur
    pub(crate) fn apply_time_calculations(&mut self, leaving_field: InputField) {
        let start_str = self.input_start_time.clone();
        let end_str = self.input_end_time.clone();
        let dur_str = self.input_duration.clone();

        let start = if !start_str.is_empty() { self.parse_time_str(&start_str) } else { None };
        let end   = if !end_str.is_empty()   { self.parse_time_str(&end_str)   } else { None };
        let dur   = if !dur_str.is_empty() {
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
            if let (Some(d), Some(m)) = (parts.next(), parts.next()) {
                if let (Ok(day), Ok(month)) = (d.parse::<u32>(), m.parse::<u32>()) {
                    return NaiveDate::from_ymd_opt(current_year, month, day);
                }
            }
        }
        // YYYY-MM-DD
        if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Some(nd);
        }
        // MM-DD
        if s.len() == 5 && s.contains('-') {
            let with_year = format!("{}-{}", current_year, s);
            if let Ok(nd) = NaiveDate::parse_from_str(&with_year, "%Y-%m-%d") {
                return Some(nd);
            }
        }
        None
    }

    /// Parse a time string. Supports 24-hour and 12-hour (am/pm) formats with
    /// `:` or `.` as separator; minutes default to `00` when omitted.
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
            if m > 59 { return None; }
            (h, m)
        } else {
            let h: u32 = rest.trim().parse().ok()?;
            (h, 0)
        };

        let hour_24 = if is_12h {
            if hour == 0 || hour > 12 { return None; }
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

    fn parse_tags(&self) -> Vec<String> {
        self.input_tags
            .split_whitespace()
            .map(|s| s.trim_start_matches('#').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}
