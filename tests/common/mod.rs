//! The shared harness for the `tt agent` integration tests.
//!
//! Every case runs in a throwaway `HOME` *and* `TT_MARK_DIR`, and [`Case::run`]
//! refuses a command whose paths are not inside the sandbox. Time is fabricated at
//! synthetic epochs; expectations are derived from the fixture, never written down.
//!
//! Each test binary compiles this module separately, so an item only one of them
//! uses is `dead_code` in the other — hence the crate-level allow.
#![allow(dead_code)]

use chrono::{DateTime, Local};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// One case's sandbox: a `HOME`, a mark directory inside it, and nothing else.
pub struct Case {
    pub home: PathBuf,
    pub marks: PathBuf,
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
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&marks).unwrap();
        Self { home, marks }
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
        assert!(
            self.home.starts_with(std::env::temp_dir())
                && self.marks.starts_with(self.home.parent().unwrap()),
            "sandbox paths escaped the scratch directory: {:?}",
            self.home
        );

        let mut command = Command::new(env!("CARGO_BIN_EXE_tt"));
        command.env_clear();
        command.env("HOME", &self.home);
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

    /// Write a `config.toml` into the sandbox, at the path `config::load` reads.
    pub fn write_config(&self, body: &str) {
        let dir = self.home.join(".config/timetracker-rs");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), body).unwrap();
    }

    /// Fabricate an open mark started at an absolute epoch.
    pub fn write_mark(&self, key: &str, start: i64) {
        fs::write(self.mark_file(key), format!("{start}\n")).unwrap();
    }

    /// Run `tt <args>` with **no** `agent` prefix, for the store-reading commands.
    pub fn run_bare(&self, args: &[&str]) -> Run {
        self.run_with(args, true)
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

    /// The sandbox's store directory — where `data.json` and `data.lock` would land.
    pub fn data_dir(&self) -> PathBuf {
        self.home
            .join("Library/Application Support/com.timetracker.tt")
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

/// The rounding `tt` applies to a logged duration.
pub fn round_quarter(minutes: i64) -> i64 {
    (((minutes + 7) / 15) * 15).max(15)
}

/// The `- Duration:` tail `cli::log` prints for `minutes` — the figure it was asked
/// for, never the stored span.
pub fn logged_duration(minutes: i64) -> String {
    let rounded = round_quarter(minutes);
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

    /// The minutes `cli::log` said it logged, read off its own output — the requested
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
