//! Candidate providers for dynamic shell completion. Every provider runs on each
//! Tab press: read through the lock-free `storage::load_data`, never `with_data`,
//! and on any failure return nothing rather than an error.

use std::collections::BTreeSet;
use std::ffi::OsString;

use clap::CommandFactory;
use clap_complete::CompletionCandidate;
use clap_complete::env::{Bash, Elvish, EnvCompleter, Fish, Powershell, Shells, Zsh};

use crate::agent::PHASES;
use crate::cli::Cli;
use crate::marks::{Mark, open_marks};
use crate::report::classify;
use crate::storage::load_data;
use crate::tracker::{self, TimeData};

/// Every supported shell; the single definition behind `CompleteEnv`, the
/// `completions` argument and its error message.
pub const SHELLS: Shells<'static> = Shells(&[&Bash, &Elvish, &Fish, &Nu, &Powershell, &Zsh]);

/// Nushell. The registration script is saved to disk, not evaluated, so it
/// invokes `tt` by PATH name rather than by absolute path.
pub struct Nu;

impl EnvCompleter for Nu {
    fn name(&self) -> &'static str {
        "nu"
    }

    fn is(&self, name: &str) -> bool {
        name == "nu" || name == "nushell"
    }

    fn write_registration(
        &self,
        var: &str,
        name: &str,
        bin: &str,
        _completer: &str,
        buf: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        writeln!(
            buf,
            r##"def {name}-completer [spans: list<string>]: nothing -> list {{
    with-env {{{var}: "nu"}} {{ ^r#'{bin}'# -- ...$spans }} | from json
}}

@complete {name}-completer
def --wrapped {bin} [...args] {{ ^r#'{bin}'# ...$args }}"##
        )
    }

    fn write_complete(
        &self,
        cmd: &mut clap::Command,
        mut args: Vec<OsString>,
        current_dir: Option<&std::path::Path>,
        buf: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        // nu hands over `" "` for an empty word after a space.
        if let Some(last) = args.last_mut()
            && last.to_string_lossy().trim().is_empty()
        {
            *last = OsString::new();
        }
        let index = args.len().saturating_sub(1);
        let candidates = clap_complete::engine::complete(cmd, args, index, current_dir)?;
        let json: Vec<serde_json::Value> = candidates.iter().map(candidate_json).collect();
        writeln!(buf, "{}", serde_json::Value::Array(json))
    }
}

fn candidate_json(c: &CompletionCandidate) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("value".into(), c.get_value().to_string_lossy().into());
    if let Some(help) = c.get_help() {
        let first = help
            .to_string()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        obj.insert("description".into(), first.into());
    }
    serde_json::Value::Object(obj)
}

/// Every project named by a store entry or an open mark.
pub fn projects() -> Vec<CompletionCandidate> {
    to_candidates(project_names(&store(), &open_marks()))
}

/// Issues known for the project typed earlier on the line, or every issue when
/// no project can be read off it. Always includes the no-issue sentinel `-`.
pub fn issues() -> Vec<CompletionCandidate> {
    let args = std::env::args_os().map(|a| a.to_string_lossy().into_owned());
    let project = typed_project(args);
    to_candidates(issue_names(&store(), &open_marks(), project.as_deref()))
}

/// The store as `tt report` sees it: migrated in memory, never written back.
fn store() -> TimeData {
    let mut data = load_data().unwrap_or_default();
    tracker::migrate(&mut data);
    data
}

pub fn phases() -> Vec<CompletionCandidate> {
    PHASES.iter().map(CompletionCandidate::new).collect()
}

fn to_candidates(names: BTreeSet<String>) -> Vec<CompletionCandidate> {
    names.into_iter().map(CompletionCandidate::new).collect()
}

fn project_names(data: &TimeData, marks: &[Mark]) -> BTreeSet<String> {
    data.entries
        .iter()
        .filter_map(|e| e.project.clone())
        .chain(marks.iter().map(|m| m.project.clone()))
        .filter(|p| !p.is_empty())
        .collect()
}

fn issue_names(data: &TimeData, marks: &[Mark], project: Option<&str>) -> BTreeSet<String> {
    let wanted = |p: &str| project.is_none_or(|w| w == p);
    let from_entries = data.entries.iter().filter_map(|e| {
        let (item, _) = classify(&e.tags);
        let (p, issue) = item?.split_once('/')?;
        (wanted(p) && !issue.is_empty()).then(|| issue.to_string())
    });
    let from_marks = marks
        .iter()
        .filter(|m| wanted(&m.project))
        .filter_map(|m| m.issue.clone());
    from_entries
        .chain(from_marks)
        .chain(std::iter::once("-".to_string()))
        .collect()
}

/// The project positional already typed on an `agent` command line, read from
/// the completer's own argv: `<bin> -- tt agent <sub> [flags] <project> ...`.
/// `None` whenever that shape is not found, so the caller falls back to every issue.
fn typed_project<I: Iterator<Item = String>>(args: I) -> Option<String> {
    let words: Vec<String> = args.skip_while(|a| a != "--").skip(1).collect();
    let matches = Cli::command()
        .ignore_errors(true)
        .try_get_matches_from(words)
        .ok()?;
    let (_, sub) = matches.subcommand()?.1.subcommand()?;
    let project = sub.get_one::<String>("project")?;
    (!project.is_empty()).then(|| project.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::TimeEntry;
    use chrono::Local;

    fn entry(project: Option<&str>, tags: &[&str]) -> TimeEntry {
        TimeEntry {
            id: 0,
            description: String::new(),
            project: project.map(String::from),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: Local::now(),
            end_time: None,
            idle: Vec::new(),
            data: None,
        }
    }

    fn mark(project: &str, issue: Option<&str>) -> Mark {
        Mark {
            project: project.to_string(),
            issue: issue.map(String::from),
            phase: "impl".to_string(),
            start: Local::now(),
        }
    }

    fn data(entries: Vec<TimeEntry>) -> TimeData {
        TimeData {
            entries,
            next_id: 1,
            schema_version: 1,
        }
    }

    fn argv(words: &[&str]) -> impl Iterator<Item = String> {
        words
            .iter()
            .map(|w| w.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn nu_complete(words: &[&str]) -> Vec<serde_json::Value> {
        let args = words.iter().map(OsString::from).collect();
        let mut buf = Vec::new();
        Nu.write_complete(&mut Cli::command(), args, None, &mut buf)
            .unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    #[test]
    fn nu_registration_calls_tt_by_name_and_attaches_the_completer() {
        let mut buf = Vec::new();
        Nu.write_registration("COMPLETE", "tt", "tt", "/abs/path/tt", &mut buf)
            .unwrap();
        let script = String::from_utf8(buf).unwrap();
        assert!(script.contains("@complete tt-completer"));
        assert!(script.contains("def tt-completer [spans: list<string>]"));
        assert!(script.contains("with-env {COMPLETE: \"nu\"}"));
        assert!(script.contains("def --wrapped tt [...args]"));
        assert!(!script.contains("/abs/path"), "{script}");
    }

    #[test]
    fn nu_candidates_are_json_records_with_optional_descriptions() {
        let got = nu_complete(&["tt", "agent", "begin", "proj", "-", "im"]);
        assert_eq!(got, [serde_json::json!({"value": "impl"})]);
        let subs = nu_complete(&["tt", "compl"]);
        assert_eq!(subs[0]["value"], "completions");
        assert!(
            subs[0]["description"]
                .as_str()
                .unwrap()
                .starts_with("Print the shell completion hook")
        );
    }

    #[test]
    fn nu_whitespace_span_completes_like_an_empty_one() {
        assert_eq!(
            nu_complete(&["tt", "agent", "begin", "p", "-", " "]),
            nu_complete(&["tt", "agent", "begin", "p", "-", ""])
        );
        let values: Vec<_> = nu_complete(&["tt", "agent", "begin", "p", "-", ""])
            .into_iter()
            .filter(|v| !v["value"].as_str().unwrap().starts_with("--"))
            .collect();
        assert_eq!(values.len(), PHASES.len());
    }

    #[test]
    fn projects_union_store_and_marks_deduplicated_and_sorted() {
        let d = data(vec![
            entry(Some("zeta"), &[]),
            entry(Some("alpha"), &[]),
            entry(None, &[]),
            entry(Some(""), &[]),
        ]);
        let names = project_names(&d, &[mark("alpha", None), mark("mid", None)]);
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            ["alpha", "mid", "zeta"]
        );
    }

    #[test]
    fn issues_scoped_to_project_with_sentinel() {
        let d = data(vec![
            entry(Some("a"), &["a/10", "impl"]),
            entry(Some("b"), &["b/20"]),
            entry(Some("a"), &["a/"]),
        ]);
        let marks = [
            mark("a", Some("11")),
            mark("b", Some("21")),
            mark("a", None),
        ];
        let scoped = issue_names(&d, &marks, Some("a"));
        assert_eq!(scoped.into_iter().collect::<Vec<_>>(), ["-", "10", "11"]);
    }

    #[test]
    fn issues_unfiltered_without_project() {
        let d = data(vec![
            entry(Some("a"), &["a/10"]),
            entry(Some("b"), &["b/20"]),
        ]);
        let all = issue_names(&d, &[mark("b", Some("21"))], None);
        assert_eq!(all.into_iter().collect::<Vec<_>>(), ["-", "10", "20", "21"]);
    }

    #[test]
    fn phases_are_the_canonical_list() {
        let got: Vec<String> = phases()
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, PHASES);
    }

    #[test]
    fn typed_project_reads_the_agent_positional() {
        let a = argv(&["/bin/tt", "--", "tt", "agent", "begin", "proj", ""]);
        assert_eq!(typed_project(a), Some("proj".to_string()));
    }

    #[test]
    fn typed_project_skips_flags_before_the_positionals() {
        let a = argv(&["/bin/tt", "--", "tt", "agent", "end", "--full", "proj", ""]);
        assert_eq!(typed_project(a), Some("proj".to_string()));
    }

    #[test]
    fn typed_project_is_none_on_odd_shapes() {
        assert_eq!(typed_project(argv(&["/bin/tt"])), None);
        assert_eq!(
            typed_project(argv(&["/bin/tt", "tt", "agent", "begin", "proj"])),
            None
        );
        assert_eq!(
            typed_project(argv(&["/bin/tt", "--", "tt", "agent", "begin"])),
            None
        );
        assert_eq!(
            typed_project(argv(&["/bin/tt", "--", "tt", "agent", "begin", ""])),
            None
        );
        assert_eq!(
            typed_project(argv(&["/bin/tt", "--", "tt", "start", "--project", ""])),
            None
        );
    }
}
