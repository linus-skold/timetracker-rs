//! Rollups over the store — the `tt report` surface: a [`TimeEntry::project`] →
//! item tree with per-phase breakdowns and an overlap count, the scope flags, and
//! the terminal and `--json` renderings.

use std::collections::BTreeMap;

use chrono::{Duration, Local, NaiveDate};
use serde::Serialize;

use crate::agent::PHASES;
use crate::duration as fmt_duration;
use crate::icons;
use crate::tracker::{TimeData, TimeEntry};

/// The bucket an entry with no `project` field lands in.
pub const NO_PROJECT: &str = "(no project)";

/// One item's totals — the item being a `<project>/<issue>` tag, else a description.
#[derive(Debug, Default)]
pub struct ItemNode {
    pub seconds: i64,
    /// Seconds per phase, for the ` · `-joined breakdown; empty when none carried one.
    pub phases: BTreeMap<String, i64>,
    /// The first contributing entry's description, shown when it differs from the key.
    pub description: String,
    /// Whether any contributing entry is still running.
    pub active: bool,
}

/// One project's totals and its items.
#[derive(Debug, Default)]
pub struct ProjectNode {
    pub seconds: i64,
    pub items: BTreeMap<String, ItemNode>,
}

/// A whole rollup: the tree, the grand total, and the overlap count.
#[derive(Debug, Default)]
pub struct Rollup {
    pub projects: BTreeMap<String, ProjectNode>,
    pub total_seconds: i64,
    /// Overlapping *pairs*, not entries — see [`count_overlaps`].
    pub overlaps: usize,
}

impl Rollup {
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

/// Split an entry's tags into its item axis (first tag containing a `/`) and its
/// phase axis (first tag in [`PHASES`]). Stored tags carry **no** `#`.
pub fn classify(tags: &[String]) -> (Option<&str>, Option<&str>) {
    let mut item = None;
    let mut phase = None;
    for tag in tags {
        if tag.contains('/') {
            if item.is_none() {
                item = Some(tag.as_str());
            }
        } else if phase.is_none() && PHASES.contains(&tag.as_str()) {
            phase = Some(tag.as_str());
        }
    }
    (item, phase)
}

/// An entry's billable seconds, clamped at zero so an entry whose end precedes its
/// start cannot subtract from the totals around it.
fn entry_seconds(entry: &TimeEntry) -> i64 {
    entry.duration().num_seconds().max(0)
}

/// Count overlapping *pairs* of spans, not overlapping entries — `tt log`
/// back-dates from now, so a batch of entries claims overlapping slots. The early
/// `break` is sound because the list is sorted by start.
pub fn count_overlaps(entries: &[&TimeEntry]) -> usize {
    let mut ordered: Vec<&&TimeEntry> = entries.iter().collect();
    ordered.sort_by_key(|entry| entry.start_time);

    let mut pairs = 0;
    for (i, first) in ordered.iter().enumerate() {
        let first_end = first.end_time.unwrap_or_else(Local::now);
        for second in &ordered[i + 1..] {
            if second.start_time >= first_end {
                break;
            }
            pairs += 1;
        }
    }
    pairs
}

/// Build the rollup for a set of entries.
pub fn rollup(entries: &[&TimeEntry]) -> Rollup {
    let mut result = Rollup {
        overlaps: count_overlaps(entries),
        ..Default::default()
    };

    for entry in entries {
        let seconds = entry_seconds(entry);
        let (item, phase) = classify(&entry.tags);
        let project = entry.project.as_deref().unwrap_or(NO_PROJECT);
        // Else the description, so a hand-written entry still gets its own row.
        let key = item.unwrap_or(entry.description.as_str());

        result.total_seconds += seconds;
        let project_node = result.projects.entry(project.to_string()).or_default();
        project_node.seconds += seconds;

        let item_node = project_node.items.entry(key.to_string()).or_default();
        if item_node.description.is_empty() {
            item_node.description = entry.description.clone();
        }
        item_node.seconds += seconds;
        item_node.active |= entry.end_time.is_none();
        if let Some(phase) = phase {
            *item_node.phases.entry(phase.to_string()).or_insert(0) += seconds;
        }
    }

    result
}

/// Keys of a map by descending value, the name breaking ties so the order is stable.
fn by_seconds_desc<T>(map: &BTreeMap<String, T>, seconds: impl Fn(&T) -> i64) -> Vec<&str> {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_by(|a, b| {
        seconds(&map[*b])
            .cmp(&seconds(&map[*a]))
            .then_with(|| a.cmp(b))
    });
    keys
}

fn seconds_to_duration(seconds: i64) -> Duration {
    Duration::seconds(seconds)
}

/// Truncate to `n` **characters** — byte slicing panics on a multi-byte boundary.
fn truncate(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// The human rollup: a header, then a row per project with its items beneath it.
pub fn render(rollup: &Rollup, label: &str) -> String {
    if rollup.is_empty() {
        return format!("No entries for {label}.\n");
    }

    let mut out = String::new();
    out.push_str(&format!(
        "{} {label} — {}\n\n",
        icons::calendar(),
        fmt_duration::format(seconds_to_duration(rollup.total_seconds))
    ));

    for project in by_seconds_desc(&rollup.projects, |node| node.seconds) {
        let node = &rollup.projects[project];
        out.push_str(&format!(
            "{:<28} {:>8}\n",
            project,
            fmt_duration::format(seconds_to_duration(node.seconds))
        ));

        for key in by_seconds_desc(&node.items, |item| item.seconds) {
            let item = &node.items[key];
            let marker = if item.active { " *" } else { "" };
            out.push_str(&format!(
                "  {:<26} {:>8}{}\n",
                truncate(key, 26),
                fmt_duration::format(seconds_to_duration(item.seconds)),
                marker
            ));
            // Its own line only when it says something the key does not.
            if item.description != key && !item.description.is_empty() {
                out.push_str(&format!("      {}\n", truncate(&item.description, 60)));
            }
            if !item.phases.is_empty() {
                let breakdown: Vec<String> = by_seconds_desc(&item.phases, |secs| *secs)
                    .into_iter()
                    .map(|phase| {
                        format!(
                            "{phase} {}",
                            fmt_duration::format(seconds_to_duration(item.phases[phase]))
                        )
                    })
                    .collect();
                out.push_str(&format!("      {}\n", breakdown.join(" · ")));
            }
        }
        out.push('\n');
    }

    if rollup.overlaps > 0 {
        out.push_str(&format!(
            "{} {} overlapping span(s) — retro `tt log` back-dates from now, so\n",
            icons::warning(),
            rollup.overlaps
        ));
        out.push_str(
            "totals are right but the timeline is not. Log at each commit, not in batches.\n",
        );
    }

    out
}

/// The `--json` payload: `projects` keyed on the `project` **field** with the
/// fieldless bucket under [`NO_PROJECT`], integer seconds, and the scope `label`.
#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub label: String,
    pub total_seconds: i64,
    pub overlaps: usize,
    pub projects: BTreeMap<String, JsonProject>,
}

#[derive(Debug, Serialize)]
pub struct JsonProject {
    pub seconds: i64,
    pub items: BTreeMap<String, JsonItem>,
}

#[derive(Debug, Serialize)]
pub struct JsonItem {
    pub seconds: i64,
    pub phases: BTreeMap<String, i64>,
    pub description: String,
    pub active: bool,
}

pub fn to_json(rollup: &Rollup, label: &str) -> JsonReport {
    JsonReport {
        label: label.to_string(),
        total_seconds: rollup.total_seconds,
        overlaps: rollup.overlaps,
        projects: rollup
            .projects
            .iter()
            .map(|(name, node)| {
                (
                    name.clone(),
                    JsonProject {
                        seconds: node.seconds,
                        items: node
                            .items
                            .iter()
                            .map(|(key, item)| {
                                (
                                    key.clone(),
                                    JsonItem {
                                        seconds: item.seconds,
                                        phases: item.phases.clone(),
                                        description: item.description.clone(),
                                        active: item.active,
                                    },
                                )
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}

/// What period a report covers, and what to call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Inclusive lower bound; `None` means unbounded (`--all`).
    pub from: Option<NaiveDate>,
    /// Inclusive upper bound; `None` means "up to the end of the data".
    pub until: Option<NaiveDate>,
    pub label: String,
}

/// Resolve the scope flags into bounds and a label. `--week` has a lower bound
/// only, so it includes anything dated after today.
pub fn resolve_scope(
    today: NaiveDate,
    all: bool,
    week: bool,
    since: Option<NaiveDate>,
    until: Option<NaiveDate>,
    project: Option<&str>,
) -> Scope {
    let mut scope = if all {
        Scope {
            from: None,
            until: None,
            label: "All entries".to_string(),
        }
    } else if week {
        let start = TimeData::week_start(today);
        Scope {
            from: Some(start),
            until: None,
            label: format!("Week of {start}"),
        }
    } else if let Some(since) = since {
        Scope {
            from: Some(since),
            until: None,
            label: format!("Since {since}"),
        }
    } else {
        Scope {
            from: Some(today),
            until: Some(today),
            label: format!("Today, {today}"),
        }
    };

    // `--until` only ever narrows, and clap requires a scope alongside it.
    if let Some(until) = until {
        scope.until = Some(until);
    }
    if let Some(project) = project {
        scope.label.push_str(&format!(" — #{project}"));
    }
    scope
}

/// The entries a scope selects: inside its dates, matching `project` on the field.
pub fn select<'a>(data: &'a TimeData, scope: &Scope, project: Option<&str>) -> Vec<&'a TimeEntry> {
    data.entries
        .iter()
        .filter(|entry| {
            let date = entry.start_time.date_naive();
            scope.from.is_none_or(|from| date >= from)
                && scope.until.is_none_or(|until| date <= until)
                && project.is_none_or(|wanted| entry.project.as_deref() == Some(wanted))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> chrono::DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 8, 18, hour, minute, 0)
            .single()
            .expect("a real local time")
    }

    fn entry(
        id: u64,
        project: Option<&str>,
        tags: &[&str],
        from: (u32, u32),
        minutes: i64,
    ) -> TimeEntry {
        let start = at(from.0, from.1);
        TimeEntry {
            id,
            description: "did the thing".to_string(),
            project: project.map(str::to_string),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: start,
            end_time: Some(start + Duration::minutes(minutes)),
            idle: Vec::new(),
            data: None,
        }
    }

    #[test]
    fn an_agent_shaped_entry_rolls_up_under_its_project_not_under_agent() {
        // `agent` tags every agent-written entry and must never be the project.
        let e = entry(1, Some("vinge"), &["vinge/6", "plan", "agent"], (9, 0), 30);
        let rolled = rollup(&[&e]);

        assert!(
            rolled.projects.contains_key("vinge"),
            "grouped on the project field"
        );
        assert!(
            !rolled.projects.contains_key("agent"),
            "the provenance tag is not a project"
        );
        let node = &rolled.projects["vinge"];
        assert_eq!(node.items.keys().collect::<Vec<_>>(), vec!["vinge/6"]);
        assert_eq!(node.items["vinge/6"].phases["plan"], 30 * 60);
    }

    #[test]
    fn an_entry_with_no_item_tag_is_keyed_on_its_description() {
        let e = entry(1, Some("tt"), &["impl", "agent"], (9, 0), 15);
        let rolled = rollup(&[&e]);
        assert_eq!(
            rolled.projects["tt"].items.keys().collect::<Vec<_>>(),
            vec!["did the thing"]
        );
    }

    #[test]
    fn an_entry_with_no_project_field_buckets_under_no_project() {
        let e = entry(1, None, &["plan"], (9, 0), 15);
        let rolled = rollup(&[&e]);
        assert_eq!(rolled.projects.keys().collect::<Vec<_>>(), vec![NO_PROJECT]);
    }

    #[test]
    fn phase_seconds_sum_across_entries_under_one_item() {
        let a = entry(1, Some("tt"), &["tt/12", "plan"], (9, 0), 30);
        let b = entry(2, Some("tt"), &["tt/12", "plan"], (10, 0), 15);
        let c = entry(3, Some("tt"), &["tt/12", "impl"], (11, 0), 60);
        let rolled = rollup(&[&a, &b, &c]);

        let item = &rolled.projects["tt"].items["tt/12"];
        assert_eq!(item.phases["plan"], 45 * 60, "two plan passes add up");
        assert_eq!(item.phases["impl"], 60 * 60);
        assert_eq!(item.seconds, 105 * 60);
        assert_eq!(rolled.total_seconds, 105 * 60);
    }

    #[test]
    fn an_active_entry_is_marked_and_measured_to_now() {
        let mut e = entry(1, Some("tt"), &["tt/12", "impl"], (9, 0), 30);
        e.start_time = Local::now() - Duration::minutes(20);
        e.end_time = None;

        let rolled = rollup(&[&e]);
        let item = &rolled.projects["tt"].items["tt/12"];
        assert!(item.active, "a running entry is flagged");
        // Measured against now, so it is about twenty minutes rather than zero.
        assert!(
            item.seconds >= 19 * 60 && item.seconds <= 21 * 60,
            "measured to now, got {}s",
            item.seconds
        );
    }

    #[test]
    fn overlaps_count_pairs_and_the_early_break_does_not_undercount() {
        // Two pairs (a×b, b×c) and not a×c — the `break` must not hide b×c.
        let a = entry(1, Some("tt"), &["tt/1"], (9, 0), 60); // 09:00–10:00
        let b = entry(2, Some("tt"), &["tt/2"], (9, 30), 60); // 09:30–10:30
        let c = entry(3, Some("tt"), &["tt/3"], (10, 15), 30); // 10:15–10:45

        assert_eq!(count_overlaps(&[&a, &b, &c]), 2, "a×b and b×c, not a×c");
        assert_eq!(count_overlaps(&[&a]), 0, "one span overlaps nothing");
    }

    #[test]
    fn a_corrupt_backwards_entry_contributes_no_negative_time() {
        let mut e = entry(1, Some("tt"), &["tt/12", "impl"], (9, 0), 30);
        e.end_time = Some(at(8, 0));
        assert_eq!(entry_seconds(&e), 0, "clamped, not negative");
    }

    #[test]
    fn classify_ignores_the_provenance_tag_and_takes_the_first_of_each_axis() {
        let tags: Vec<String> = ["agent", "vinge/6", "vinge/9", "plan", "impl"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(classify(&tags), (Some("vinge/6"), Some("plan")));
        assert_eq!(classify(&[]), (None, None));
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
    }

    #[test]
    fn the_default_scope_is_today_bounded_at_both_ends() {
        let today = date(2026, 8, 18);
        let scope = resolve_scope(today, false, false, None, None, None);
        assert_eq!(scope.from, Some(today));
        assert_eq!(scope.until, Some(today));
        assert_eq!(scope.label, "Today, 2026-08-18");
    }

    #[test]
    fn all_is_unbounded_and_week_starts_on_monday_with_no_upper_bound() {
        // 2026-08-18 is a Tuesday, so the week starts the day before.
        let today = date(2026, 8, 18);

        let all = resolve_scope(today, true, false, None, None, None);
        assert_eq!((all.from, all.until), (None, None));
        assert_eq!(all.label, "All entries");

        let week = resolve_scope(today, false, true, None, None, None);
        assert_eq!(week.from, Some(date(2026, 8, 17)), "Monday");
        assert_eq!(week.until, None, "a week report has no upper bound");
        assert_eq!(week.label, "Week of 2026-08-17");
    }

    #[test]
    fn since_and_until_bound_the_range_and_project_suffixes_any_label() {
        let today = date(2026, 8, 18);
        let scope = resolve_scope(
            today,
            false,
            false,
            Some(date(2026, 8, 1)),
            Some(date(2026, 8, 5)),
            Some("vinge"),
        );
        assert_eq!(scope.from, Some(date(2026, 8, 1)));
        assert_eq!(scope.until, Some(date(2026, 8, 5)));
        assert_eq!(scope.label, "Since 2026-08-01 — #vinge");
    }

    #[test]
    fn an_empty_rollup_says_so_and_renders_nothing_else() {
        let rendered = render(&Rollup::default(), "Today, 2026-08-18");
        assert_eq!(rendered, "No entries for Today, 2026-08-18.\n");
    }

    #[test]
    fn the_human_form_carries_the_header_the_rows_and_the_phase_breakdown() {
        let a = entry(1, Some("tt"), &["tt/12", "plan", "agent"], (9, 0), 30);
        let b = entry(2, Some("tt"), &["tt/12", "impl", "agent"], (11, 0), 60);
        let rendered = render(&rollup(&[&a, &b]), "Today, 2026-08-18");

        assert!(
            rendered.starts_with(&format!(
                "{} Today, 2026-08-18 — 1h 30m\n\n",
                icons::calendar()
            )),
            "header carries the icon, the label and the total: {rendered:?}"
        );
        assert!(
            rendered.contains("tt                             1h 30m\n"),
            "{rendered:?}"
        );
        assert!(
            rendered.contains("  tt/12                        1h 30m\n"),
            "{rendered:?}"
        );
        // Longest phase first, joined with the middot.
        assert!(
            rendered.contains("      impl 1h 0m · plan 0h 30m\n"),
            "{rendered:?}"
        );
        // The description line appears because it differs from the item key.
        assert!(rendered.contains("      did the thing\n"), "{rendered:?}");
        assert!(
            !rendered.contains("overlapping"),
            "two disjoint spans warn about nothing"
        );
    }

    #[test]
    fn the_overlap_warning_appears_only_when_spans_collide() {
        let a = entry(1, Some("tt"), &["tt/1", "impl"], (9, 0), 60);
        let b = entry(2, Some("tt"), &["tt/2", "impl"], (9, 30), 60);
        let rendered = render(&rollup(&[&a, &b]), "Today, 2026-08-18");
        assert!(
            rendered.contains(&format!("{} 1 overlapping span(s)", icons::warning())),
            "{rendered:?}"
        );
        assert!(rendered.contains("Log at each commit, not in batches."));
    }

    #[test]
    fn an_active_entry_is_starred_in_the_item_row() {
        let mut e = entry(1, Some("tt"), &["tt/12", "impl"], (9, 0), 30);
        e.end_time = None;
        let rendered = render(&rollup(&[&e]), "Today");
        assert!(
            rendered
                .lines()
                .any(|l| l.starts_with("  tt/12") && l.ends_with(" *")),
            "{rendered:?}"
        );
    }

    #[test]
    fn a_long_key_and_description_are_truncated_by_characters_not_bytes() {
        // Multi-byte throughout: byte slicing would panic mid-character.
        let mut e = entry(1, Some("tt"), &[], (9, 0), 30);
        e.description = "å".repeat(80);
        let rendered = render(&rollup(&[&e]), "Today");
        // The key is the description here, so it is cut at 26 and not repeated.
        assert!(rendered.contains(&"å".repeat(26)), "{rendered:?}");
        assert!(!rendered.contains(&"å".repeat(27)), "cut at 26 characters");
    }

    #[test]
    fn select_filters_on_the_date_bounds_and_on_the_project_field() {
        let inside = entry(1, Some("vinge"), &["vinge/6", "plan"], (9, 0), 30);
        let mut earlier = entry(2, Some("vinge"), &["vinge/7", "plan"], (9, 0), 30);
        earlier.start_time = at(9, 0) - Duration::days(3);
        earlier.end_time = Some(earlier.start_time + Duration::minutes(30));
        let other_project = entry(3, Some("tt"), &["tt/12", "plan"], (10, 0), 30);

        let data = TimeData {
            entries: vec![inside.clone(), earlier.clone(), other_project.clone()],
            ..Default::default()
        };

        let today = date(2026, 8, 18);
        let day = resolve_scope(today, false, false, None, None, None);
        assert_eq!(select(&data, &day, None).len(), 2, "the two dated today");

        let all = resolve_scope(today, true, false, None, None, None);
        assert_eq!(select(&data, &all, None).len(), 3);
        assert_eq!(
            select(&data, &all, Some("vinge"))
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "filtered on the field, not on a tag"
        );
    }

    #[test]
    fn the_json_payload_keys_projects_on_the_field_with_integer_seconds() {
        let agent_shaped = entry(1, Some("vinge"), &["vinge/6", "plan", "agent"], (9, 0), 30);
        let fieldless = entry(2, None, &["plan"], (9, 15), 15);
        let rolled = rollup(&[&agent_shaped, &fieldless]);

        let json = serde_json::to_value(to_json(&rolled, "Today, 2026-08-18"))
            .expect("the payload serialises");

        assert_eq!(json["total_seconds"], 45 * 60);
        assert_eq!(json["overlaps"], 1, "the two spans collide");
        assert_eq!(json["label"], "Today, 2026-08-18");

        // Project keys are the field, and the fieldless entry buckets separately.
        let projects = json["projects"].as_object().expect("an object");
        let mut names: Vec<&str> = projects.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, vec![NO_PROJECT, "vinge"]);
        assert!(
            !projects.contains_key("agent"),
            "the provenance tag is not a project"
        );

        let item = &json["projects"]["vinge"]["items"]["vinge/6"];
        assert_eq!(item["seconds"], 30 * 60);
        assert_eq!(item["phases"]["plan"], 30 * 60);
        assert_eq!(item["description"], "did the thing");
        assert_eq!(item["active"], false);

        // Integers throughout — no `.0` anywhere.
        assert!(json["total_seconds"].is_i64());
        assert!(item["seconds"].is_i64());
    }
}
