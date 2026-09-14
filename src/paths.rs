//! The path conventions every other module resolves through: this app's own
//! directories, the rule a `TT_*` override follows, and the sanitiser that
//! turns untrusted text into a filename.
//!
//! These were four sets of copies across `marks`, `activity`, `storage` and
//! `config` — modules with no other reason to know about each other.

use std::ffi::OsString;
use std::path::PathBuf;

/// The `ProjectDirs` triple every directory in the app derives from.
///
/// One place on purpose: it is the single fact that makes a sandboxed `HOME`
/// redirect the store, the marks and the activity ledger *together*. Three
/// copies of it could drift apart and send one of them at the real directory
/// during a test. See [`crate::storage::env_sandbox`].
fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("com", "timetracker", "tt")
}

/// This app's cache directory — the parent of `marks/` and `activity/`.
pub fn cache_dir() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.cache_dir().to_path_buf())
}

/// This app's data directory — where `data.json` lives.
pub fn data_dir() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.data_dir().to_path_buf())
}

/// The user's home directory.
///
/// `~/.claude` and the agent skills directories under it belong to other tools,
/// not to this app, so they do not come from `project_dirs`.
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from)
}

/// The rule every `TT_*` path override follows: the variable when it is set and
/// non-empty, else `default`.
///
/// An empty variable is **no setting at all**, not a setting of "". Without
/// that, a blank left in the environment would resolve to a relative empty path
/// rather than falling back, and the caller would silently read and write
/// somewhere other than where it meant to.
pub fn env_or(value: Option<OsString>, default: Option<PathBuf>) -> Option<PathBuf> {
    match value {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => default,
    }
}

/// Every character outside `[A-Za-z0-9_-]` replaced by `_`, and every `.` by `-`.
///
/// Both a mark's key and an activity session's id become filenames built from
/// text this program does not control — a project name from the environment, a
/// session id from the harness. Sanitising them the same way is what lets a
/// path be rebuilt from its parts and still find the file it wrote. A `.` is
/// reserved as [`crate::marks::mark_key`]'s segment separator, so no sanitised
/// segment may hold one.
pub fn sanitise_key(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '.' => '-',
            '_' | '-' => c,
            c if c.is_ascii_alphanumeric() => c,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one test for the override rule. Each caller then asserts only what is
    /// its own — which variable it reads and what its default is composed of.
    #[test]
    fn an_override_wins_unless_it_is_empty() {
        let default = || Some(PathBuf::from("default"));
        assert_eq!(env_or(None, default()), default(), "unset falls back");
        assert_eq!(
            env_or(Some("".into()), default()),
            default(),
            "an empty variable is no setting at all"
        );
        assert_eq!(
            env_or(Some("elsewhere".into()), default()),
            Some(PathBuf::from("elsewhere")),
            "a set variable wins"
        );
        assert_eq!(env_or(None, None), None, "no default, nothing to resolve");
        assert_eq!(
            env_or(Some("elsewhere".into()), None),
            Some(PathBuf::from("elsewhere")),
            "an override alone is enough to run"
        );
    }

    #[test]
    fn a_key_keeps_what_is_legal_and_replaces_the_rest() {
        assert_eq!(sanitise_key("tt.8.impl"), "tt-8-impl", "a dot is not legal");
        assert_eq!(sanitise_key("-"), "-", "the sentinel");
        assert_eq!(sanitise_key("my proj/7"), "my_proj_7");
        assert_eq!(sanitise_key("a_b-c"), "a_b-c", "both legal symbols");
    }

    /// Not asserting a concrete path — it is per-OS and per-user. What matters is
    /// that both come from one triple, so a redirected HOME moves them together.
    #[test]
    fn the_cache_and_data_directories_resolve_together() {
        assert_eq!(cache_dir().is_some(), data_dir().is_some());
    }
}
