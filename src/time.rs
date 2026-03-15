use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};


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
        let dur = self.duration();
        let hours = dur.num_hours();
        let minutes = dur.num_minutes() % 60;
        format!("{}h {}m", hours, minutes)
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
