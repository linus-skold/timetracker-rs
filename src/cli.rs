use anyhow::Result;
use chrono::{Duration, Local};
use clap::{Parser, Subcommand};

use crate::storage::{load_data, save_data};
use crate::time::TimeEntry;

#[derive(Parser)]
#[command(name = "tt", about = "Simple time tracking CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start tracking a new task
    Start {
        /// Description of the task
        #[arg(required = true)]
        description: Vec<String>,
    },
    /// Stop the current active task
    Stop,
    /// Log a completed task with a specific duration
    Log {
        /// Description of the task
        description: String,
        /// Duration in format like "1h30m", "45m", "2h"
        time: String,
    },
    /// Show all entries for today
    Today,
    /// Show all entries
    List,
    /// Open interactive TUI
    Tui,
    /// Show current status
    Status,
}

pub fn start(description: Vec<String>) -> Result<()> {
    let mut data = load_data()?;

    // Check if there's already an active entry
    if let Some(active) = data.entries.iter().find(|e| e.is_active()) {
        println!(
            "⚠️  Already tracking: \"{}\" (started at {})",
            active.description,
            active.start_time.format("%H:%M")
        );
        println!("Stop it first with: tt stop");
        return Ok(());
    }

    let desc = description.join(" ");
    let entry = TimeEntry {
        id: data.next_id,
        description: desc.clone(),
        start_time: Local::now(),
        end_time: None,
    };
    data.next_id += 1;
    data.entries.push(entry.clone());
    save_data(&data)?;

    println!(
        "▶️  Started: \"{}\" at {}",
        desc,
        entry.start_time.format("%H:%M:%S")
    );
    Ok(())
}

pub fn stop() -> Result<()> {
    let mut data = load_data()?;

    let active_idx = data.entries.iter().position(|e| e.is_active());
    match active_idx {
        Some(idx) => {
            data.entries[idx].end_time = Some(Local::now());
            let entry = &data.entries[idx];
            save_data(&data)?;
            println!(
                "⏹️  Stopped: \"{}\" - Duration: {}",
                entry.description,
                entry.format_duration()
            );
        }
        None => {
            println!("No active task to stop.");
        }
    }
    Ok(())
}

fn parse_duration(time_str: &str) -> Result<Duration> {
    let mut hours = 0i64;
    let mut minutes = 0i64;
    let mut current_num = String::new();

    for c in time_str.chars() {
        match c {
            '0'..='9' => current_num.push(c),
            'h' | 'H' => {
                hours = current_num.parse().unwrap_or(0);
                current_num.clear();
            }
            'm' | 'M' => {
                minutes = current_num.parse().unwrap_or(0);
                current_num.clear();
            }
            _ => {}
        }
    }

    // Handle bare number as minutes
    if !current_num.is_empty() && hours == 0 && minutes == 0 {
        minutes = current_num.parse().unwrap_or(0);
    }

    Ok(Duration::hours(hours) + Duration::minutes(minutes))
}

pub fn log(description: String, time: String) -> Result<()> {
    let mut data = load_data()?;
    let duration = parse_duration(&time)?;
    let end_time = Local::now();
    let start_time = end_time - duration;

    let entry = TimeEntry {
        id: data.next_id,
        description: description.clone(),
        start_time,
        end_time: Some(end_time),
    };
    data.next_id += 1;
    data.entries.push(entry.clone());
    save_data(&data)?;

    println!(
        "📝 Logged: \"{}\" - Duration: {}",
        description,
        entry.format_duration()
    );
    Ok(())
}

pub fn today() -> Result<()> {
    let data = load_data()?;
    let today = Local::now().date_naive();

    let today_entries: Vec<_> = data
        .entries
        .iter()
        .filter(|e| e.start_time.date_naive() == today)
        .collect();

    if today_entries.is_empty() {
        println!("No entries for today.");
        return Ok(());
    }

    println!("📅 Today's entries:\n");
    let mut total = Duration::zero();
    for entry in &today_entries {
        let status = if entry.is_active() { "▶️ " } else { "  " };
        println!(
            "{}{} - {} ({})",
            status,
            entry.start_time.format("%H:%M"),
            entry.description,
            entry.format_duration()
        );
        total = total + entry.duration();
    }
    println!(
        "\nTotal: {}h {}m",
        total.num_hours(),
        total.num_minutes() % 60
    );
    Ok(())
}

pub fn list() -> Result<()> {
    let data = load_data()?;

    if data.entries.is_empty() {
        println!("No entries yet.");
        return Ok(());
    }

    println!("📋 All entries:\n");
    for entry in data.entries.iter().rev().take(20) {
        let status = if entry.is_active() { "▶️ " } else { "  " };
        println!(
            "{}{} {} - {} ({})",
            status,
            entry.start_time.format("%Y-%m-%d"),
            entry.start_time.format("%H:%M"),
            entry.description,
            entry.format_duration()
        );
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let data = load_data()?;

    if let Some(active) = data.entries.iter().find(|e| e.is_active()) {
        println!("▶️  Currently tracking: \"{}\"", active.description);
        println!("   Started at: {}", active.start_time.format("%H:%M:%S"));
        println!("   Duration: {}", active.format_duration());
    } else {
        println!("No active task. Start one with: tt start <description>");
    }
    Ok(())
}
