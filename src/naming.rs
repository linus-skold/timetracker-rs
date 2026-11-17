//! The one spelling of project and tag names: trimmed and lowercase.
//! Every writer and the store loader go through here.

use crate::tracker::TimeData;

pub fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

pub fn normalize_project(project: Option<String>) -> Option<String> {
    project.map(|p| normalize_name(&p))
}

/// Lowercases each tag and drops repeats, keeping the first of each.
pub fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = normalize_name(&tag);
        if !seen.contains(&tag) {
            seen.push(tag);
        }
    }
    seen
}

/// Applied at load, so old mixed-case data is lowercase in memory and
/// reaches the disk at the next save.
pub fn normalize_store(data: &mut TimeData) {
    for entry in &mut data.entries {
        entry.project = normalize_project(entry.project.take());
        entry.tags = normalize_tags(std::mem::take(&mut entry.tags));
    }
}

#[cfg(test)]
#[path = "naming_tests.rs"]
mod tests;
