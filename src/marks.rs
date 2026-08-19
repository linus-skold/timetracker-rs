//! Reader and writer for the agent layer's open phase marks.
//!
//! This module owns *every* fact about the format: one file per phase at
//! `<mark dir>/<project>.<issue>.<phase>` holding a unix-seconds start timestamp,
//! where the mark directory is `$TT_MARK_DIR` if set, else `marks` inside this
//! app's cache directory. The name is sanitised `[^A-Za-z0-9._-]` → `_`, so a
//! segment may itself contain `.` or `_` and the name is **not** losslessly
//! splittable. Heartbeats are one append-only file per mark in a `beats/`
//! subdirectory.
//!
//! Only the start timestamp is read; see [`open_marks_in`].

use chrono::{DateTime, Local, TimeDelta};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::duration;

/// One open phase mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub project: String,
    /// `None` when the mark was made with the no-issue sentinel `-`.
    pub issue: Option<String>,
    pub phase: String,
    pub start: DateTime<Local>,
}

impl Mark {
    /// `project/issue phase`, or bare `project phase` for the `-` sentinel.
    pub fn label(&self) -> String {
        let subject = match &self.issue {
            Some(issue) => format!("{}/{}", self.project, issue),
            None => self.project.clone(),
        };
        if self.phase.is_empty() {
            return subject;
        }
        format!("{} {}", subject, self.phase)
    }

    /// The clock time the mark was made, `HH:MM`.
    pub fn started_at(&self) -> String {
        self.start.format("%H:%M").to_string()
    }

    /// How long this mark has been open, as `2m` or `2h 6m`. Derived on every
    /// call and **never cached**: the mark list is only re-read on a directory
    /// change.
    pub fn elapsed(&self) -> String {
        self.elapsed_at(Local::now())
    }

    /// The `now`-taking half of [`elapsed`](Mark::elapsed), for tests.
    pub fn elapsed_at(&self, now: DateTime<Local>) -> String {
        // A start in the future reads as 0m, never as a negative age.
        let minutes = (now - self.start).num_minutes().max(0);
        match minutes / 60 {
            0 => format!("{}m", minutes),
            hours => format!("{}h {}m", hours, minutes % 60),
        }
    }

    /// How long this mark has been open in the house `{h}h {m}m` format, not
    /// [`elapsed`](Mark::elapsed)'s narrow one. Clamped first, or a start in the
    /// future would print `0h -5m`.
    pub fn age_at(&self, now: DateTime<Local>) -> String {
        duration::format((now - self.start).max(TimeDelta::zero()))
    }
}

/// Narrowest label column `tt agent list` will use. `src/tui/render.rs` keeps its
/// own copy for the surface's rows.
const LABEL_WIDTH: usize = 18;

/// The rows `tt agent list` prints, without the CLI's indent: one label column,
/// ` - since HH:MM`, and the house `{h}h {m}m` duration.
pub fn rows(marks: &[Mark]) -> Vec<String> {
    rows_at(marks, Local::now())
}

/// The `now`-taking half of [`rows`], for tests.
pub fn rows_at(marks: &[Mark], now: DateTime<Local>) -> Vec<String> {
    // One column for the whole list, so the start times line up.
    let width = marks
        .iter()
        .map(|mark| mark.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(LABEL_WIDTH);

    marks
        .iter()
        .map(|mark| {
            let label = mark.label();
            let pad = " ".repeat(width.saturating_sub(label.chars().count()));
            format!(
                "{}{} - since {} ({})",
                label,
                pad,
                mark.started_at(),
                mark.age_at(now)
            )
        })
        .collect()
}

/// The directory the marks live in: `$TT_MARK_DIR` when set and non-empty, else
/// `marks` inside this app's cache directory. `None` only when there is no home
/// directory to resolve.
pub fn mark_dir() -> Option<PathBuf> {
    resolve_mark_dir(std::env::var_os("TT_MARK_DIR"), cache_dir())
}

/// This app's cache directory, from the same `ProjectDirs` triple
/// `storage::get_data_path` uses, so a sandboxed `HOME` redirects the marks too.
fn cache_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "timetracker", "tt")?;
    Some(dirs.cache_dir().to_path_buf())
}

/// The env-free half of [`mark_dir`], taking the resolved cache root.
fn resolve_mark_dir(mark_dir: Option<OsString>, cache: Option<PathBuf>) -> Option<PathBuf> {
    match mark_dir {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(cache?.join("marks")),
    }
}

/// Every open mark, newest first. A missing mark directory is an empty list.
pub fn open_marks() -> Vec<Mark> {
    match mark_dir() {
        Some(dir) => open_marks_in(&dir),
        None => Vec::new(),
    }
}

/// Every open mark in `dir`, newest first; a bad file is skipped, never fatal.
///
/// **Read no heartbeat here:** callers refresh on the directory's mtime, which a
/// beat appended inside `beats/` does not change. The `beats/` subdirectory is
/// skipped by the file-type filter below, never by its name — see [`beats_path`].
pub fn open_marks_in(dir: &Path) -> Vec<Mark> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let names: Vec<OsString> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name())
        .collect();

    let mut marks: Vec<Mark> = names
        .iter()
        .filter_map(|name| read_mark(dir, name))
        .collect();

    // Never `read_dir` order.
    marks.sort_by(|a, b| {
        b.start
            .cmp(&a.start)
            .then_with(|| (&a.project, &a.issue, &a.phase).cmp(&(&b.project, &b.issue, &b.phase)))
    });
    marks
}

/// The instant a mark file holds, or `None` when it holds anything else. The one
/// place the file's body is interpreted.
fn read_start(path: &Path) -> Option<DateTime<Local>> {
    let contents = fs::read_to_string(path).ok()?;
    let seconds: i64 = contents.trim().parse().ok()?;
    Some(DateTime::from_timestamp(seconds, 0)?.with_timezone(&Local))
}

/// Parse one mark file, or `None` if it is not one.
fn read_mark(dir: &Path, name: &OsString) -> Option<Mark> {
    let start = read_start(&dir.join(name))?;
    let (project, issue, phase) = split_key(name.to_str()?);
    Some(Mark {
        project,
        issue,
        phase,
        start,
    })
}

/// Split a mark filename back into `<project>.<issue>.<phase>`. Lossy: any
/// segment may contain a `.`, so the split takes the **first** and **last** one
/// and a pathological name degrades to an imperfect label.
fn split_key(name: &str) -> (String, Option<String>, String) {
    let issue = |raw: &str| (raw != "-").then(|| raw.to_string());

    match name.split_once('.') {
        // Extra dots land in the middle field.
        Some((project, rest)) => match rest.rsplit_once('.') {
            Some((mid, phase)) => (project.to_string(), issue(mid), phase.to_string()),
            // One `.`: an issue-less `<project>.<phase>`, as `-` produces.
            None => (project.to_string(), None, rest.to_string()),
        },
        // Nothing to split: the whole name is the project.
        None => (name.to_string(), None, String::new()),
    }
}

// --- writer ---------------------------------------------------------------
//
// Nothing here locks anything: no path below is the store, and `create_new` plus
// `O_APPEND` cover the only two races there are.

/// What [`begin_in`] found: a mark it created, or one that was already open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Begin {
    /// The mark file was created, holding this instant.
    Created(DateTime<Local>),
    /// A mark was already open and is left byte-identical. `None` when its
    /// contents are not a timestamp, which the caller renders as `??:??`.
    AlreadyOpen(Option<DateTime<Local>>),
}

/// What [`touch_in`] did: recorded a heartbeat, or refused for want of a mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    Recorded,
    /// No mark file for this phase, so no beats file was created either.
    NoMark,
}

/// The sanitised filename for one phase: `<project>.<issue>.<phase>` with every
/// character outside `[A-Za-z0-9._-]` replaced by `_`, and the `-` sentinel
/// written literally. Build every mark and beats path from this one key, or a
/// phase can end up with beats it cannot find again.
pub fn mark_key(project: &str, issue: &str, phase: &str) -> String {
    format!("{project}.{issue}.{phase}")
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

pub fn mark_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(key)
}

/// The heartbeat file for `key` inside `dir`: a `beats/` **subdirectory**, never
/// a `<mark>.<suffix>` sibling. Every [`mark_key`] holds at least two dots, so no
/// mark can be named `beats`.
pub fn beats_path(dir: &Path, key: &str) -> PathBuf {
    dir.join("beats").join(key)
}

/// Open a mark for one phase, or report the one already open. `create_new` keeps
/// the check and the write atomic, and an existing mark is never rewritten.
pub fn begin_in(dir: &Path, project: &str, issue: &str, phase: &str) -> io::Result<Begin> {
    let path = mark_path(dir, &mark_key(project, issue, phase));
    fs::create_dir_all(dir)?;

    let start = Local::now();
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            writeln!(file, "{}", start.timestamp())?;
            Ok(Begin::Created(start))
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Ok(Begin::AlreadyOpen(read_start(&path)))
        }
        Err(err) => Err(err),
    }
}

/// Append one heartbeat for a phase that is already marked. Appended, never
/// overwritten — the *sequence* of beats is the evidence. A phase nobody began
/// records nothing at all.
pub fn touch_in(dir: &Path, project: &str, issue: &str, phase: &str) -> io::Result<Touch> {
    let key = mark_key(project, issue, phase);
    if !mark_path(dir, &key).is_file() {
        return Ok(Touch::NoMark);
    }

    let beats = beats_path(dir, &key);
    if let Some(parent) = beats.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().append(true).create(true).open(&beats)?;
    writeln!(file, "{}", Local::now().timestamp())?;
    Ok(Touch::Recorded)
}

/// Drop a mark and its `beats/` entry, each allowed to be absent. The `beats/`
/// directory itself stays; other phases' beats live in it.
pub fn cancel_in(dir: &Path, project: &str, issue: &str, phase: &str) -> io::Result<()> {
    let key = mark_key(project, issue, phase);
    for path in [mark_path(dir, &key), beats_path(dir, &key)] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

// --- what `end` measures --------------------------------------------------
//
// `agent.rs` gets a [`Phase`] and a list of gaps, and never learns the format.

/// One marked phase as `end` needs to read it back, all in unix seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    /// The instant the mark was opened.
    pub started: i64,
    /// Every heartbeat, **in file order**, dropping lines whose first field is
    /// not a bare timestamp. Never sort or dedup: [`gaps_over`] judges that.
    pub beats: Vec<i64>,
    /// The instant the phase is measured to: the beats file's **last line**, not
    /// its largest beat. `None` when there is no such line, leaving the caller
    /// nothing better to measure to than now.
    pub ended: Option<i64>,
}

/// A line's leading whitespace-delimited field as a bare timestamp, or `None`.
fn beat_of(line: &str) -> Option<i64> {
    let field = line.split_whitespace().next()?;
    all_digits(field).then(|| field.parse().ok())?
}

/// A non-empty run of ASCII digits: no sign, no whitespace, no `+`.
fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Read a marked phase back, or `None` when the phase is not marked at all. A
/// mark holding something other than a timestamp is an error naming the path.
pub fn read_phase_in(
    dir: &Path,
    project: &str,
    issue: &str,
    phase: &str,
) -> io::Result<Option<Phase>> {
    let key = mark_key(project, issue, phase);
    let mark = mark_path(dir, &key);
    if !mark.is_file() {
        return Ok(None);
    }
    let started = read_start(&mark)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} does not hold a unix timestamp", mark.display()),
            )
        })?
        .timestamp();

    let source = beats_path(dir, &key);

    // No beats file leaves the single start→end interval to judge.
    let body = fs::read_to_string(&source).unwrap_or_default();
    let beats = body.lines().filter_map(beat_of).collect();
    let ended = body.lines().next_back().filter(|line| all_digits(line));

    Ok(Some(Phase {
        started,
        beats,
        ended: ended.and_then(|line| line.parse().ok()),
    }))
}

/// Every stretch of silence longer than `threshold_minutes`, as chronological
/// `(from, to)` epoch pairs.
///
/// Pure and total: no I/O, and "no gaps" is an answer rather than an error. The
/// sequence judged is `start, beats…, end`, so the leading and trailing intervals
/// count too and a phase with no beats is one stretch across its whole span.
///
/// A beat that does not advance the sequence — a duplicate, an out-of-order line,
/// a beat outside the mark's window — is skipped without advancing `prev`, so it
/// cannot shorten the gap around it.
///
/// The threshold test is `(beat - prev) / 60 > threshold`: **integer-floor
/// minutes, strictly greater**, so at 45 a 45m59s hole is not a gap and 46m00s
/// is.
pub fn gaps_over(start: i64, end: i64, beats: &[i64], threshold_minutes: i64) -> Vec<(i64, i64)> {
    let mut gaps = Vec::new();
    let mut prev = start;

    for &beat in beats {
        if beat <= prev || beat >= end {
            continue;
        }
        if (beat - prev) / 60 > threshold_minutes {
            gaps.push((prev, beat));
        }
        prev = beat;
    }

    if end > prev && (end - prev) / 60 > threshold_minutes {
        gaps.push((prev, end));
    }
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Serialises the one test that repoints `TT_MARK_DIR`, since env is
    /// process-wide; shared with the TUI's tests via `storage::env_guard`.
    use crate::storage::env_guard;

    /// A fresh scratch mark directory: the real one is live and off limits.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-marks-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    fn at(seconds: i64) -> DateTime<Local> {
        DateTime::from_timestamp(seconds, 0)
            .unwrap()
            .with_timezone(&Local)
    }

    fn labels(marks: &[Mark]) -> Vec<(String, Option<String>, String)> {
        marks
            .iter()
            .map(|m| (m.project.clone(), m.issue.clone(), m.phase.clone()))
            .collect()
    }

    #[test]
    fn marks_come_back_newest_first() {
        let dir = sandbox("newest-first");
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "loremind.64.plan", "1000300\n");
        write(&dir, "vinge.-.plan", "1000100\n");

        let marks = open_marks_in(&dir);
        assert_eq!(
            labels(&marks),
            vec![
                ("loremind".into(), Some("64".into()), "plan".into()),
                ("tt".into(), Some("8".into()), "impl".into()),
                ("vinge".into(), None, "plan".into()),
            ]
        );
        assert_eq!(marks[0].start.timestamp(), 1000300);
    }

    #[test]
    fn a_phase_literally_called_last_is_a_mark() {
        let dir = sandbox("phase-last");
        write(&dir, "proj.-.last", "1000100\n");
        write(&dir, "tt.8.impl", "1000200\n");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![
                ("tt".into(), Some("8".into()), "impl".into()),
                ("proj".into(), None, "last".into()),
            ]
        );
    }

    /// The file-type filter excludes `beats/`, so a mark whose *phase* is
    /// `beats` is still listed.
    #[test]
    fn the_beats_subdirectory_is_not_a_mark_but_a_beats_phase_is() {
        let dir = sandbox("beats");
        write(&dir, "tt.8.impl", "1000200\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        write(&dir.join("beats"), "tt.8.impl", "1000210\n1000220\n");
        write(&dir, "proj.-.beats", "1000100\n");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![
                ("tt".into(), Some("8".into()), "impl".into()),
                ("proj".into(), None, "beats".into()),
            ]
        );
    }

    #[test]
    fn an_unparseable_file_is_skipped_without_failing_its_siblings() {
        let dir = sandbox("unparseable");
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "broken.1.impl", "not a timestamp\n");
        write(&dir, "empty.1.impl", "");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![("tt".into(), Some("8".into()), "impl".into())]
        );
    }

    #[test]
    fn a_missing_or_empty_directory_yields_no_marks() {
        let dir = sandbox("missing");
        assert_eq!(open_marks_in(&dir), Vec::new(), "empty directory");
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(open_marks_in(&dir), Vec::new(), "missing directory");
    }

    #[test]
    fn tt_mark_dir_is_honoured() {
        let _guard = env_guard();
        let dir = sandbox("env");
        write(&dir, "tt.13.impl", "1000200\n");

        let restore = std::env::var_os("TT_MARK_DIR");
        unsafe { std::env::set_var("TT_MARK_DIR", &dir) };
        assert_eq!(mark_dir().as_deref(), Some(dir.as_path()));
        let marks = open_marks();
        match restore {
            Some(value) => unsafe { std::env::set_var("TT_MARK_DIR", value) },
            None => unsafe { std::env::remove_var("TT_MARK_DIR") },
        }

        assert_eq!(
            labels(&marks),
            vec![("tt".into(), Some("13".into()), "impl".into())]
        );
    }

    #[test]
    fn the_default_directory_is_marks_inside_the_app_cache_dir() {
        let cache = || Some(PathBuf::from("cache"));
        assert_eq!(
            resolve_mark_dir(None, cache()),
            Some(PathBuf::from("cache").join("marks"))
        );
        // An empty `TT_MARK_DIR` is no setting at all.
        assert_eq!(
            resolve_mark_dir(Some("".into()), cache()),
            Some(PathBuf::from("cache").join("marks"))
        );
        assert_eq!(
            resolve_mark_dir(Some("elsewhere".into()), cache()),
            Some(PathBuf::from("elsewhere"))
        );
        assert_eq!(
            resolve_mark_dir(None, None),
            None,
            "no cache dir, no default"
        );
        // …but an override alone is enough to run.
        assert_eq!(
            resolve_mark_dir(Some("elsewhere".into()), None),
            Some(PathBuf::from("elsewhere"))
        );
    }

    #[test]
    fn a_row_reads_project_slash_issue_phase_and_drops_a_missing_issue() {
        let dir = sandbox("row-format");
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "loremind.64.plan", "1000300\n");
        write(&dir, "vinge.-.plan", "1000100\n");
        write(&dir, "bare", "1000000\n");

        let labels: Vec<String> = open_marks_in(&dir).iter().map(Mark::label).collect();
        assert_eq!(
            labels,
            vec!["loremind/64 plan", "tt/8 impl", "vinge plan", "bare"]
        );

        let mark = &open_marks_in(&dir)[1];
        assert_eq!(mark.started_at(), mark.start.format("%H:%M").to_string());
        assert_eq!(mark.started_at().len(), 5, "HH:MM");
    }

    #[test]
    fn elapsed_is_derived_from_the_start_and_counts_up() {
        let mark = |seconds: i64| Mark {
            project: "tt".into(),
            issue: Some("14".into()),
            phase: "impl".into(),
            start: DateTime::from_timestamp(seconds, 0)
                .unwrap()
                .with_timezone(&Local),
        };
        let start = 1_000_000_000;
        let now = |offset: i64| {
            DateTime::from_timestamp(start + offset, 0)
                .unwrap()
                .with_timezone(&Local)
        };

        assert_eq!(mark(start).elapsed_at(now(0)), "0m");
        assert_eq!(mark(start).elapsed_at(now(119)), "1m", "seconds truncate");
        assert_eq!(mark(start).elapsed_at(now(120)), "2m");
        assert_eq!(mark(start).elapsed_at(now(60 * 60)), "1h 0m");
        assert_eq!(mark(start).elapsed_at(now(126 * 60)), "2h 6m");
        let m = mark(start);
        assert_eq!(m.elapsed_at(now(60)), "1m");
        assert_eq!(m.elapsed_at(now(61 * 60)), "1h 1m");
        // A clock that stepped backwards must not print a negative age.
        assert_eq!(m.elapsed_at(now(-90)), "0m");
    }

    #[test]
    fn a_lossy_name_degrades_to_a_label_instead_of_an_error() {
        let dir = sandbox("lossy");
        write(&dir, "my.proj.7.impl", "1000400\n");
        write(&dir, "tt.8.impl.v2", "1000300\n");
        write(&dir, "my_proj.-.code_review", "1000200\n");
        write(&dir, "bare", "1000100\n");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![
                ("my".into(), Some("proj.7".into()), "impl".into()),
                ("tt".into(), Some("8.impl".into()), "v2".into()),
                ("my_proj".into(), None, "code_review".into()),
                ("bare".into(), None, String::new()),
            ]
        );
    }

    /// The house-style row `tt agent list` prints.
    #[test]
    fn a_row_is_one_padded_label_column_then_since_and_the_house_duration() {
        let dir = sandbox("rows");
        let now = 1_000_000_000;
        write(
            &dir,
            "timetracker-rs.54.plan",
            &format!("{}\n", now - 15 * 60),
        );
        write(&dir, "vinge.12.impl", &format!("{}\n", now - 45 * 60));

        let marks = open_marks_in(&dir);
        let rows = rows_at(&marks, at(now));
        let since = |offset: i64| at(now - offset).format("%H:%M").to_string();
        assert_eq!(
            rows,
            vec![
                format!("timetracker-rs/54 plan - since {} (0h 15m)", since(15 * 60)),
                format!("vinge/12 impl          - since {} (0h 45m)", since(45 * 60)),
            ]
        );
        let separator = |row: &String| row.find(" - since").unwrap();
        assert_eq!(separator(&rows[0]), separator(&rows[1]));
    }

    #[test]
    fn a_row_reads_the_names_back_the_way_the_reader_does() {
        let dir = sandbox("rows-names");
        let now = 1_000_000_000;
        // The `-` sentinel collapses away entirely — never `vinge/-`.
        write(&dir, "vinge.-.plan", &format!("{}\n", now));
        // Nothing to split: a bare project.
        write(&dir, "solo", &format!("{}\n", now));
        // A phase literally called `last` is a mark.
        write(&dir, "proj.-.last", &format!("{}\n", now));

        let rows = rows_at(&open_marks_in(&dir), at(now));
        assert!(
            rows.iter().any(|row| row.starts_with("vinge plan ")),
            "the - sentinel leaked or the row is missing: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.starts_with("solo ")),
            "a dotless name should list as a bare project: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.starts_with("proj last ")),
            "a phase called `last` should still be listed: {rows:?}"
        );
        assert!(
            !rows.iter().any(|row| row.contains("vinge/-")),
            "the - sentinel leaked into a label: {rows:?}"
        );
        // All three labels are under the minimum column, so `since` lines up.
        for row in &rows {
            assert_eq!(row.find(" - since"), Some(18), "{row:?}");
        }
    }

    #[test]
    fn a_rows_duration_is_the_house_format_and_never_negative() {
        let dir = sandbox("rows-age");
        let now = 1_000_000_000;
        write(&dir, "long.1.impl", &format!("{}\n", now - 126 * 60));
        write(&dir, "short.2.impl", &format!("{}\n", now - 2 * 60));
        // A start in the future reads as `0h 0m`, not as `0h -10m`.
        write(&dir, "future.3.impl", &format!("{}\n", now + 600));

        let rows = rows_at(&open_marks_in(&dir), at(now));
        let ages: Vec<&str> = rows
            .iter()
            .map(|row| &row[row.find('(').unwrap()..])
            .collect();
        assert_eq!(ages, vec!["(0h 0m)", "(0h 2m)", "(2h 6m)"], "{rows:?}");
    }

    #[test]
    fn no_marks_have_no_rows() {
        let dir = sandbox("rows-empty");
        assert_eq!(
            rows_at(&open_marks_in(&dir), Local::now()),
            Vec::<String>::new()
        );
    }

    // --- writer -----------------------------------------------------------

    fn read(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn a_key_is_sanitised_once_and_keeps_the_no_issue_sentinel() {
        assert_eq!(mark_key("tt", "8", "impl"), "tt.8.impl");
        assert_eq!(mark_key("vinge", "-", "plan"), "vinge.-.plan");
        assert_eq!(
            mark_key("my proj", "7", "code/review"),
            "my_proj.7.code_review"
        );
        let dir = Path::new("/marks");
        assert_eq!(
            mark_path(dir, &mark_key("my proj", "7", "impl")),
            PathBuf::from("/marks/my_proj.7.impl")
        );
        assert_eq!(
            beats_path(dir, &mark_key("my proj", "7", "impl")),
            PathBuf::from("/marks/beats/my_proj.7.impl")
        );
    }

    #[test]
    fn begin_writes_the_start_the_reader_reads_back() {
        let dir = sandbox("begin");
        let Begin::Created(start) = begin_in(&dir, "tt", "8", "impl").unwrap() else {
            panic!("a fresh directory should have no mark to find");
        };

        assert_eq!(read(&dir, "tt.8.impl"), format!("{}\n", start.timestamp()));
        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![("tt".into(), Some("8".into()), "impl".into())]
        );
        assert_eq!(open_marks_in(&dir)[0].start.timestamp(), start.timestamp());
    }

    #[test]
    fn a_second_begin_keeps_the_original_start_byte_for_byte() {
        let dir = sandbox("begin-again");
        write(&dir, "tt.8.impl", "1000200\n");

        let again = begin_in(&dir, "tt", "8", "impl").unwrap();
        assert_eq!(
            read(&dir, "tt.8.impl"),
            "1000200\n",
            "the mark was rewritten"
        );
        match again {
            Begin::AlreadyOpen(Some(start)) => assert_eq!(start.timestamp(), 1000200),
            other => panic!("expected an already-open mark, got {other:?}"),
        }

        // An unreadable start still reports the mark as open, never a new start.
        write(&dir, "broken.1.impl", "not a timestamp\n");
        assert_eq!(
            begin_in(&dir, "broken", "1", "impl").unwrap(),
            Begin::AlreadyOpen(None)
        );
    }

    #[test]
    fn each_touch_appends_one_beat_and_leaves_the_mark_alone() {
        let dir = sandbox("touch");
        begin_in(&dir, "tt", "8", "impl").unwrap();
        let mark = read(&dir, "tt.8.impl");

        for _ in 0..3 {
            assert_eq!(touch_in(&dir, "tt", "8", "impl").unwrap(), Touch::Recorded);
        }

        let beats = read(&dir, "beats/tt.8.impl");
        assert_eq!(beats.lines().count(), 3, "one line per touch: {beats:?}");
        assert!(
            beats.lines().all(|line| line.parse::<i64>().is_ok()),
            "every beat is a bare epoch: {beats:?}"
        );
        assert_eq!(read(&dir, "tt.8.impl"), mark, "the mark was rewritten");
        assert!(!dir.join("tt.8.impl.beats").exists());
        assert!(!dir.join("tt.8.impl.last").exists());
    }

    #[test]
    fn touch_on_an_unbegun_phase_writes_nothing_at_all() {
        let dir = sandbox("touch-unbegun");
        assert_eq!(touch_in(&dir, "tt", "8", "impl").unwrap(), Touch::NoMark);
        assert!(
            !dir.join("beats").exists(),
            "no beats directory was created"
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "the directory is untouched"
        );
    }

    #[test]
    fn cancel_clears_the_mark_and_its_beats_but_not_a_sibling_phase() {
        let dir = sandbox("cancel");
        begin_in(&dir, "tt", "8", "impl").unwrap();
        touch_in(&dir, "tt", "8", "impl").unwrap();
        begin_in(&dir, "other", "9", "plan").unwrap();
        touch_in(&dir, "other", "9", "plan").unwrap();

        cancel_in(&dir, "tt", "8", "impl").unwrap();

        assert!(!dir.join("tt.8.impl").exists());
        assert!(!dir.join("beats/tt.8.impl").exists());
        assert!(
            dir.join("beats").is_dir(),
            "the beats directory itself stays"
        );
        assert!(
            dir.join("other.9.plan").is_file(),
            "a sibling mark was cleared"
        );
        assert!(dir.join("beats/other.9.plan").is_file());

        cancel_in(&dir, "tt", "8", "impl").unwrap();
        cancel_in(&dir, "never", "1", "begun").unwrap();
    }

    // --- what `end` measures ----------------------------------------------

    #[test]
    fn a_phase_with_no_beats_is_one_unvouched_stretch() {
        // No beats: the whole span is one silence, judged like any other.
        assert_eq!(gaps_over(0, 46 * 60, &[], 45), vec![(0, 46 * 60)]);
        assert_eq!(gaps_over(0, 10 * 60, &[], 45), vec![]);
    }

    #[test]
    fn silence_before_the_first_beat_is_a_gap() {
        let start = 1_000_000;
        let first = start + 60 * 60;
        let gaps = gaps_over(start, first + 300, &[first], 45);
        assert_eq!(gaps, vec![(start, first)]);
    }

    #[test]
    fn silence_after_the_last_beat_is_a_gap() {
        let start = 1_000_000;
        let beat = start + 300;
        let end = beat + 60 * 60;
        assert_eq!(gaps_over(start, end, &[beat], 45), vec![(beat, end)]);
    }

    #[test]
    fn a_beat_that_does_not_advance_the_sequence_is_skipped() {
        let start = 1_000_000;
        let beat = start + 10 * 60;
        let end = start + 70 * 60;
        // None of the skipped beats advances `prev`, so the hole after `beat`
        // is still measured from `beat` itself.
        let beats = [beat, beat, beat - 60, end + 600, end];
        assert_eq!(gaps_over(start, end, &beats, 45), vec![(beat, end)]);
    }

    #[test]
    fn the_threshold_is_floor_minutes_and_strictly_greater() {
        let start = 1_000_000;
        // 45m59s floors to 45, which is not *greater* than 45.
        assert_eq!(gaps_over(start, start + 45 * 60 + 59, &[], 45), vec![]);
        // 46m00s is.
        let end = start + 46 * 60;
        assert_eq!(gaps_over(start, end, &[], 45), vec![(start, end)]);
    }

    #[test]
    fn two_holes_come_back_in_chronological_order() {
        let start = 1_000_000;
        let beats = [
            start + 10 * 60,
            start + 70 * 60,
            start + 80 * 60,
            start + 140 * 60,
        ];
        let end = start + 150 * 60;
        assert_eq!(
            gaps_over(start, end, &beats, 45),
            vec![(beats[0], beats[1]), (beats[2], beats[3])]
        );
    }

    #[test]
    fn a_phase_reads_back_its_start_beats_and_last_line() {
        let dir = sandbox("phase-read");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        // `end` measures to the last beat recorded, not the highest one.
        write(&dir, "beats/proj.7.impl", "1000600\n1002000\n1001200\n");

        let phase = read_phase_in(&dir, "proj", "7", "impl").unwrap().unwrap();
        assert_eq!(phase.started, 1_000_000);
        assert_eq!(phase.beats, vec![1_000_600, 1_002_000, 1_001_200]);
        assert_eq!(phase.ended, Some(1_001_200));
    }

    #[test]
    fn a_line_that_is_not_a_bare_timestamp_is_dropped() {
        let dir = sandbox("phase-garbage");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        write(
            &dir,
            "beats/proj.7.impl",
            "\nnope\n1000600 note\n-5\nx1000700\n",
        );

        let phase = read_phase_in(&dir, "proj", "7", "impl").unwrap().unwrap();
        // `1000600 note` counts as a beat, but is no instant to measure to.
        assert_eq!(phase.beats, vec![1_000_600]);
        assert_eq!(phase.ended, None);
    }

    #[test]
    fn an_unmarked_phase_reads_back_as_nothing() {
        let dir = sandbox("phase-unmarked");
        assert_eq!(read_phase_in(&dir, "proj", "7", "impl").unwrap(), None);
    }

    #[test]
    fn a_mark_that_is_not_a_timestamp_is_an_error_naming_the_path() {
        let dir = sandbox("phase-bad-mark");
        write(&dir, "proj.7.impl", "not a timestamp\n");

        let err = read_phase_in(&dir, "proj", "7", "impl").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("proj.7.impl"),
            "the error names the path: {err}"
        );
    }
}
