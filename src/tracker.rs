use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::duration;
use crate::icons;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TimeEntry {
    pub id: u64,
    pub description: String,
    /// Project this entry belongs to, if known
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub start_time: DateTime<Local>,
    pub end_time: Option<DateTime<Local>>,
    /// Silent stretches inside this entry's span, as `tt log --idle` recorded them.
    #[serde(default)]
    pub idle: Vec<IdleInterval>,
    /// Free-form JSON hung on the entry by `--data` or the form's Data field.
    /// Always an object; see [`crate::entry_data`], which is the only thing that
    /// builds one. Skipped when unset, so entries without it are unchanged on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// One silent stretch inside an entry.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct IdleInterval {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

impl IdleInterval {
    pub fn new(start: DateTime<Local>, end: DateTime<Local>) -> Self {
        Self { start, end }
    }

    pub fn duration(&self) -> Duration {
        self.end.signed_duration_since(self.start)
    }

    /// `09:50 – 10:20 (30m)`, as the detail popover lists it.
    pub fn format_span(&self) -> String {
        format!(
            "{} – {} ({})",
            self.start.format("%H:%M"),
            self.end.format("%H:%M"),
            duration::format(self.duration())
        )
    }
}

/// Parse tags (words starting with #) from text and return (clean_text, tags)
pub fn parse_tags(text: &str) -> (String, Vec<String>) {
    let mut tags = Vec::new();
    let mut clean_parts = Vec::new();

    for word in text.split_whitespace() {
        if word.starts_with('#') && word.len() > 1 {
            // Remove the # prefix and add to tags
            tags.push(word[1..].to_string());
        } else {
            clean_parts.push(word);
        }
    }

    (clean_parts.join(" "), tags)
}

/// Infer the project from an entry's tags: the tag `X` for which some other tag
/// starts with `X/`; failing that, the first tag; with no tags, `None`.
pub fn infer_project(tags: &[String]) -> Option<String> {
    let with_child = tags.iter().enumerate().find(|(i, candidate)| {
        let prefix = format!("{}/", candidate);
        tags.iter()
            .enumerate()
            .any(|(j, other)| j != *i && other.starts_with(&prefix))
    });

    with_child
        .map(|(_, candidate)| candidate)
        .or_else(|| tags.first())
        .cloned()
}

/// One-time migration: infer any missing `project`, then stamp schema version 1.
pub fn migrate(data: &mut TimeData) {
    if data.schema_version >= 1 {
        return;
    }
    for entry in &mut data.entries {
        if entry.project.is_none() {
            entry.project = infer_project(&entry.tags);
        }
    }
    data.schema_version = 1;
}

/// Format tags for display (with # prefix), e.g. "#backend #bug"
pub fn format_tags(tags: &[String]) -> String {
    tags.iter()
        .map(|t| format!("#{}", t))
        .collect::<Vec<_>>()
        .join(" ")
}

impl TimeEntry {
    pub fn duration(&self) -> Duration {
        let end = self.end_time.unwrap_or_else(Local::now);
        end.signed_duration_since(self.start_time)
    }

    pub fn format_duration(&self) -> String {
        duration::format(self.duration())
    }

    /// The date column, as the entries table prints it.
    pub fn format_date(&self) -> String {
        self.start_time.format("%Y-%m-%d").to_string()
    }

    pub fn format_start_time(&self) -> String {
        self.start_time.format("%H:%M").to_string()
    }

    /// The end-time column: a clock time, or an em dash while the entry runs.
    pub fn format_end_time(&self) -> String {
        self.end_time
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_else(|| "—".to_string())
    }

    pub fn is_active(&self) -> bool {
        self.end_time.is_none()
    }

    /// Returns the status icon for this entry (active or empty)
    pub fn status_icon(&self) -> &'static str {
        if self.is_active() {
            icons::active()
        } else {
            ""
        }
    }

    /// Format tags for display (with # prefix)
    pub fn format_tags(&self) -> String {
        format_tags(&self.tags)
    }

    /// Check if entry has a specific tag (case-insensitive)
    pub fn has_tag(&self, tag: &str) -> bool {
        let tag_lower = tag.to_lowercase();
        self.tags.iter().any(|t| t.to_lowercase() == tag_lower)
    }

    /// Whether the entry's project is `name` (trim, case-insensitive). An entry
    /// with no project matches nothing.
    pub fn is_project(&self, name: &str) -> bool {
        self.project
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .is_some_and(|p| p.to_lowercase() == name.trim().to_lowercase())
    }

    /// The haystack `/` searches: everything a row or the popover can show, built
    /// from the same formatters the table renders with. Keep the two in step.
    pub fn search_haystack(&self) -> String {
        [
            self.id.to_string(),
            self.description.clone(),
            self.project.clone().unwrap_or_default(),
            self.format_tags(),
            self.format_date(),
            self.format_start_time(),
            self.format_end_time(),
            self.format_duration(),
        ]
        .join(" ")
        .to_lowercase()
    }

    /// Whether `needle` (already lower-cased) appears anywhere in this entry.
    pub fn matches_search(&self, needle_lower: &str) -> bool {
        self.search_haystack().contains(needle_lower)
    }

    /// The spans this entry would be left as if its idle stretches were trimmed
    /// away, earliest first. Pure, so the trim prompt can state the outcome before
    /// [`TimeData::split_at_idle`](TimeData::split_at_idle) writes exactly these
    /// spans; do not copy the arithmetic elsewhere.
    ///
    /// Each interval is clamped to the entry's span and dropped if that empties it;
    /// the rest are sorted and overlapping or touching neighbours merged, so such a
    /// pair cuts **one** hole. The result is the complement of those holes, so a
    /// hole at an endpoint contributes nothing.
    ///
    /// Empty means no trim: no `end_time`, nothing left after clamping, or idle
    /// covering the whole span, which leaves the entry as it is.
    pub fn trim_spans(&self) -> Vec<(DateTime<Local>, DateTime<Local>)> {
        let span_start = self.start_time;
        let Some(span_end) = self.end_time else {
            return Vec::new();
        };

        let mut gaps: Vec<IdleInterval> = self
            .idle
            .iter()
            .map(|gap| IdleInterval {
                start: gap.start.clamp(span_start, span_end),
                end: gap.end.clamp(span_start, span_end),
            })
            .filter(|gap| gap.end > gap.start)
            .collect();
        gaps.sort_by_key(|gap| gap.start);
        let mut holes: Vec<IdleInterval> = Vec::new();
        for gap in gaps {
            match holes.last_mut() {
                Some(last) if gap.start <= last.end => last.end = last.end.max(gap.end),
                _ => holes.push(gap),
            }
        }
        if holes.is_empty() {
            return Vec::new();
        }

        let mut spans: Vec<(DateTime<Local>, DateTime<Local>)> = Vec::new();
        let mut cursor = span_start;
        for hole in &holes {
            if hole.start > cursor {
                spans.push((cursor, hole.start));
            }
            cursor = hole.end;
        }
        if span_end > cursor {
            spans.push((cursor, span_end));
        }
        spans
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct TimeData {
    pub entries: Vec<TimeEntry>,
    pub next_id: u64,
    /// Store schema version; an unversioned file reads as 0 and migrates to 1
    #[serde(default)]
    pub schema_version: u32,
}

impl TimeData {
    pub fn active_entry(&self) -> Option<&TimeEntry> {
        self.entries.iter().find(|e| e.is_active())
    }

    pub fn active_entry_mut(&mut self) -> Option<&mut TimeEntry> {
        self.entries.iter_mut().find(|e| e.is_active())
    }

    /// Stop the currently active entry. Returns true if an entry was stopped.
    pub fn stop_active(&mut self) -> bool {
        if let Some(entry) = self.active_entry_mut() {
            entry.end_time = Some(Local::now());
            true
        } else {
            false
        }
    }

    pub fn today_entries(&self) -> Vec<&TimeEntry> {
        let today = Local::now().date_naive();
        self.entries
            .iter()
            .filter(|e| e.start_time.date_naive() == today)
            .collect()
    }

    pub fn today_total(&self) -> Duration {
        self.today_entries()
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    pub fn add_entry(
        &mut self,
        description: String,
        project: Option<String>,
        tags: Vec<String>,
        start_time: DateTime<Local>,
        end_time: Option<DateTime<Local>>,
    ) -> &TimeEntry {
        let entry = TimeEntry {
            id: self.next_id,
            description,
            project,
            tags,
            start_time,
            end_time,
            idle: Vec::new(),
            data: None,
        };
        self.next_id += 1;
        self.entries.push(entry);
        self.entries.last().unwrap()
    }

    /// Get entries for a specific date
    pub fn entries_for_date(&self, date: NaiveDate) -> Vec<&TimeEntry> {
        self.entries
            .iter()
            .filter(|e| e.start_time.date_naive() == date)
            .collect()
    }

    /// Get total duration for a specific date
    pub fn total_for_date(&self, date: NaiveDate) -> Duration {
        self.entries_for_date(date)
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    /// Get the start of the week (Monday) for a given date
    pub fn week_start(date: NaiveDate) -> NaiveDate {
        let days_from_monday = date.weekday().num_days_from_monday();
        date - Duration::days(days_from_monday as i64)
    }

    /// Get entries for a specific week (starting Monday)
    pub fn entries_for_week(&self, week_start: NaiveDate) -> Vec<&TimeEntry> {
        let week_end = week_start + Duration::days(7);
        self.entries
            .iter()
            .filter(|e| {
                let date = e.start_time.date_naive();
                date >= week_start && date < week_end
            })
            .collect()
    }

    /// Get total duration for a specific week
    pub fn total_for_week(&self, week_start: NaiveDate) -> Duration {
        self.entries_for_week(week_start)
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    /// Get daily breakdown for a week (returns Vec of (date, total_duration))
    pub fn daily_breakdown(&self, week_start: NaiveDate) -> Vec<(NaiveDate, Duration)> {
        (0..7)
            .map(|i| {
                let date = week_start + Duration::days(i);
                (date, self.total_for_date(date))
            })
            .collect()
    }

    /// Total tracked duration per day for a given year, keyed by date. Only
    /// days with at least one entry appear — a single pass over `entries`,
    /// unlike calling `total_for_date` once per day of the year.
    pub fn year_breakdown(&self, year: i32) -> std::collections::BTreeMap<NaiveDate, Duration> {
        let mut map = std::collections::BTreeMap::new();
        for entry in &self.entries {
            let date = entry.start_time.date_naive();
            if date.year() == year {
                *map.entry(date).or_insert_with(Duration::zero) += entry.duration();
            }
        }
        map
    }

    /// Update an existing entry by ID
    pub fn update_entry(
        &mut self,
        id: u64,
        description: String,
        project: Option<String>,
        tags: Vec<String>,
        start_time: DateTime<Local>,
        end_time: Option<DateTime<Local>>,
    ) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.description = description;
            entry.project = project;
            entry.tags = tags;
            entry.start_time = start_time;
            entry.end_time = end_time;
            true
        } else {
            false
        }
    }

    /// Set (or with `None`, clear) an entry's custom data. Separate from
    /// `add_entry`/`update_entry` so every writer — `--data`, the form — goes
    /// through one place, and `None` always means "clear it".
    pub fn set_entry_data(&mut self, id: u64, data: Option<serde_json::Value>) -> bool {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.data = data;
                true
            }
            None => false,
        }
    }

    /// Get an entry by ID
    pub fn get_entry(&self, id: u64) -> Option<&TimeEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Split the entry with `id` on every idle interval it carries — the store
    /// mechanic behind the user-facing trim. Returns the pieces' ids, earliest
    /// first, or an empty vec when nothing changed.
    ///
    /// The earliest piece keeps the original id in place, later pieces take fresh
    /// ids from `next_id`, and every piece inherits description, project, tags
    /// and custom data.
    ///
    /// [`TimeEntry::trim_spans`] owns the arithmetic; this is the mutation and the
    /// id policy only, pure over `&mut self` so the caller owns the transaction.
    pub fn split_at_idle(&mut self, id: u64) -> Vec<u64> {
        let Some(entry) = self.get_entry(id) else {
            return Vec::new();
        };
        let pieces = entry.trim_spans();
        if pieces.is_empty() {
            return Vec::new();
        }
        let template = entry.clone();

        let mut ids = Vec::with_capacity(pieces.len());
        for (i, (piece_start, piece_end)) in pieces.into_iter().enumerate() {
            let inside: Vec<IdleInterval> = template
                .idle
                .iter()
                .copied()
                .filter(|gap| gap.start >= piece_start && gap.end <= piece_end)
                .collect();
            if i == 0 {
                // In place, so the earliest piece keeps the original id.
                let first = self
                    .entries
                    .iter_mut()
                    .find(|e| e.id == id)
                    .expect("the entry resolved above");
                first.start_time = piece_start;
                first.end_time = Some(piece_end);
                first.idle = inside;
                ids.push(id);
            } else {
                let new_id = self.next_id;
                self.next_id += 1;
                self.entries.push(TimeEntry {
                    id: new_id,
                    description: template.description.clone(),
                    project: template.project.clone(),
                    tags: template.tags.clone(),
                    start_time: piece_start,
                    end_time: Some(piece_end),
                    idle: inside,
                    data: template.data.clone(),
                });
                ids.push(new_id);
            }
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|t| t.to_string()).collect()
    }

    fn entry(id: u64, project: Option<&str>, entry_tags: &[&str]) -> TimeEntry {
        TimeEntry {
            id,
            description: format!("entry {}", id),
            project: project.map(|p| p.to_string()),
            tags: tags(entry_tags),
            start_time: Local::now(),
            end_time: None,
            idle: Vec::new(),
            data: None,
        }
    }

    #[test]
    fn infers_project_from_sibling_tag_not_first_tag() {
        assert_eq!(
            infer_project(&tags(&[
                "62",
                "77",
                "90/#91/#94/#95",
                "loremind",
                "loremind/62",
                "ops"
            ])),
            Some("loremind".to_string())
        );
    }

    #[test]
    fn infers_project_from_sibling_tag_when_summary_token_is_first() {
        assert_eq!(
            infer_project(&tags(&["63:", "loremind", "loremind/63", "plan"])),
            Some("loremind".to_string())
        );
    }

    #[test]
    fn falls_back_to_first_tag_when_no_sibling_pair() {
        assert_eq!(
            infer_project(&tags(&["tt", "impl"])),
            Some("tt".to_string())
        );
    }

    #[test]
    fn infers_no_project_without_tags() {
        assert_eq!(infer_project(&[]), None);
    }

    #[test]
    fn migrate_fills_missing_projects_and_sets_schema_version() {
        let mut data = TimeData {
            entries: vec![
                entry(1, None, &["tt", "impl"]),
                entry(2, None, &["63:", "loremind", "loremind/63", "plan"]),
                entry(3, Some("explicit"), &["tt", "impl"]),
                entry(4, None, &[]),
            ],
            next_id: 5,
            schema_version: 0,
        };

        migrate(&mut data);

        assert_eq!(data.schema_version, 1);
        assert_eq!(data.entries[0].project.as_deref(), Some("tt"));
        assert_eq!(data.entries[1].project.as_deref(), Some("loremind"));
        assert_eq!(data.entries[2].project.as_deref(), Some("explicit"));
        assert_eq!(data.entries[3].project, None);
    }

    /// A fixed wall clock, so assertions read as offsets rather than literals.
    fn base() -> DateTime<Local> {
        NaiveDate::from_ymd_opt(2026, 8, 18)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
    }

    /// One logged entry of `span_minutes`, with idle as minute offsets from start.
    fn logged_with_idle(span_minutes: i64, gaps: &[(i64, i64)]) -> TimeData {
        let start = base();
        TimeData {
            entries: vec![TimeEntry {
                id: 7,
                description: "logged span".to_string(),
                project: Some("tt".to_string()),
                tags: tags(&["tt", "tt/36"]),
                start_time: start,
                end_time: Some(start + Duration::minutes(span_minutes)),
                idle: gaps
                    .iter()
                    .map(|(from, to)| {
                        IdleInterval::new(
                            start + Duration::minutes(*from),
                            start + Duration::minutes(*to),
                        )
                    })
                    .collect(),
                data: None,
            }],
            next_id: 8,
            schema_version: 1,
        }
    }

    /// The entry spans, as (minutes from `base`, minutes from `base`) pairs.
    fn spans(data: &TimeData) -> Vec<(i64, i64)> {
        data.entries
            .iter()
            .map(|e| {
                (
                    e.start_time.signed_duration_since(base()).num_minutes(),
                    e.end_time
                        .unwrap()
                        .signed_duration_since(base())
                        .num_minutes(),
                )
            })
            .collect()
    }

    #[test]
    fn splitting_on_one_idle_interval_gives_two_pieces_excluding_it() {
        let mut data = logged_with_idle(120, &[(50, 80)]);

        let ids = data.split_at_idle(7);

        assert_eq!(ids.len(), 2, "one hole in the middle cuts the span in two");
        assert_eq!(ids[0], 7, "the earliest piece keeps the original id");
        assert_eq!(ids[1], 8, "the later piece takes the next id");
        assert_eq!(data.next_id, 9);
        assert_eq!(spans(&data), vec![(0, 50), (50 + 30, 120)]);
        for entry in &data.entries {
            assert_eq!(entry.description, "logged span");
            assert_eq!(entry.project.as_deref(), Some("tt"));
            assert_eq!(entry.tags, tags(&["tt", "tt/36"]));
            assert!(entry.idle.is_empty(), "a piece kept a hole it excludes");
        }
    }

    #[test]
    fn splitting_on_two_idle_intervals_gives_three_pieces() {
        let mut data = logged_with_idle(180, &[(30, 45), (100, 130)]);

        let ids = data.split_at_idle(7);

        assert_eq!(ids, vec![7, 8, 9]);
        assert_eq!(spans(&data), vec![(0, 30), (45, 100), (130, 180)]);
    }

    #[test]
    fn an_idle_interval_touching_an_endpoint_gives_one_piece_not_an_empty_one() {
        let mut leading = logged_with_idle(90, &[(0, 20)]);
        assert_eq!(leading.split_at_idle(7), vec![7]);
        assert_eq!(spans(&leading), vec![(20, 90)]);
        assert_eq!(
            leading.next_id, 8,
            "no fresh id was spent on an empty piece"
        );

        let mut trailing = logged_with_idle(90, &[(70, 90)]);
        assert_eq!(trailing.split_at_idle(7), vec![7]);
        assert_eq!(spans(&trailing), vec![(0, 70)]);
        assert_eq!(trailing.next_id, 8);
    }

    #[test]
    fn idle_covering_the_whole_span_leaves_the_entry_untouched() {
        let mut data = logged_with_idle(90, &[(0, 90)]);
        let before = serde_json::to_string(&data).unwrap();

        assert!(data.split_at_idle(7).is_empty());

        assert_eq!(serde_json::to_string(&data).unwrap(), before);
        assert_eq!(spans(&data), vec![(0, 90)], "the logged entry survived");
    }

    #[test]
    fn an_entry_with_no_idle_is_not_split() {
        let mut data = logged_with_idle(90, &[]);
        let before = serde_json::to_string(&data).unwrap();

        assert!(data.split_at_idle(7).is_empty());
        assert!(data.split_at_idle(999).is_empty(), "an unknown id");

        assert_eq!(serde_json::to_string(&data).unwrap(), before);
    }

    #[test]
    fn the_pieces_sum_to_the_span_minus_the_idle_intervals() {
        let gaps = [(20, 35), (60, 61), (100, 140)];
        let mut data = logged_with_idle(180, &gaps);
        let original = data.entries[0].duration();
        let idle_total = data.entries[0]
            .idle
            .iter()
            .fold(Duration::zero(), |acc, gap| acc + gap.duration());

        data.split_at_idle(7);

        let after = data
            .entries
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration());
        assert_eq!(after, original - idle_total);
    }

    /// `trim_spans` as minute offsets from `base`.
    fn trim_offsets(span_minutes: i64, gaps: &[(i64, i64)]) -> Vec<(i64, i64)> {
        logged_with_idle(span_minutes, gaps).entries[0]
            .trim_spans()
            .into_iter()
            .map(|(from, to)| {
                (
                    from.signed_duration_since(base()).num_minutes(),
                    to.signed_duration_since(base()).num_minutes(),
                )
            })
            .collect()
    }

    #[test]
    fn trim_spans_cuts_the_span_at_every_hole() {
        assert_eq!(trim_offsets(120, &[(50, 80)]), vec![(0, 50), (80, 120)]);
        assert_eq!(
            trim_offsets(180, &[(30, 45), (100, 130)]),
            vec![(0, 30), (45, 100), (130, 180)]
        );
        // Unsorted input is sorted first, so stored order cannot change the answer.
        assert_eq!(
            trim_offsets(180, &[(100, 130), (30, 45)]),
            vec![(0, 30), (45, 100), (130, 180)]
        );
    }

    #[test]
    fn trim_spans_drops_the_empty_piece_at_an_endpoint() {
        assert_eq!(trim_offsets(90, &[(0, 20)]), vec![(20, 90)]);
        assert_eq!(trim_offsets(90, &[(70, 90)]), vec![(0, 70)]);
    }

    /// Overlapping and touching neighbours merge into one hole.
    #[test]
    fn trim_spans_merges_overlapping_intervals_into_one_hole() {
        assert_eq!(
            trim_offsets(120, &[(30, 60), (50, 80)]),
            vec![(0, 30), (80, 120)]
        );
        // Abutting exactly, and one interval swallowing another, are the same hole.
        assert_eq!(
            trim_offsets(120, &[(30, 60), (60, 80)]),
            vec![(0, 30), (80, 120)]
        );
        assert_eq!(
            trim_offsets(120, &[(30, 90), (40, 50)]),
            vec![(0, 30), (90, 120)]
        );
    }

    /// Empty is the answer to every case where nothing would change.
    #[test]
    fn trim_spans_is_empty_when_nothing_would_change() {
        assert!(trim_offsets(90, &[]).is_empty(), "no idle at all");
        assert!(
            trim_offsets(90, &[(0, 90)]).is_empty(),
            "idle covering the whole span leaves the entry alone"
        );
        assert!(
            trim_offsets(90, &[(-30, 0)]).is_empty(),
            "an interval clamped to nothing outside the span"
        );

        // A running entry has no end to split against.
        let mut running = logged_with_idle(90, &[(30, 45)]);
        running.entries[0].end_time = None;
        assert!(running.entries[0].trim_spans().is_empty());
    }

    #[test]
    fn a_store_written_before_idle_existed_loads_and_stays_at_schema_version_1() {
        let json = r#"{
            "entries": [
                {
                    "id": 1,
                    "description": "an older entry",
                    "project": "tt",
                    "tags": ["tt", "impl"],
                    "start_time": "2026-08-18T09:00:00+02:00",
                    "end_time": "2026-08-18T10:00:00+02:00"
                }
            ],
            "next_id": 2,
            "schema_version": 1
        }"#;

        let mut data: TimeData = serde_json::from_str(json).unwrap();

        assert!(data.entries[0].idle.is_empty(), "absent means empty");
        assert_eq!(data.entries[0].duration(), Duration::hours(1));
        migrate(&mut data);
        assert_eq!(data.schema_version, 1, "no version bump, no migration");

        // …and round-trips unchanged.
        let round_tripped: TimeData =
            serde_json::from_str(&serde_json::to_string(&data).unwrap()).unwrap();
        assert!(round_tripped.entries[0].idle.is_empty());
        assert_eq!(round_tripped.schema_version, 1);
    }

    #[test]
    fn custom_data_is_absent_from_the_json_until_it_is_set() {
        let mut data = TimeData {
            entries: vec![entry(1, None, &["tt"])],
            next_id: 2,
            schema_version: 1,
        };
        assert!(
            !serde_json::to_string(&data).unwrap().contains("\"data\""),
            "an unset field must not appear on disk"
        );

        assert!(data.set_entry_data(1, Some(serde_json::json!({"pr": 69}))));
        assert!(!data.set_entry_data(999, None), "an unknown id");

        let round_tripped: TimeData =
            serde_json::from_str(&serde_json::to_string(&data).unwrap()).unwrap();
        assert_eq!(
            round_tripped.entries[0].data,
            Some(serde_json::json!({"pr": 69}))
        );

        assert!(data.set_entry_data(1, None));
        assert_eq!(data.entries[0].data, None, "None clears it");
    }

    #[test]
    fn a_store_written_before_custom_data_existed_loads_with_none() {
        let json = r#"{
            "entries": [
                {
                    "id": 1,
                    "description": "an older entry",
                    "tags": [],
                    "start_time": "2026-08-18T09:00:00+02:00",
                    "end_time": "2026-08-18T10:00:00+02:00"
                }
            ],
            "next_id": 2,
            "schema_version": 1
        }"#;

        let data: TimeData = serde_json::from_str(json).unwrap();
        assert_eq!(data.entries[0].data, None, "absent means none");
    }

    /// Trimming copies the data onto every piece, the way it copies the tags.
    #[test]
    fn split_pieces_inherit_the_custom_data() {
        let mut data = logged_with_idle(120, &[(50, 80)]);
        data.set_entry_data(7, Some(serde_json::json!({"issue": 69})));

        data.split_at_idle(7);

        assert_eq!(data.entries.len(), 2);
        for entry in &data.entries {
            assert_eq!(entry.data, Some(serde_json::json!({"issue": 69})));
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut data = TimeData {
            entries: vec![entry(1, None, &["tt", "impl"])],
            next_id: 2,
            schema_version: 0,
        };

        migrate(&mut data);
        let after_first = serde_json::to_string(&data).unwrap();

        // A tag vector that would now infer differently must not be re-applied
        data.entries[0].tags = tags(&["other", "other/1"]);
        migrate(&mut data);

        data.entries[0].tags = tags(&["tt", "impl"]);
        assert_eq!(serde_json::to_string(&data).unwrap(), after_first);
        assert_eq!(data.entries[0].project.as_deref(), Some("tt"));
        assert_eq!(data.schema_version, 1);
    }

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
    }

    fn spanning(id: u64, start: DateTime<Local>, minutes: i64) -> TimeEntry {
        TimeEntry {
            id,
            description: format!("entry {}", id),
            project: None,
            tags: Vec::new(),
            start_time: start,
            end_time: Some(start + Duration::minutes(minutes)),
            idle: Vec::new(),
            data: None,
        }
    }

    #[test]
    fn year_breakdown_sums_same_day_entries_and_excludes_other_years() {
        let day1_morning = at(2026, 3, 1, 9);
        let day1_afternoon = at(2026, 3, 1, 14);
        let day2 = at(2026, 3, 2, 9);
        let other_year = at(2025, 3, 1, 9);

        let data = TimeData {
            entries: vec![
                spanning(1, day1_morning, 60),
                spanning(2, day1_afternoon, 30),
                spanning(3, day2, 45),
                spanning(4, other_year, 999),
            ],
            next_id: 5,
            schema_version: 1,
        };

        let breakdown = data.year_breakdown(2026);

        assert_eq!(breakdown.len(), 2, "only the two 2026 days appear");
        assert_eq!(
            breakdown[&day1_morning.date_naive()],
            Duration::minutes(90),
            "same-day entries are summed"
        );
        assert_eq!(breakdown[&day2.date_naive()], Duration::minutes(45));
        assert!(!breakdown.contains_key(&other_year.date_naive()));
    }

    #[test]
    fn year_breakdown_is_empty_for_a_year_with_no_entries() {
        let data = TimeData {
            entries: vec![spanning(1, at(2026, 1, 1, 9), 60)],
            next_id: 2,
            schema_version: 1,
        };

        assert!(data.year_breakdown(2020).is_empty());
    }
}
