use anyhow::{Context, Result};
use std::{fs, path::PathBuf};

use crate::time;

pub fn get_data_path() -> Result<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("com", "timetracker", "tt")
        .context("Could not determine config directory")?;
    let data_dir = proj_dirs.data_dir();
    fs::create_dir_all(data_dir)?;
    Ok(data_dir.join("data.json"))
}

pub fn load_data() -> Result<time::TimeData> {
    let path = get_data_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(time::TimeData::default())
    }
}

pub fn save_data(data: &time::TimeData) -> Result<()> {
    let path = get_data_path()?;
    let content = serde_json::to_string_pretty(data)?;
    fs::write(path, content)?;
    Ok(())
}
