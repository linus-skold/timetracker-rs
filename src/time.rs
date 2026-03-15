use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};

use crate::duration;

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
    ) -> TimeEntry {
        let entry = TimeEntry {
            id: self.next_id,
            description,
            start_time,
            end_time,
        };
        self.next_id += 1;
        self.entries.push(entry.clone());
        entry
    }
}
