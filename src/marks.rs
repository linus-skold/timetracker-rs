//! Reader and writer for the agent layer's open phase marks: one file per phase at
//! `<mark dir>/<project>.<issue>.<phase>[.<agent>]` holding a unix-seconds start,
//! heartbeats in `beats/<key>`, an unfinished close in `closing/<key>`. Each segment is
//! sanitised on its own, so a key holds exactly the two or three dots that join it.

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
    /// `None` for a mark with no `--agent` label.
    pub agent: Option<String>,
    pub start: DateTime<Local>,
}

impl Mark {
    /// `project/issue phase:agent`, dropping the `/issue` for the `-` sentinel and the
    /// `:agent` for an unlabelled mark.
    pub fn label(&self) -> String {
        let subject = match &self.issue {
            Some(issue) => format!("{}/{}", self.project, issue),
            None => self.project.clone(),
        };
        if self.phase.is_empty() {
            return subject;
        }
        match &self.agent {
            Some(agent) => format!("{} {}:{}", subject, self.phase, agent),
            None => format!("{} {}", subject, self.phase),
        }
    }

    /// The segments that address this mark, with the `-` sentinel for a missing issue.
    pub fn as_key(&self) -> MarkRef<'_> {
        MarkRef {
            project: &self.project,
            issue: self.issue.as_deref().unwrap_or("-"),
            phase: &self.phase,
            agent: self.agent.as_deref(),
        }
    }

    /// The clock time the mark was made, `HH:MM`.
    pub fn started_at(&self) -> String {
        self.start.format("%H:%M").to_string()
    }

    /// How long this mark has been open, as `2m` or `2h 6m`.
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

    /// How long this mark has been open in the house `{h}h {m}m` format, clamped
    /// so a start in the future prints `0h 0m`.
    pub fn age_at(&self, now: DateTime<Local>) -> String {
        duration::format((now - self.start).max(TimeDelta::zero()))
    }
}

/// The segments that address one mark. `issue` carries the `-` sentinel literally,
/// never `None`; `agent` is the optional fourth key segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkRef<'a> {
    pub project: &'a str,
    pub issue: &'a str,
    pub phase: &'a str,
    pub agent: Option<&'a str>,
}

impl MarkRef<'_> {
    /// The `<project> <issue> <phase> [--agent <label>]` words that address this mark on
    /// a `tt agent` command line.
    pub fn args(&self) -> String {
        let mut args = format!("{} {} {}", self.project, self.issue, self.phase);
        if let Some(agent) = self.agent {
            args.push_str(&format!(" --agent {agent}"));
        }
        args
    }
}

/// The pair of silences `tt agent end` and the audit judge by — see
/// [`crate::audit::thresholds`]. `gap` bounds a silence around a bare beat;
/// `unvouched` is the whole-span grace for a phase the model never vouched for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    pub gap: i64,
    pub unvouched: i64,
}

/// Grace in minutes past the last evidence of life: gap with it, unvouched without, plus one.
pub fn grace_minutes(lively: bool, thresholds: Thresholds) -> i64 {
    let allowed = if lively {
        thresholds.gap
    } else {
        thresholds.unvouched
    };
    allowed + 1
}

/// One open mark plus the instant [`crate::agent`]'s `end` would measure it to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub mark: Mark,
    /// The beats file's **last** line's timestamp whatever tag follows it, never its
    /// largest beat.
    pub last_seen: Option<DateTime<Local>>,
    /// Whether the file holds any bare line: an automatic beat is presence, not work.
    pub vouched: bool,
}

impl Lease {
    /// The **first** instant this mark no longer vouches: [`grace_minutes`] past
    /// its last beat, or past `mark.start` when it never beat.
    pub fn expires_at(&self, thresholds: Thresholds) -> DateTime<Local> {
        let since = self.last_seen.unwrap_or(self.mark.start);
        since + TimeDelta::minutes(grace_minutes(self.last_seen.is_some(), thresholds))
    }

    /// Whether the lease has run out by `now`: the one boundary the coverage bound
    /// and the `[stale]` marker share.
    pub fn is_expired_at(&self, now: DateTime<Local>, thresholds: Thresholds) -> bool {
        now >= self.expires_at(thresholds)
    }

    /// The mark's last heartbeat as `HH:MM`, or `never` when it has none.
    pub fn last_seen_at(&self) -> String {
        match self.last_seen {
            Some(seen) => seen.format("%H:%M").to_string(),
            None => "never".to_string(),
        }
    }

    /// The `tt agent end` line that logs this mark's work and clears it, or
    /// [`remove_by_hand`]'s note for a mark no command can address. `--trim` only
    /// for a phase the model vouched for; the project printed is the mark's own
    /// sanitised name, which is what [`same_project`] accepts.
    pub fn close_command(&self) -> String {
        if !closable(&self.mark) {
            return remove_by_hand(&self.mark);
        }
        let tail = if self.vouched { "--trim" } else { "<minutes>" };
        format!(
            "tt agent end {} \"<summary>\" {}",
            self.mark.as_key().args(),
            tail
        )
    }
}

/// Read one mark's heartbeat file to pair it with its last-seen instant. Unlike
/// [`open_marks_in`] this reads `beats/`, which no mark-directory mtime reflects.
pub fn lease_in(dir: &Path, mark: &Mark) -> Lease {
    let key = mark_key(mark.as_key());
    let body = fs::read_to_string(beats_path(dir, &key)).unwrap_or_default();
    // The last *parseable* line, whatever tag follows its timestamp, never the
    // largest beat.
    let last_seen = body
        .lines()
        .rev()
        .find_map(beat_of)
        .and_then(crate::time::instant);
    Lease {
        mark: mark.clone(),
        last_seen,
        vouched: body.lines().any(all_digits),
    }
}

/// Every open mark in `dir` with its last-seen instant, newest first.
pub fn open_leases_in(dir: &Path) -> Vec<Lease> {
    open_marks_in(dir)
        .iter()
        .map(|mark| lease_in(dir, mark))
        .collect()
}

/// Narrowest label column `tt agent list` will use; `src/tui/render/surfaces.rs`
/// keeps its own copy.
const LABEL_WIDTH: usize = 18;

/// The rows `tt agent list` prints, without the CLI's indent. An expired row is
/// marked `[stale]` and followed by an indented [`Lease::close_command`].
pub fn rows(leases: &[Lease], thresholds: Thresholds) -> Vec<String> {
    rows_at(leases, Local::now(), thresholds)
}

/// The `now`-taking half of [`rows`], for tests.
pub fn rows_at(leases: &[Lease], now: DateTime<Local>, thresholds: Thresholds) -> Vec<String> {
    // One column for the whole list, so the start times line up.
    let width = leases
        .iter()
        .map(|lease| lease.mark.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(LABEL_WIDTH);

    let mut rows = Vec::new();
    for lease in leases {
        let label = lease.mark.label();
        let pad = " ".repeat(width.saturating_sub(label.chars().count()));
        let stale = lease.is_expired_at(now, thresholds);
        rows.push(format!(
            "{}{} - since {} ({}) last seen {}{}",
            label,
            pad,
            lease.mark.started_at(),
            lease.mark.age_at(now),
            lease.last_seen_at(),
            if stale { " [stale]" } else { "" }
        ));
        if stale {
            rows.push(format!("  {}", lease.close_command()));
        }
    }
    rows
}

/// The directory the marks live in: `$TT_MARK_DIR` when set and non-empty, else
/// `marks` inside this app's cache directory.
pub fn mark_dir() -> Option<PathBuf> {
    resolve_mark_dir(std::env::var_os("TT_MARK_DIR"), crate::paths::cache_dir())
}

/// The env-free half of [`mark_dir`], taking the resolved cache root.
fn resolve_mark_dir(mark_dir: Option<OsString>, cache: Option<PathBuf>) -> Option<PathBuf> {
    crate::paths::env_or(mark_dir, Some(cache?.join("marks")))
}

/// Every open mark, newest first. A missing mark directory is an empty list.
pub fn open_marks() -> Vec<Mark> {
    match mark_dir() {
        Some(dir) => open_marks_in(&dir),
        None => Vec::new(),
    }
}

/// Every open mark in `dir`, newest first; a bad file is skipped, never fatal.
/// **Read no heartbeat here** — callers refresh on the directory's mtime, which a
/// beat inside `beats/` does not change. A subdirectory is skipped by the
/// file-type filter, never by its name.
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
        b.start.cmp(&a.start).then_with(|| {
            (&a.project, &a.issue, &a.phase, &a.agent)
                .cmp(&(&b.project, &b.issue, &b.phase, &b.agent))
        })
    });
    marks
}

/// The instant a mark file holds, or `None`: the one place the body is interpreted.
fn read_start(path: &Path) -> Option<DateTime<Local>> {
    let contents = fs::read_to_string(path).ok()?;
    let seconds: i64 = contents.trim().parse().ok()?;
    crate::time::instant(seconds)
}

/// Parse one mark file, or `None` if it is not one.
fn read_mark(dir: &Path, name: &OsString) -> Option<Mark> {
    let start = read_start(&dir.join(name))?;
    let (project, issue, phase, agent) = split_key(name.to_str()?);
    Some(Mark {
        project: crate::naming::normalize_name(&project),
        issue,
        phase,
        agent,
        start,
    })
}

/// Split a mark filename back into its segments. Every key [`mark_key`] wrote holds
/// three or four of them; a longer hand-made name falls back to first dot and last dot,
/// which [`closable`] then rejects.
fn split_key(name: &str) -> (String, Option<String>, String, Option<String>) {
    let issue = |raw: &str| (raw != "-").then(|| raw.to_string());
    let own = str::to_string;
    let segments: Vec<&str> = name.split('.').collect();

    match segments.as_slice() {
        // One `.`: an issue-less `<project>.<phase>`, as `-` produces.
        [project, phase] => (own(project), None, own(phase), None),
        [project, mid, phase] => (own(project), issue(mid), own(phase), None),
        [project, mid, phase, agent] => (own(project), issue(mid), own(phase), Some(own(agent))),
        // More segments than a key has: the first dot and the last one, as before.
        [project, mid @ .., phase] => (own(project), issue(&mid.join(".")), own(phase), None),
        // Nothing to split: the whole name is the project.
        _ => (own(name), None, String::new(), None),
    }
}

// --- writer ---------------------------------------------------------------
//
// Nothing here locks: `create_new` plus `O_APPEND` cover the races.

/// What [`begin_in`] found: a mark it created, or one that was already open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Begin {
    /// The mark file was created, holding this instant.
    Created(DateTime<Local>),
    /// A mark was already open and is left byte-identical. `None` when its
    /// contents are not a timestamp, which the caller renders as `??:??`.
    AlreadyOpen(Option<DateTime<Local>>),
    /// A close for this phase is unfinished, so nothing was written at all.
    Closing,
}

/// What [`touch_in`] did: recorded a heartbeat, or refused for want of a mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    Recorded,
    /// No mark file for this phase, so no beats file was created either.
    NoMark,
}

/// The sanitised filename for one mark: `<project>.<issue>.<phase>`, plus `.<agent>`
/// when it is labelled, with the `-` sentinel written literally. Sanitise the segments,
/// never the joined string — [`crate::paths::sanitise_key`] maps `.` to `-`. Build every
/// path from this key.
pub fn mark_key(mark: MarkRef) -> String {
    let segment = crate::paths::sanitise_key;
    let mut key = format!(
        "{}.{}.{}",
        project_key(mark.project),
        segment(mark.issue),
        segment(mark.phase)
    );
    if let Some(agent) = mark.agent {
        key.push('.');
        key.push_str(&segment(agent));
    }
    key
}

/// `folder/key`, or a file from before projects were lowercased whose key
/// differs only in the case of the project segment. Never renames the file.
fn path_in(folder: PathBuf, key: &str) -> PathBuf {
    let exact = folder.join(key);
    if exact.exists() {
        return exact;
    }
    let same_but_project_case = |name: &str| match (name.split_once('.'), key.split_once('.')) {
        (Some((old, old_rest)), Some((new, new_rest))) => {
            old.to_lowercase() == new && old_rest == new_rest
        }
        _ => false,
    };
    fs::read_dir(&folder)
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.file_name().to_str().is_some_and(same_but_project_case))
        .map(|e| e.path())
        .unwrap_or(exact)
}

pub fn mark_path(dir: &Path, key: &str) -> PathBuf {
    path_in(dir.to_path_buf(), key)
}

/// The heartbeat file for `key` inside `dir`: a `beats/` **subdirectory**, never
/// a `<mark>.<suffix>` sibling.
pub fn beats_path(dir: &Path, key: &str) -> PathBuf {
    path_in(dir.join("beats"), key)
}

/// The in-progress-close sentinel for `key` inside `dir`: a `closing/`
/// **subdirectory** entry, so the mark listing's file-type filter skips it.
pub fn closing_path(dir: &Path, key: &str) -> PathBuf {
    path_in(dir.join("closing"), key)
}

/// Open a mark for one phase, or report the one already open. `create_new` keeps
/// the check and the write atomic, and an existing mark is never rewritten.
pub fn begin_in(dir: &Path, mark: MarkRef) -> io::Result<Begin> {
    let key = mark_key(mark);
    let path = mark_path(dir, &key);
    fs::create_dir_all(dir)?;

    if closing_path(dir, &key).exists() {
        return Ok(Begin::Closing);
    }

    let start = Local::now();
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            writeln!(file, "{}", start.timestamp())?;
            // `create_new` succeeded, so any beats here are a cancel's leftovers.
            remove_if_present(&beats_path(dir, &key))?;
            Ok(Begin::Created(start))
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Ok(Begin::AlreadyOpen(read_start(&path)))
        }
        Err(err) => Err(err),
    }
}

/// Append one heartbeat for a phase that is already marked; appended, never
/// overwritten. A phase nobody began records nothing at all.
pub fn touch_in(dir: &Path, mark: MarkRef) -> io::Result<Touch> {
    let key = mark_key(mark);
    if !mark_path(dir, &key).is_file() {
        return Ok(Touch::NoMark);
    }

    append_beat(dir, &key, None)?;
    Ok(Touch::Recorded)
}

/// The one writer of a beat line: `<epoch>` bare, or `<epoch> <tag>`. Only a bare line
/// anchors what `end` bills — never tag a `touch`, never leave an automatic beat bare.
fn append_beat(dir: &Path, key: &str, tag: Option<&str>) -> io::Result<()> {
    let beats = beats_path(dir, key);
    if let Some(parent) = beats.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().append(true).create(true).open(&beats)?;
    match tag {
        Some(tag) => writeln!(file, "{} {}", Local::now().timestamp(), tag)?,
        None => writeln!(file, "{}", Local::now().timestamp())?,
    }
    Ok(())
}

/// Append one automatic `hook` beat to every open mark in `dir` whose project
/// matches, case-insensitively; a mark whose beats cannot be written is skipped.
/// Tagged, never bare, or the beat moves what `end` measures.
pub fn touch_project_in(dir: &Path, project: &str) {
    for mark in open_marks_in(dir) {
        if !owned_by(&mark, project) {
            continue;
        }
        let _ = append_beat(dir, &mark_key(mark.as_key()), Some("hook"));
    }
}

/// Whether two project names name the same project, compared as whole sanitised
/// segments, case-insensitively. Sanitise both sides, never one. Sanitisation is
/// not injective: `my proj` and a real `my_proj` cannot be told apart here.
pub fn same_project(a: &str, b: &str) -> bool {
    project_key(a) == project_key(b)
}

/// The key [`same_project`] compares by, for grouping projects rather than
/// pairing them.
pub fn project_key(project: &str) -> String {
    crate::paths::sanitise_key(project).to_ascii_lowercase()
}

/// Whether `project` owns `mark`.
pub fn owned_by(mark: &Mark, project: &str) -> bool {
    same_project(&mark.project, project)
}

/// Whether any command can address this mark: its segments have to survive [`mark_key`]
/// and [`split_key`] unchanged, or `end` and `cancel` build a key that names no file.
pub fn closable(mark: &Mark) -> bool {
    split_key(&mark_key(mark.as_key()))
        == (
            mark.project.clone(),
            mark.issue.clone(),
            mark.phase.clone(),
            mark.agent.clone(),
        )
}

/// The one line offered for a mark [`closable`] rejects: what to log and which
/// file to delete, carrying the project token as every mark line does.
pub fn remove_by_hand(mark: &Mark) -> String {
    let key = mark.as_key();
    let name = match key.agent {
        Some(agent) => format!("{}.{}.{}.{}", key.project, key.issue, key.phase, agent),
        None => format!("{}.{}.{}", key.project, key.issue, key.phase),
    };
    format!(
        "{} cannot be closed — log it with tt agent item {} {} {} \"<summary>\" <minutes> and remove the file by hand",
        name, key.project, key.issue, key.phase
    )
}

/// Every mark in `dir` no command can close, as its own note.
pub fn unclosable_in(dir: &Path) -> Vec<String> {
    open_marks_in(dir)
        .iter()
        .filter(|mark| !closable(mark))
        .map(remove_by_hand)
        .collect()
}

/// Record that a close for one phase is under way, holding the mark's start so the
/// file names its own span. An existing sentinel is overwritten; [`is_closing_in`] refuses.
pub fn start_closing_in(dir: &Path, mark: MarkRef) -> io::Result<()> {
    let key = mark_key(mark);
    let path = closing_path(dir, &key);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // No mark on the explicit-minutes path, which reads no timestamps.
    let start = read_start(&mark_path(dir, &key)).unwrap_or_else(Local::now);
    fs::write(path, format!("{}\n", start.timestamp()))
}

/// Whether a close for one phase was started and never finished.
pub fn is_closing_in(dir: &Path, mark: MarkRef) -> bool {
    closing_path(dir, &mark_key(mark)).exists()
}

/// Drop only the `closing/` sentinel, for a close that failed before it logged.
pub fn abandon_close_in(dir: &Path, mark: MarkRef) -> io::Result<()> {
    remove_if_present(&closing_path(dir, &mark_key(mark)))
}

/// Remove one path, treating an absent one as done.
fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Drop a mark, its `beats/` entry and its `closing/` sentinel, each allowed to be
/// absent. **Beats go first**, or a failure part-way leaves beats a later phase
/// would read back as its own. The `beats/` and `closing/` directories stay.
pub fn cancel_in(dir: &Path, mark: MarkRef) -> io::Result<()> {
    let key = mark_key(mark);
    for path in [
        beats_path(dir, &key),
        mark_path(dir, &key),
        closing_path(dir, &key),
    ] {
        remove_if_present(&path)?;
    }
    Ok(())
}

// --- what `end` measures --------------------------------------------------
//
// `agent.rs` gets a [`Phase`] and a list of gaps, and never learns the format.

/// One marked phase as `end` needs to read it back, all in unix seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub started: i64,
    /// Every **bare** heartbeat, **in file order**; a hook-beaten phase reads back
    /// as a beat-less one. Never sort or dedup: [`gaps_over`] judges that.
    pub beats: Vec<i64>,
}

impl Phase {
    /// Where `tt agent end` measures to: the last bare heartbeat, or `now` for a
    /// beat-less phase. Clamped, so a heartbeat behind the mark is a zero-length phase.
    pub fn measured_end(&self, now: i64) -> i64 {
        self.beats.last().copied().unwrap_or(now).max(self.started)
    }

    /// The silences `tt agent end` flags up to `ended`. An unvouched phase is one
    /// whole-span silence against the longer grace.
    pub fn flagged_gaps(&self, ended: i64, thresholds: Thresholds) -> Vec<(i64, i64)> {
        if self.beats.is_empty() {
            gaps_over(self.started, ended, &[], thresholds.unvouched)
        } else {
            gaps_over(self.started, ended, &self.beats, thresholds.gap)
        }
    }
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
pub fn read_phase_in(dir: &Path, mark: MarkRef) -> io::Result<Option<Phase>> {
    let key = mark_key(mark);
    let path = mark_path(dir, &key);
    if !path.is_file() {
        return Ok(None);
    }
    let started = read_start(&path)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} does not hold a unix timestamp", path.display()),
            )
        })?
        .timestamp();

    let source = beats_path(dir, &key);

    // No beats file leaves the single start→end interval to judge.
    let body = fs::read_to_string(&source).unwrap_or_default();
    // Bare lines only: a tagged beat is presence, not work.
    let beats = body
        .lines()
        .filter(|line| all_digits(line))
        .filter_map(|line| line.parse().ok())
        .collect();

    Ok(Some(Phase { started, beats }))
}

/// Every stretch of silence longer than `threshold_minutes`, as chronological
/// `(from, to)` epoch pairs. The sequence judged is `start, beats…, end`, so a
/// phase with no beats is one stretch across its whole span.
///
/// A beat that does not advance the sequence is skipped without advancing `prev`.
/// The threshold test is `(beat - prev) / 60 > threshold`: **integer-floor
/// minutes, strictly greater**.
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
    /// Serialises the one test that repoints `TT_MARK_DIR`; env is process-wide.
    use crate::storage::env_guard;

    /// The shipped defaults, stated here rather than read from config.
    const HOUSE: Thresholds = Thresholds {
        gap: 45,
        unvouched: 120,
    };

    /// One unlabelled mark, `<project>.<issue>.<phase>`.
    fn key<'a>(project: &'a str, issue: &'a str, phase: &'a str) -> MarkRef<'a> {
        MarkRef {
            project,
            issue,
            phase,
            agent: None,
        }
    }

    /// The same mark under one agent label.
    fn labelled<'a>(
        project: &'a str,
        issue: &'a str,
        phase: &'a str,
        agent: &'a str,
    ) -> MarkRef<'a> {
        MarkRef {
            agent: Some(agent),
            ..key(project, issue, phase)
        }
    }

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

    #[test]
    fn touch_beats_every_open_mark_of_that_project_and_nothing_else() {
        let dir = sandbox("touch-project");
        write(&dir, "a.7.impl", "1000100\n");
        // The `-` sentinel has to round-trip through the key to be beaten.
        write(&dir, "a.-.plan", "1000200\n");
        // A labelled mark is beaten under its own four-segment key.
        write(&dir, "a.7.review.code", "1000250\n");
        write(&dir, "b.9.impl", "1000300\n");

        touch_project_in(&dir, "A");

        for key in ["a.7.impl", "a.-.plan", "a.7.review.code"] {
            let body = fs::read_to_string(beats_path(&dir, key)).unwrap();
            assert_eq!(body.lines().count(), 1, "{key}");
        }
        assert!(!beats_path(&dir, "b.9.impl").exists());
        assert!(!beats_path(&dir, "a.7.review").exists());
        assert_eq!(fs::read_dir(dir.join("beats")).unwrap().count(), 3);
    }

    #[test]
    fn a_mixed_case_mark_file_is_found_and_cancelled_under_the_lowercase_key() {
        let dir = sandbox("legacy-case-paths");
        write(&dir, "Tt.7.impl", "1000100\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::create_dir_all(dir.join("closing")).unwrap();
        write(&dir.join("beats"), "Tt.7.impl", "1000200\n");
        write(&dir.join("closing"), "Tt.7.impl", "1000300\n");

        // On a case-insensitive file system the lowercase path finds it directly.
        assert!(mark_path(&dir, "tt.7.impl").is_file());
        assert!(beats_path(&dir, "tt.7.impl").is_file());
        assert!(closing_path(&dir, "tt.7.impl").is_file());
        assert!(
            !mark_path(&dir, "tt.8.impl").exists(),
            "a different issue is not matched"
        );

        cancel_in(&dir, key("tt", "7", "impl")).unwrap();
        assert!(!dir.join("Tt.7.impl").exists());
        assert!(!dir.join("beats/Tt.7.impl").exists());
        assert!(!dir.join("closing/Tt.7.impl").exists());
    }

    #[test]
    fn a_mark_read_back_names_its_project_in_lowercase() {
        let dir = sandbox("read-mixed-case");
        write(&dir, "Project1.7.impl", "1000100\n");

        let marks = open_marks_in(&dir);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].project, "project1");
        assert_eq!(marks[0].issue.as_deref(), Some("7"));
    }

    #[test]
    fn an_automatic_beat_is_tagged_so_it_enters_no_judgement() {
        let dir = sandbox("touch-project-tag");
        write(&dir, "a.7.impl", "1000100\n");

        touch_project_in(&dir, "a");

        let body = fs::read_to_string(beats_path(&dir, "a.7.impl")).unwrap();
        assert!(body.trim_end().ends_with(" hook"), "{body}");
        let phase = read_phase_in(&dir, key("a", "7", "impl")).unwrap().unwrap();
        assert!(phase.beats.is_empty(), "a tagged beat is not a beat here");
    }

    #[test]
    fn a_beat_matches_the_mark_by_its_sanitised_key() {
        let dir = sandbox("touch-project-lossy");
        write(&dir, "my_proj.7.impl", "1000100\n");
        write(&dir, "app-web.7.impl", "1000200\n");

        touch_project_in(&dir, "my proj");
        assert!(beats_path(&dir, "my_proj.7.impl").is_file());
        assert!(!beats_path(&dir, "app-web.7.impl").exists());

        touch_project_in(&dir, "app.web");
        assert!(beats_path(&dir, "app-web.7.impl").is_file());
    }

    #[test]
    fn a_dot_related_project_does_not_cross_beat() {
        let dir = sandbox("touch-project-segment");
        write(&dir, "app.7.impl", "1000100\n");
        write(&dir, "app-web.7.impl", "1000200\n");

        touch_project_in(&dir, "app");
        assert!(beats_path(&dir, "app.7.impl").is_file());
        assert!(
            !beats_path(&dir, "app-web.7.impl").exists(),
            "app beat app.web's mark"
        );

        let _ = fs::remove_file(beats_path(&dir, "app.7.impl"));
        touch_project_in(&dir, "app.web");
        assert!(beats_path(&dir, "app-web.7.impl").is_file());
        assert!(
            !beats_path(&dir, "app.7.impl").exists(),
            "app.web beat app's mark"
        );
    }

    #[test]
    fn a_dotted_issue_stays_inside_its_own_segment() {
        let dir = sandbox("touch-project-dotted-issue");
        begin_in(&dir, key("app", "1.2", "impl")).unwrap();
        assert!(mark_path(&dir, "app.1-2.impl").is_file());

        touch_project_in(&dir, "app");
        assert!(beats_path(&dir, "app.1-2.impl").is_file());
        assert!(owned_by(&open_marks_in(&dir)[0], "app"));
    }

    fn at(seconds: i64) -> DateTime<Local> {
        crate::time::instant(seconds).unwrap()
    }

    #[test]
    fn the_grace_is_the_gap_with_evidence_of_life_and_the_whole_span_without() {
        assert_eq!(grace_minutes(true, HOUSE), 46);
        assert_eq!(grace_minutes(false, HOUSE), 121);
    }

    #[test]
    fn a_mark_that_never_beat_expires_one_minute_past_the_unvouched_grace() {
        let dir = sandbox("lease-unvouched");
        write(&dir, "proj.7.impl", "1000000\n");
        let leases = open_leases_in(&dir);
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].last_seen, None);
        assert_eq!(leases[0].expires_at(HOUSE), at(1_000_000 + 121 * 60));
    }

    #[test]
    fn a_beaten_mark_expires_one_minute_past_the_gap_after_its_last_beat() {
        let dir = sandbox("lease-beaten");
        write(&dir, "proj.-.plan", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(beats_path(&dir, "proj.-.plan"), "1000300\n1000600\n").unwrap();
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert_eq!(lease.last_seen, Some(at(1_000_600)));
        assert_eq!(lease.expires_at(HOUSE), at(1_000_600 + 46 * 60));
    }

    #[test]
    fn a_tagged_beat_is_still_a_last_seen_for_liveness() {
        let dir = sandbox("lease-annotated");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(beats_path(&dir, "proj.7.impl"), "1000600 hook\n").unwrap();
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert_eq!(lease.last_seen, Some(at(1_000_600)));
        assert_eq!(lease.expires_at(HOUSE), at(1_000_600 + 46 * 60));
    }

    #[test]
    fn an_empty_beats_file_takes_the_unvouched_grace() {
        let dir = sandbox("lease-empty-beats");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(beats_path(&dir, "proj.7.impl"), "\n").unwrap();
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert_eq!(lease.last_seen, None);
        assert_eq!(lease.expires_at(HOUSE), at(1_000_000 + 121 * 60));
    }

    #[test]
    fn only_a_bare_beat_vouches_for_a_lease() {
        let dir = sandbox("lease-vouch");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();

        fs::write(beats_path(&dir, "proj.7.impl"), "1000600 hook\n").unwrap();
        let hooked = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert!(!hooked.vouched);
        assert!(hooked.close_command().ends_with("<minutes>"));

        fs::write(beats_path(&dir, "proj.7.impl"), "1000600\n1000900 hook\n").unwrap();
        let touched = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert!(touched.vouched, "a bare beat anywhere in the file vouches");
        assert!(touched.close_command().ends_with("--trim"));
    }

    #[test]
    fn a_mark_with_more_segments_than_a_key_has_prints_no_close_line() {
        let dir = sandbox("legacy-dotted");
        write(&dir, "app.web.7.impl.v2", "1000000\n");
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);

        assert!(!closable(&lease.mark));
        let note = lease.close_command();
        assert!(!note.contains("tt agent end"), "{note}");
        assert!(note.contains("app.web.7.impl.v2"), "{note}");
        assert!(
            note.contains("tt agent item app web.7.impl v2"),
            "the note has to name the work to log and its project: {note}"
        );
    }

    #[test]
    fn a_healthy_mark_is_closable_in_both_forms() {
        let dir = sandbox("legacy-healthy");
        write(&dir, "app.7.impl", "1000000\n");
        write(&dir, "app.-.plan", "1000000\n");
        for mark in open_marks_in(&dir) {
            assert!(closable(&mark), "{:?}", mark);
        }
    }

    #[test]
    fn only_the_marks_no_command_can_close_get_a_note() {
        let dir = sandbox("legacy-notes");
        write(&dir, "app.web.7.impl.v2", "1000000\n");
        write(&dir, "app.7.impl", "1000000\n");
        write(&dir, "app.7.impl.reviewer", "1000000\n");
        let notes = unclosable_in(&dir);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("app.web.7.impl.v2"), "{notes:?}");
    }

    #[test]
    fn the_last_beats_line_wins_over_the_largest_beat() {
        let dir = sandbox("lease-last-line");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(beats_path(&dir, "proj.7.impl"), "1009000\n1000600\n").unwrap();
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert_eq!(lease.last_seen, Some(at(1_000_600)));
    }

    #[test]
    fn a_trailing_blank_line_does_not_hide_the_last_beat() {
        let dir = sandbox("lease-trailing-blank");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(beats_path(&dir, "proj.7.impl"), "1000300\n1000600\n\n").unwrap();
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert_eq!(lease.last_seen, Some(at(1_000_600)));
    }

    #[test]
    fn a_trailing_junk_line_does_not_hide_the_last_beat() {
        let dir = sandbox("lease-trailing-junk");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(beats_path(&dir, "proj.7.impl"), "1000600\nnot-a-beat\n").unwrap();
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);
        assert_eq!(lease.last_seen, Some(at(1_000_600)));
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
        assert_eq!(
            resolve_mark_dir(None, None),
            None,
            "no cache dir, no default"
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
            agent: None,
            start: crate::time::instant(seconds).unwrap(),
        };
        let start = 1_000_000_000;
        let now = |offset: i64| crate::time::instant(start + offset).unwrap();

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
        write(&dir, "my.proj.7.impl.v2", "1000400\n");
        write(&dir, "my_proj.-.code_review", "1000200\n");
        write(&dir, "bare", "1000100\n");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![
                ("my".into(), Some("proj.7.impl".into()), "v2".into()),
                ("my_proj".into(), None, "code_review".into()),
                ("bare".into(), None, String::new()),
            ]
        );
    }

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

        let rows = rows_at(&open_leases_in(&dir), at(now), HOUSE);
        let since = |offset: i64| at(now - offset).format("%H:%M").to_string();
        assert_eq!(
            rows,
            vec![
                format!(
                    "timetracker-rs/54 plan - since {} (0h 15m) last seen never",
                    since(15 * 60)
                ),
                format!(
                    "vinge/12 impl          - since {} (0h 45m) last seen never",
                    since(45 * 60)
                ),
            ]
        );
        let separator = |row: &String| row.find(" - since").unwrap();
        assert_eq!(separator(&rows[0]), separator(&rows[1]));
    }

    #[test]
    fn a_row_reads_the_names_back_the_way_the_reader_does() {
        let dir = sandbox("rows-names");
        let now = 1_000_000_000;
        write(&dir, "vinge.-.plan", &format!("{}\n", now));
        write(&dir, "solo", &format!("{}\n", now));
        write(&dir, "proj.-.last", &format!("{}\n", now));

        let rows = rows_at(&open_leases_in(&dir), at(now), HOUSE);
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
        write(&dir, "future.3.impl", &format!("{}\n", now + 600));

        // A grace wide enough that no row reads as stale, so each keeps its age.
        let rows = rows_at(
            &open_leases_in(&dir),
            at(now),
            Thresholds {
                gap: 45,
                unvouched: 10_000,
            },
        );
        let ages: Vec<&str> = rows
            .iter()
            .map(|row| &row[row.find('(').unwrap()..row.find(')').unwrap() + 1])
            .collect();
        assert_eq!(ages, vec!["(0h 0m)", "(0h 2m)", "(2h 6m)"], "{rows:?}");
    }

    #[test]
    fn a_stale_beatless_mark_is_flagged_with_the_explicit_minutes_command() {
        let dir = sandbox("rows-stale-beatless");
        let now = 1_000_000_000;
        write(&dir, "vinge.10.review", &format!("{}\n", now - 114 * 3600));

        let rows = rows_at(&open_leases_in(&dir), at(now), HOUSE);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].contains("last seen never [stale]"), "{rows:?}");
        assert_eq!(
            rows[1],
            "  tt agent end vinge 10 review \"<summary>\" <minutes>"
        );
        assert!(rows[1].contains("vinge"), "the project token must survive");
    }

    #[test]
    fn a_stale_beaten_mark_is_flagged_with_the_trim_command() {
        let dir = sandbox("rows-stale-beaten");
        let now = 1_000_000_000;
        write(&dir, "loremind.-.ops", &format!("{}\n", now - 6 * 3600));
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(
            beats_path(&dir, "loremind.-.ops"),
            format!("{}\n", now - 3 * 3600),
        )
        .unwrap();

        let rows = rows_at(&open_leases_in(&dir), at(now), HOUSE);
        assert_eq!(rows.len(), 2, "{rows:?}");
        let seen = at(now - 3 * 3600).format("%H:%M").to_string();
        assert!(
            rows[0].ends_with(&format!("last seen {seen} [stale]")),
            "{rows:?}"
        );
        assert_eq!(
            rows[1],
            "  tt agent end loremind - ops \"<summary>\" --trim"
        );
        assert!(
            rows[1].contains("loremind"),
            "the project token must survive"
        );
    }

    #[test]
    fn a_stale_hook_beaten_mark_is_flagged_with_the_explicit_minutes_command() {
        let dir = sandbox("rows-stale-hook-beaten");
        let now = 1_000_000_000;
        write(&dir, "loremind.-.ops", &format!("{}\n", now - 6 * 3600));
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(
            beats_path(&dir, "loremind.-.ops"),
            format!("{} hook\n", now - 3 * 3600),
        )
        .unwrap();

        let rows = rows_at(&open_leases_in(&dir), at(now), HOUSE);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(
            rows[1],
            "  tt agent end loremind - ops \"<summary>\" <minutes>"
        );
    }

    #[test]
    fn no_marks_have_no_rows() {
        let dir = sandbox("rows-empty");
        assert_eq!(
            rows_at(&open_leases_in(&dir), Local::now(), HOUSE),
            Vec::<String>::new()
        );
    }

    // --- writer -----------------------------------------------------------

    /// Regular files directly in `dir`, so `beats/` and `closing/` do not count.
    fn count_files(dir: &Path) -> usize {
        match fs::read_dir(dir) {
            Ok(entries) => entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .count(),
            Err(_) => 0,
        }
    }

    fn read(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn a_key_is_sanitised_once_and_keeps_the_no_issue_sentinel() {
        assert_eq!(mark_key(key("tt", "8", "impl")), "tt.8.impl");
        assert_eq!(mark_key(key("vinge", "-", "plan")), "vinge.-.plan");
        assert_eq!(
            mark_key(key("my proj", "7", "code/review")),
            "my_proj.7.code_review"
        );
        assert_eq!(mark_key(key("app", "1.2", "impl")), "app.1-2.impl");
        assert_eq!(mark_key(key("app.web", "7", "impl")), "app-web.7.impl");
        let dir = Path::new("/marks");
        assert_eq!(
            mark_path(dir, &mark_key(key("my proj", "7", "impl"))),
            PathBuf::from("/marks/my_proj.7.impl")
        );
        assert_eq!(
            beats_path(dir, &mark_key(key("my proj", "7", "impl"))),
            PathBuf::from("/marks/beats/my_proj.7.impl")
        );
    }

    #[test]
    fn an_agent_label_is_a_fourth_sanitised_segment() {
        assert_eq!(
            mark_key(labelled("tt", "8", "impl", "reviewer")),
            "tt.8.impl.reviewer"
        );
        assert_eq!(
            mark_key(labelled("tt", "-", "impl", "code review")),
            "tt.-.impl.code_review"
        );
        assert_eq!(
            mark_key(labelled("tt", "8", "impl", "v1.2")),
            "tt.8.impl.v1-2"
        );
    }

    #[test]
    fn a_four_segment_name_reads_back_as_a_labelled_mark() {
        let dir = sandbox("agent-roundtrip");
        write(&dir, "tt.8.impl.reviewer", "1000200\n");

        let mark = &open_marks_in(&dir)[0];
        assert_eq!(mark.project, "tt");
        assert_eq!(mark.issue.as_deref(), Some("8"));
        assert_eq!(mark.phase, "impl");
        assert_eq!(mark.agent.as_deref(), Some("reviewer"));
        assert_eq!(mark.label(), "tt/8 impl:reviewer");
        assert!(closable(mark));
    }

    #[test]
    fn two_labels_on_one_phase_are_independent_marks() {
        let dir = sandbox("agent-independent");
        begin_in(&dir, labelled("tt", "8", "review", "style")).unwrap();
        begin_in(&dir, labelled("tt", "8", "review", "code")).unwrap();
        begin_in(&dir, key("tt", "8", "review")).unwrap();
        assert_eq!(count_files(&dir), 3);

        touch_in(&dir, labelled("tt", "8", "review", "style")).unwrap();
        let beats = |agent| {
            read_phase_in(&dir, labelled("tt", "8", "review", agent))
                .unwrap()
                .unwrap()
                .beats
                .len()
        };
        assert_eq!((beats("style"), beats("code")), (1, 0));

        cancel_in(&dir, labelled("tt", "8", "review", "style")).unwrap();
        assert!(!mark_path(&dir, "tt.8.review.style").exists());
        assert!(mark_path(&dir, "tt.8.review.code").is_file());
        assert!(mark_path(&dir, "tt.8.review").is_file());
    }

    #[test]
    fn a_labelled_stale_row_closes_the_mark_it_names() {
        let dir = sandbox("agent-close-line");
        let now = 1_000_000_000;
        write(&dir, "tt.8.review.style", &format!("{}\n", now - 6 * 3600));

        let rows = rows_at(&open_leases_in(&dir), at(now), HOUSE);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].starts_with("tt/8 review:style "), "{rows:?}");
        assert_eq!(
            rows[1],
            "  tt agent end tt 8 review --agent style \"<summary>\" <minutes>"
        );
    }

    #[test]
    fn begin_writes_the_start_the_reader_reads_back() {
        let dir = sandbox("begin");
        let Begin::Created(start) = begin_in(&dir, key("tt", "8", "impl")).unwrap() else {
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

        let again = begin_in(&dir, key("tt", "8", "impl")).unwrap();
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
            begin_in(&dir, key("broken", "1", "impl")).unwrap(),
            Begin::AlreadyOpen(None)
        );
    }

    #[test]
    fn each_touch_appends_one_beat_and_leaves_the_mark_alone() {
        let dir = sandbox("touch");
        begin_in(&dir, key("tt", "8", "impl")).unwrap();
        let mark = read(&dir, "tt.8.impl");

        for _ in 0..3 {
            assert_eq!(
                touch_in(&dir, key("tt", "8", "impl")).unwrap(),
                Touch::Recorded
            );
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
        assert_eq!(
            touch_in(&dir, key("tt", "8", "impl")).unwrap(),
            Touch::NoMark
        );
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
        begin_in(&dir, key("tt", "8", "impl")).unwrap();
        touch_in(&dir, key("tt", "8", "impl")).unwrap();
        begin_in(&dir, key("other", "9", "plan")).unwrap();
        touch_in(&dir, key("other", "9", "plan")).unwrap();

        cancel_in(&dir, key("tt", "8", "impl")).unwrap();

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

        cancel_in(&dir, key("tt", "8", "impl")).unwrap();
        cancel_in(&dir, key("never", "1", "begun")).unwrap();
    }

    // --- the closing sentinel ---------------------------------------------

    #[test]
    fn the_sentinel_holds_the_marks_own_start() {
        let dir = sandbox("closing-round-trip");
        let Begin::Created(start) = begin_in(&dir, key("tt", "8", "impl")).unwrap() else {
            panic!("a fresh mark is created");
        };
        assert!(!is_closing_in(&dir, key("tt", "8", "impl")));

        start_closing_in(&dir, key("tt", "8", "impl")).unwrap();
        assert!(is_closing_in(&dir, key("tt", "8", "impl")));
        assert_eq!(
            read(&dir, "closing/tt.8.impl").trim(),
            start.timestamp().to_string()
        );
    }

    #[test]
    fn cancel_clears_the_sentinel_with_the_mark_and_the_beats() {
        let dir = sandbox("closing-cancel");
        begin_in(&dir, key("tt", "8", "impl")).unwrap();
        touch_in(&dir, key("tt", "8", "impl")).unwrap();
        start_closing_in(&dir, key("tt", "8", "impl")).unwrap();

        cancel_in(&dir, key("tt", "8", "impl")).unwrap();

        assert!(!dir.join("tt.8.impl").exists());
        assert!(!dir.join("beats/tt.8.impl").exists());
        assert!(!dir.join("closing/tt.8.impl").exists());
        assert!(
            dir.join("closing").is_dir(),
            "the closing directory itself stays"
        );
    }

    #[test]
    fn cancel_tolerates_any_of_the_three_paths_being_absent() {
        let dir = sandbox("closing-cancel-partial");

        begin_in(&dir, key("tt", "8", "impl")).unwrap();
        cancel_in(&dir, key("tt", "8", "impl")).unwrap();

        fs::create_dir_all(dir.join("beats")).unwrap();
        write(&dir.join("beats"), "tt.8.impl", "1000200\n");
        cancel_in(&dir, key("tt", "8", "impl")).unwrap();

        fs::create_dir_all(dir.join("closing")).unwrap();
        write(&dir.join("closing"), "tt.8.impl", "1000100\n");
        cancel_in(&dir, key("tt", "8", "impl")).unwrap();

        assert_eq!(
            count_files(&dir),
            0,
            "nothing is left in the mark directory"
        );
        assert_eq!(count_files(&dir.join("beats")), 0);
        assert_eq!(count_files(&dir.join("closing")), 0);
    }

    #[test]
    fn a_directory_holding_only_a_sentinel_lists_no_marks() {
        let dir = sandbox("closing-not-a-mark");
        fs::create_dir_all(dir.join("closing")).unwrap();
        write(&dir.join("closing"), "tt.8.impl", "1000100\n");

        assert_eq!(open_marks_in(&dir), Vec::new());
    }

    #[test]
    fn begin_drops_the_beats_a_part_way_cancel_left_behind() {
        let dir = sandbox("closing-stale-beats");
        let stale = "1000100\n1000200\n";
        fs::create_dir_all(dir.join("beats")).unwrap();
        write(&dir.join("beats"), "tt.8.impl", stale);

        let Begin::Created(_) = begin_in(&dir, key("tt", "8", "impl")).unwrap() else {
            panic!("no mark was open, so one is created");
        };

        let phase = read_phase_in(&dir, key("tt", "8", "impl"))
            .unwrap()
            .unwrap();
        assert_eq!(phase.beats, Vec::<i64>::new(), "the stale beats survived");

        touch_in(&dir, key("tt", "8", "impl")).unwrap();
        let beats = read(&dir, "beats/tt.8.impl");
        assert_eq!(
            beats.lines().count(),
            stale.lines().count() - 1,
            "the new beat is the only line: {beats:?}"
        );
    }

    #[test]
    fn begin_over_a_sentinel_refuses_and_writes_nothing() {
        let dir = sandbox("closing-refuses-begin");
        let stale = "1000100\n1000200\n";
        fs::create_dir_all(dir.join("beats")).unwrap();
        write(&dir.join("beats"), "tt.8.impl", stale);
        fs::create_dir_all(dir.join("closing")).unwrap();
        write(&dir.join("closing"), "tt.8.impl", "1000100\n");

        assert_eq!(
            begin_in(&dir, key("tt", "8", "impl")).unwrap(),
            Begin::Closing
        );
        assert!(!dir.join("tt.8.impl").exists(), "a mark was opened");
        assert_eq!(read(&dir, "beats/tt.8.impl"), stale);
        assert_eq!(read(&dir, "closing/tt.8.impl"), "1000100\n");
    }

    // --- what `end` measures ----------------------------------------------

    #[test]
    fn a_phase_with_no_beats_is_one_unvouched_stretch() {
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
        // No skipped beat advances `prev`.
        let beats = [beat, beat, beat - 60, end + 600, end];
        assert_eq!(gaps_over(start, end, &beats, 45), vec![(beat, end)]);
    }

    #[test]
    fn the_threshold_is_floor_minutes_and_strictly_greater() {
        let start = 1_000_000;
        // 45m59s floors to 45, which is not *greater* than 45.
        assert_eq!(gaps_over(start, start + 45 * 60 + 59, &[], 45), vec![]);
        let end = start + 46 * 60;
        assert_eq!(gaps_over(start, end, &[], 45), vec![(start, end)]);
    }

    #[test]
    fn expiry_is_floor_minutes_and_strictly_greater() {
        let dir = sandbox("lease-expiry-floor");
        let start = 1_000_000;
        write(&dir, "proj.7.impl", &format!("{start}\n"));
        fs::create_dir_all(dir.join("beats")).unwrap();
        fs::write(beats_path(&dir, "proj.7.impl"), format!("{start} hook\n")).unwrap();
        let lease = lease_in(&dir, &open_marks_in(&dir)[0]);

        assert!(!lease.is_expired_at(at(start + 45 * 60 + 59), HOUSE));
        assert!(lease.is_expired_at(at(start + 46 * 60), HOUSE));
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

        let phase = read_phase_in(&dir, key("proj", "7", "impl"))
            .unwrap()
            .unwrap();
        assert_eq!(phase.started, 1_000_000);
        assert_eq!(phase.beats, vec![1_000_600, 1_002_000, 1_001_200]);
    }

    #[test]
    fn a_tagged_beat_after_a_touch_does_not_discard_the_touchs_anchor() {
        let dir = sandbox("phase-tagged-after-bare");
        let start = 1_000_000;
        write(&dir, "proj.7.impl", &format!("{start}\n"));
        fs::create_dir_all(dir.join("beats")).unwrap();
        write(
            &dir,
            "beats/proj.7.impl",
            &format!("{}\n{} hook\n", start + 20 * 60, start + 21 * 60),
        );

        let phase = read_phase_in(&dir, key("proj", "7", "impl"))
            .unwrap()
            .unwrap();
        assert_eq!(phase.beats, vec![start + 20 * 60]);
        assert_eq!((phase.beats[0] - phase.started) / 60, 20);
    }

    #[test]
    fn a_phase_whose_only_beats_are_tagged_reads_back_as_beatless() {
        let dir = sandbox("phase-only-tagged");
        write(&dir, "proj.7.impl", "1000000\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        write(&dir, "beats/proj.7.impl", "1000600 hook\n1000900 hook\n");

        let phase = read_phase_in(&dir, key("proj", "7", "impl"))
            .unwrap()
            .unwrap();
        assert_eq!(
            phase,
            Phase {
                started: 1_000_000,
                beats: Vec::new()
            }
        );
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

        let phase = read_phase_in(&dir, key("proj", "7", "impl"))
            .unwrap()
            .unwrap();
        assert!(phase.beats.is_empty(), "only a bare line is a beat");
    }

    #[test]
    fn an_unmarked_phase_reads_back_as_nothing() {
        let dir = sandbox("phase-unmarked");
        assert_eq!(read_phase_in(&dir, key("proj", "7", "impl")).unwrap(), None);
    }

    #[test]
    fn a_mark_that_is_not_a_timestamp_is_an_error_naming_the_path() {
        let dir = sandbox("phase-bad-mark");
        write(&dir, "proj.7.impl", "not a timestamp\n");

        let err = read_phase_in(&dir, key("proj", "7", "impl")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("proj.7.impl"),
            "the error names the path: {err}"
        );
    }
}
