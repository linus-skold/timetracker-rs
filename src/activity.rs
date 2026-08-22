//! Reader and writer for the hook-only activity ledger.
//!
//! Alongside `marks/`, `activity/` holds one file per Claude Code session,
//! keyed by the harness's own session id. Written only by hooks (see
//! `skills/tt-time-logging/scripts/`), never by the model, so its presence
//! proves a session was active even if `tt agent begin` was never called.
//! See `docs/decisions/0001-agent-activity-tracking.md`.

use chrono::Local;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// `$TT_ACTIVITY_DIR` when set, else `activity` inside the cache dir —
/// a sibling of `marks`.
pub fn activity_dir() -> Option<PathBuf> {
    resolve_activity_dir(std::env::var_os("TT_ACTIVITY_DIR"), cache_dir())
}

/// Same cache root [`crate::marks::mark_dir`] resolves from.
fn cache_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "timetracker", "tt")?;
    Some(dirs.cache_dir().to_path_buf())
}

fn resolve_activity_dir(activity_dir: Option<OsString>, cache: Option<PathBuf>) -> Option<PathBuf> {
    match activity_dir {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(cache?.join("activity")),
    }
}

/// Sanitised the same way [`crate::marks::mark_key`] sanitises a mark's key.
fn session_key(session_id: &str) -> String {
    session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn session_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(session_key(session_id))
}

/// `SessionStart`: open this session's window. Idempotent — an existing file
/// is left untouched, so a hook firing twice never overwrites the real start.
pub fn begin_in(dir: &Path, session_id: &str, project: Option<&str>) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = session_path(dir, session_id);
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(err) => return Err(err),
    };
    writeln!(file, "start={}", Local::now().timestamp())?;
    if let Some(project) = project {
        writeln!(file, "project={project}")?;
    }
    Ok(())
}

/// `Stop`: append this session's end epoch. A missing session file is
/// created bare, so an end is never lost for want of a prior `begin_in`.
pub fn end_in(dir: &Path, session_id: &str) -> io::Result<()> {
    append_field(dir, session_id, "end")
}

/// `SubagentStop`: mark that one subagent dispatch finished. No start of its
/// own — just evidence a dispatch happened, not a windowed span.
pub fn subagent_in(dir: &Path, session_id: &str) -> io::Result<()> {
    append_field(dir, session_id, "subagent")
}

fn append_field(dir: &Path, session_id: &str, field: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = session_path(dir, session_id);
    let mut file = OpenOptions::new().append(true).create(true).open(&path)?;
    writeln!(file, "{field}={}", Local::now().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-activity-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(dir: &Path, session_id: &str) -> String {
        fs::read_to_string(session_path(dir, session_id)).unwrap()
    }

    #[test]
    fn begin_writes_start_and_project() {
        let dir = sandbox("begin");
        begin_in(&dir, "sess-1", Some("timetracker")).unwrap();

        let body = read(&dir, "sess-1");
        assert!(body.lines().any(|l| l.starts_with("start=")));
        assert_eq!(
            body.lines().find(|l| l.starts_with("project=")),
            Some("project=timetracker")
        );
    }

    #[test]
    fn begin_with_no_project_writes_no_project_line() {
        let dir = sandbox("begin-no-project");
        begin_in(&dir, "sess-1", None).unwrap();

        let body = read(&dir, "sess-1");
        assert!(!body.lines().any(|l| l.starts_with("project=")));
    }

    #[test]
    fn a_second_begin_leaves_the_first_start_untouched() {
        let dir = sandbox("begin-twice");
        begin_in(&dir, "sess-1", Some("first")).unwrap();
        let first = read(&dir, "sess-1");

        begin_in(&dir, "sess-1", Some("second")).unwrap();
        assert_eq!(
            read(&dir, "sess-1"),
            first,
            "the original start was rewritten"
        );
    }

    #[test]
    fn end_appends_without_disturbing_start() {
        let dir = sandbox("end");
        begin_in(&dir, "sess-1", Some("tt")).unwrap();
        end_in(&dir, "sess-1").unwrap();

        let body = read(&dir, "sess-1");
        let lines: Vec<&str> = body.lines().collect();
        assert!(lines[0].starts_with("start="));
        assert!(lines.iter().any(|l| l.starts_with("end=")));
    }

    #[test]
    fn end_with_no_prior_begin_still_records_something() {
        let dir = sandbox("end-no-begin");
        end_in(&dir, "sess-1").unwrap();

        let body = read(&dir, "sess-1");
        assert!(body.lines().any(|l| l.starts_with("end=")));
    }

    #[test]
    fn subagent_markers_accumulate_one_per_dispatch() {
        let dir = sandbox("subagent");
        begin_in(&dir, "sess-1", Some("tt")).unwrap();
        subagent_in(&dir, "sess-1").unwrap();
        subagent_in(&dir, "sess-1").unwrap();

        let body = read(&dir, "sess-1");
        assert_eq!(
            body.lines().filter(|l| l.starts_with("subagent=")).count(),
            2
        );
    }

    #[test]
    fn a_session_id_with_unsafe_characters_is_sanitised() {
        let dir = sandbox("sanitise");
        begin_in(&dir, "weird/id:with*chars", Some("tt")).unwrap();

        let entries: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let name = entries[0].file_name();
        assert!(
            name.to_str()
                .unwrap()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            "{name:?} was not sanitised"
        );
    }

    #[test]
    fn the_default_directory_is_activity_inside_the_app_cache_dir() {
        let cache = || Some(PathBuf::from("cache"));
        assert_eq!(
            resolve_activity_dir(None, cache()),
            Some(PathBuf::from("cache").join("activity"))
        );
        assert_eq!(
            resolve_activity_dir(Some("".into()), cache()),
            Some(PathBuf::from("cache").join("activity")),
            "an empty TT_ACTIVITY_DIR is no setting at all"
        );
        assert_eq!(
            resolve_activity_dir(Some("elsewhere".into()), cache()),
            Some(PathBuf::from("elsewhere"))
        );
        assert_eq!(
            resolve_activity_dir(None, None),
            None,
            "no cache dir, no default"
        );
    }
}
