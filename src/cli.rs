use anyhow::Result;
use chrono::Local;
use clap::{Parser, Subcommand};

use crate::duration;
use crate::icons;
use crate::storage::{load_data, save_data};

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
    /// true/false if something is active
    Active,
}

pub fn start(description: Vec<String>) -> Result<()> {
    let mut data = load_data()?;

    if let Some(active) = data.active_entry() {
        println!(
            "{}  Already tracking: \"{}\" (started at {})",
            icons::WARNING,
            active.description,
            active.start_time.format("%H:%M")
        );
        println!("Stop it first with: tt stop");
        return Ok(());
    }

    let desc = description.join(" ");
    let start_time = Local::now();
    data.add_entry(desc.clone(), start_time, None);
    save_data(&data)?;

    println!(
        "{}  Started: \"{}\" at {}",
        icons::ACTIVE,
        desc,
        start_time.format("%H:%M:%S")
    );
    Ok(())
}

pub fn stop() -> Result<()> {
    let mut data = load_data()?;

    // Get info before stopping
    let info = data.active_entry().map(|e| {
        (e.description.clone(), e.format_duration())
    });

    if data.stop_active() {
        let (desc, dur) = info.unwrap();
        save_data(&data)?;
        println!("{}  Stopped: \"{}\" - Duration: {}", icons::STOPPED, desc, dur);
    } else {
        println!("No active task to stop.");
    }
    Ok(())
}

pub fn log(description: String, time_str: String) -> Result<()> {
    let mut data = load_data()?;
    let dur = duration::parse(&time_str);
    let end_time = Local::now();
    let start_time = end_time - dur;

    data.add_entry(description.clone(), start_time, Some(end_time));
    save_data(&data)?;

    println!(
        "{} Logged: \"{}\" - Duration: {}",
        icons::LOGGED,
        description,
        duration::format(dur)
    );
    Ok(())
}

pub fn today() -> Result<()> {
    let data = load_data()?;
    let today_entries = data.today_entries();

    if today_entries.is_empty() {
        println!("No entries for today.");
        return Ok(());
    }

    println!("{} Today's entries:\n", icons::CALENDAR);
    for entry in &today_entries {
        let status = if entry.is_active() { entry.status_icon() } else { "  " };
        println!(
            "{}{} - {} ({})",
            status,
            entry.start_time.format("%H:%M"),
            entry.description,
            entry.format_duration()
        );
    }
    println!("\nTotal: {}", duration::format(data.today_total()));
    Ok(())
}

pub fn list() -> Result<()> {
    let data = load_data()?;

    if data.entries.is_empty() {
        println!("No entries yet.");
        return Ok(());
    }

    println!("{} All entries:\n", icons::LIST);
    for entry in data.entries.iter().rev().take(20) {
        let status = if entry.is_active() { entry.status_icon() } else { "  " };
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

    if let Some(active) = data.active_entry() {
        println!("{}  Currently tracking: \"{}\"", icons::ACTIVE, active.description);
        println!("   Started at: {}", active.start_time.format("%H:%M:%S"));
        println!("   Duration: {}", active.format_duration());
    } else {
        println!("No active task. Start one with: tt start <description>");
    }
    Ok(())
}

pub fn active() -> Result<()> {
    Ok(data.active_entry().is_some())
}