use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::duration;
use crate::icons;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TimeEntry {
    pub id: u64,
    pub description: String,
    pub start_time: DateTime<Local>,
    pub end_time: Option<DateTime<Local>>,
}

impl TimeEntry {
    pub fn duration(&self) -> Duration {
        let end = self.end_time.unwrap_or_else(Local::now);
        end.signed_duration_since(self.start_time)
    }

    pub fn format_duration(&self) -> String {
        duration::format(self.duration())
    }

    pub fn is_active(&self) -> bool {
        self.end_time.is_none()
    }

    /// Returns the status icon for this entry (active or empty)
    pub fn status_icon(&self) -> &'static str {
        if self.is_active() {
            icons::ACTIVE
        } else {
            ""
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TimeData {
    pub entries: Vec<TimeEntry>,
    pub next_id: u64,
}

impl TimeData {
    pub fn active_entry(&self) -> Option<&TimeEntry> {
        self.entries.iter().find(|e| e.is_active())
    }

    pub fn active_entry_mut(&mut self) -> Option<&mut TimeEntry> {
        self.entries.iter_mut().find(|e| e.is_active())
    }

    /// Stop the currently active entry. Returns true if an entry was stopped.
    pub fn stop_active(&mut self) -> bool {
        if let Some(entry) = self.active_entry_mut() {
            entry.end_time = Some(Local::now());
            true
        } else {
            false
        }
    }

    pub fn today_entries(&self) -> Vec<&TimeEntry> {
        let today = Local::now().date_naive();
        self.entries
            .iter()
            .filter(|e| e.start_time.date_naive() == today)
            .collect()
    }

    pub fn today_total(&self) -> Duration {
        self.today_entries()
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    pub fn add_entry(
        &mut self,
        description: String,
        start_time: DateTime<Local>,
        end_time: Option<DateTime<Local>>,
    ) -> &TimeEntry {
        let entry = TimeEntry {
            id: self.next_id,
            description,
            start_time,
            end_time,
        };
        self.next_id += 1;
        self.entries.push(entry);
        self.entries.last().unwrap()
    }

    /// Get entries for a specific date
    pub fn entries_for_date(&self, date: NaiveDate) -> Vec<&TimeEntry> {
        self.entries
            .iter()
            .filter(|e| e.start_time.date_naive() == date)
            .collect()
    }

    /// Get total duration for a specific date
    pub fn total_for_date(&self, date: NaiveDate) -> Duration {
        self.entries_for_date(date)
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    /// Get the start of the week (Monday) for a given date
    pub fn week_start(date: NaiveDate) -> NaiveDate {
        let days_from_monday = date.weekday().num_days_from_monday();
        date - Duration::days(days_from_monday as i64)
    }

    /// Get entries for a specific week (starting Monday)
    pub fn entries_for_week(&self, week_start: NaiveDate) -> Vec<&TimeEntry> {
        let week_end = week_start + Duration::days(7);
        self.entries
            .iter()
            .filter(|e| {
                let date = e.start_time.date_naive();
                date >= week_start && date < week_end
            })
            .collect()
    }

    /// Get total duration for a specific week
    pub fn total_for_week(&self, week_start: NaiveDate) -> Duration {
        self.entries_for_week(week_start)
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    /// Get daily breakdown for a week (returns Vec of (date, total_duration))
    pub fn daily_breakdown(&self, week_start: NaiveDate) -> Vec<(NaiveDate, Duration)> {
        (0..7)
            .map(|i| {
                let date = week_start + Duration::days(i);
                (date, self.total_for_date(date))
            })
            .collect()
    }
}
