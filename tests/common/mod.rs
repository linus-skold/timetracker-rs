//! The shared harness for the `tt agent` integration tests.
//!
//! Every case runs in a throwaway `HOME`, `TT_MARK_DIR`, `TT_DATA_DIR` and
//! `TT_CONFIG_FILE`, and [`Case::run`] refuses a command whose paths are not inside
//! the sandbox. Time is fabricated at synthetic epochs; expectations are derived
//! from the fixture, never written down.
//!
//! Each test binary compiles this module separately, so an item only one of them
//! uses is `dead_code` in the other — hence the crate-level allow.
#![allow(dead_code)]

use chrono::{DateTime, Local};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// One case's sandbox: a `HOME`, a mark directory, a store directory and a config
/// file beside it, and nothing else.
pub struct Case {
    pub home: PathBuf,
    pub marks: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
    pub activity: PathBuf,
}

pub struct Run {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Case {
    pub fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("tt-agent-test-{name}"));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let marks = root.join("marks");
        let data = root.join("store");
        let config = root.join("config.toml");
        let activity = root.join("activity");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&marks).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&activity).unwrap();
        Self {
            home,
            marks,
            data,
            config,
            activity,
        }
    }

    /// Run `tt agent <args>` with the sandbox in force. `env_clear`, not overrides:
    /// an inherited `TT_MARK_DIR` would silently point a case at the live marks.
    pub fn run(&self, args: &[&str]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_with(&argv, true)
    }

    /// Run `tt agent <args>` with **no** `TT_MARK_DIR`, exercising the default path.
    pub fn run_in_cache(&self, args: &[&str]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_with(&argv, false)
    }

    /// Run `tt agent <args>` with one extra `KEY=value` that `env_clear` would strip.
    pub fn run_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_full(&argv, true, env)
    }

    fn run_with(&self, args: &[&str], mark_dir: bool) -> Run {
        self.run_full(args, mark_dir, &[])
    }

    fn run_full(&self, args: &[&str], mark_dir: bool, env: &[(&str, &str)]) -> Run {
        let root = self.home.parent().unwrap();
        assert!(
            self.home.starts_with(std::env::temp_dir())
                && self.marks.starts_with(root)
                && self.data.starts_with(root)
                && self.config.starts_with(root)
                && self.activity.starts_with(root),
            "sandbox paths escaped the scratch directory: {:?}",
            self.home
        );

        let mut command = Command::new(env!("CARGO_BIN_EXE_tt"));
        command.env_clear();
        command.env("HOME", &self.home);
        command.env("TT_DATA_DIR", &self.data);
        command.env("TT_CONFIG_FILE", &self.config);
        command.env("TT_ACTIVITY_DIR", &self.activity);
        // `env_clear()` strips this too, so it has to be set explicitly on every
        // run — otherwise every one of these subprocess invocations would attempt
        // a real network call to GitHub's release API during `cargo test`.
        command.env("TT_SKIP_UPDATE_CHECK", "1");
        if mark_dir {
            command.env("TT_MARK_DIR", &self.marks);
        }
        for (key, value) in env {
            command.env(key, value);
        }
        command.args(args);

        let Output {
            status,
            stdout,
            stderr,
        } = command.output().expect("the tt binary should run");
        Run {
            status: status.code(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        }
    }

    pub fn mark_file(&self, key: &str) -> PathBuf {
        self.marks.join(key)
    }

    pub fn beats_file(&self, key: &str) -> PathBuf {
        self.marks.join("beats").join(key)
    }

    pub fn closing_file(&self, key: &str) -> PathBuf {
        self.marks.join("closing").join(key)
    }

    /// Fabricate the leftover an unfinished close would have left, holding the
    /// start its mark holds.
    pub fn write_closing(&self, key: &str, start: i64) {
        let file = self.closing_file(key);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, format!("{start}\n")).unwrap();
    }

    /// Write the sandbox's `config.toml`, at the path `TT_CONFIG_FILE` names.
    pub fn write_config(&self, body: &str) {
        fs::write(&self.config, body).unwrap();
    }

    /// Fabricate an open mark started at an absolute epoch.
    pub fn write_mark(&self, key: &str, start: i64) {
        fs::write(self.mark_file(key), format!("{start}\n")).unwrap();
    }

    /// Fabricate an activity-ledger session file the way a hook would, at
    /// absolute epochs. `end` is `None` for a still-open session.
    pub fn write_session(&self, session_id: &str, project: &str, start: i64, end: Option<i64>) {
        let mut body = format!("start={start}\nproject={project}\n");
        if let Some(end) = end {
            body.push_str(&format!("end={end}\n"));
        }
        fs::write(self.activity.join(session_id), body).unwrap();
    }

    /// Run `tt <args>` with **no** `agent` prefix, for the store-reading commands.
    pub fn run_bare(&self, args: &[&str]) -> Run {
        self.run_with(args, true)
    }

    /// [`Case::run_bare`] with extra `KEY=value` pairs that `env_clear` would strip.
    pub fn run_bare_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Run {
        self.run_full(args, true, env)
    }

    /// Lay a `data.json` into the sandbox from fabricated rows, at `schema_version:
    /// 1` so the migrate preamble early-returns and the `project` fields survive.
    pub fn write_store(&self, rows: &[StoreRow]) {
        let entries: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let project = match row.project {
                    Some(name) => format!("\"{name}\""),
                    None => "null".to_string(),
                };
                let tags: Vec<String> =
                    row.tags.iter().map(|tag| format!("\"{tag}\"")).collect();
                let end = match row.end {
                    Some(end) => format!("\"{}\"", stamp(end)),
                    None => "null".to_string(),
                };
                format!(
                    r#"{{"id":{},"description":"{}","project":{},"tags":[{}],"start_time":"{}","end_time":{},"idle":[]}}"#,
                    i as u64 + 1,
                    row.description,
                    project,
                    tags.join(","),
                    stamp(row.start),
                    end
                )
            })
            .collect();

        let dir = self.data_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("data.json"),
            format!(
                r#"{{"entries":[{}],"next_id":{},"schema_version":1}}"#,
                entries.join(","),
                rows.len() as u64 + 1
            ),
        )
        .unwrap();
    }

    /// Lay a `data.json` into the sandbox verbatim, for a store the migration must fix.
    pub fn write_raw_store(&self, json: &str) {
        let dir = self.data_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("data.json"), json).unwrap();
    }

    /// The sandbox's store directory — where `data.json` and `data.lock` land,
    /// because `TT_DATA_DIR` names it.
    pub fn data_dir(&self) -> PathBuf {
        self.data.clone()
    }

    /// The sandbox's store, parsed back from its own JSON.
    pub fn store(&self) -> Store {
        let path = self.data_dir().join("data.json");
        let body = fs::read_to_string(path).expect("the sandbox's data.json");
        serde_json::from_str(&body).expect("data.json should parse")
    }

    /// Fabricate a heartbeat sequence at absolute epochs, so a long phase is free.
    pub fn beats_at(&self, key: &str, beats: &[i64]) {
        let file = self.beats_file(key);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let body: String = beats.iter().map(|beat| format!("{beat}\n")).collect();
        fs::write(file, body).unwrap();
    }

    /// Regular files directly in the mark directory — the `beats` subdirectory is not one.
    pub fn mark_count(&self) -> usize {
        count_files(&self.marks)
    }
}

/// One path's permission bits for the length of a scope, put back on drop whether
/// the case passed or panicked. [`Case::new`] opens with `remove_dir_all`, so a
/// mode left behind fails the **next** run of that case rather than this one.
#[cfg(unix)]
pub struct Mode {
    path: PathBuf,
    original: fs::Permissions,
}

#[cfg(unix)]
impl Mode {
    pub fn set(path: &Path, mode: u32) -> Self {
        let original = fs::metadata(path).expect("the path to exist").permissions();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        Self {
            path: path.to_path_buf(),
            original,
        }
    }
}

#[cfg(unix)]
impl Drop for Mode {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.original.clone());
    }
}

pub fn count_files(dir: &Path) -> usize {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .count(),
        Err(_) => 0,
    }
}

pub fn count_lines(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |body| body.lines().count())
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The `HH:MM` a fixture's own epoch renders as.
pub fn clock(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string()
}

/// One fabricated store row for [`Case::write_store`].
pub struct StoreRow {
    pub description: &'static str,
    pub project: Option<&'static str>,
    pub tags: &'static [&'static str],
    pub start: i64,
    /// Epoch seconds; `None` leaves the entry running.
    pub end: Option<i64>,
}

/// An epoch second as the store's RFC 3339 local timestamp.
pub fn stamp(epoch: i64) -> String {
    DateTime::<chrono::Utc>::from_timestamp(epoch, 0)
        .expect("a real epoch")
        .with_timezone(&Local)
        .to_rfc3339()
}

/// The store as `tt` writes it, with only the fields these tests assert on.
#[derive(Debug, serde::Deserialize)]
pub struct Store {
    pub entries: Vec<Entry>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Entry {
    pub description: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub start_time: DateTime<Local>,
    pub end_time: Option<DateTime<Local>>,
    #[serde(default)]
    pub idle: Vec<Idle>,
}

impl Entry {
    /// How long this entry covers, in whole seconds.
    pub fn seconds(&self) -> i64 {
        self.end_time
            .expect("a logged entry is closed")
            .signed_duration_since(self.start_time)
            .num_seconds()
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct Idle {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

impl Idle {
    /// The recorded stretch as a raw `(from, to)` epoch pair.
    pub fn epochs(&self) -> (i64, i64) {
        (self.start.timestamp(), self.end.timestamp())
    }
}

/// The rounding `tt` applies to a logged duration: up to the next 5 minutes,
/// never below 5.
pub fn round_five(minutes: i64) -> i64 {
    (((minutes + 4) / 5) * 5).max(5)
}

/// The `- Duration:` tail `commands::log` prints for `minutes` — the figure it was asked
/// for, never the stored span.
pub fn logged_duration(minutes: i64) -> String {
    let rounded = round_five(minutes);
    format!("- Duration: {}h {}m", rounded / 60, rounded % 60)
}

impl Run {
    pub fn assert_status(&self, expected: i32) {
        assert_eq!(
            self.status,
            Some(expected),
            "exit code (stderr: {:?}, stdout: {:?})",
            self.stderr,
            self.stdout
        );
    }

    pub fn assert_stdout_has(&self, needle: &str) {
        assert!(
            self.stdout.contains(needle),
            "stdout missing {needle:?}: {:?}",
            self.stdout
        );
    }

    /// The minutes `commands::log` said it logged, read off its own output — the requested
    /// figure, before any split, never the stored span.
    pub fn logged_minutes(&self) -> i64 {
        let tail = self
            .stdout
            .split("- Duration: ")
            .nth(1)
            .unwrap_or_else(|| panic!("no logged duration in {:?}", self.stdout));
        let (hours, rest) = tail.split_once('h').expect("`<h>h <m>m`");
        let (minutes, _) = rest.trim_start().split_once('m').expect("`<h>h <m>m`");
        hours.trim().parse::<i64>().unwrap() * 60 + minutes.trim().parse::<i64>().unwrap()
    }

    pub fn assert_stderr_has(&self, needle: &str) {
        assert!(
            self.stderr.contains(needle),
            "stderr missing {needle:?}: {:?}",
            self.stderr
        );
    }
}
