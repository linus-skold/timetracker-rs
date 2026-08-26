//! Shell completion through the real binary: the dynamic candidate protocol and
//! the static `tt completions <shell>` scripts, against a sandboxed store.

mod common;
use common::{Case, StoreRow, now};
use std::fs;

const SHELLS: [&str; 6] = ["bash", "elvish", "fish", "nu", "powershell", "zsh"];

fn row(project: &'static str, tags: &'static [&'static str]) -> StoreRow {
    let start = now() - 3600;
    StoreRow {
        description: "work",
        project: Some(project),
        tags,
        start,
        end: Some(start + 600),
    }
}

fn seeded(name: &str) -> Case {
    let case = Case::new(name);
    case.write_store(&[
        row("alpha", &["alpha/10", "impl", "agent"]),
        row("beta", &["beta/20", "impl", "agent"]),
        row("alpha", &["alpha/11", "qa", "agent"]),
    ]);
    case.write_mark("gamma.30.impl", now() - 60);
    case
}

/// Drive the completer as the shell hook does: `tt -- tt <words…>` with the
/// cursor at `index`, returning one candidate per line.
fn complete(case: &Case, index: usize, words: &[&str]) -> Vec<String> {
    let mut args = vec!["--", "tt"];
    args.extend_from_slice(words);
    let index = index.to_string();
    let run = case.run_bare_with_env(
        &args,
        &[
            ("COMPLETE", "bash"),
            ("_CLAP_COMPLETE_INDEX", &index),
            ("_CLAP_COMPLETE_COMP_TYPE", "9"),
            ("_CLAP_IFS", "\n"),
        ],
    );
    run.assert_status(0);
    run.stdout
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with("--"))
        .map(str::to_string)
        .collect()
}

/// Drive the completer as the generated nu `tt-completer` does: `COMPLETE=nu
/// tt -- tt <spans…>`, the cursor implied by the last span, JSON records back.
fn complete_nu(case: &Case, words: &[&str]) -> Vec<String> {
    let mut args = vec!["--", "tt"];
    args.extend_from_slice(words);
    let run = case.run_bare_with_env(&args, &[("COMPLETE", "nu")]);
    run.assert_status(0);
    let records: Vec<serde_json::Value> = serde_json::from_str(&run.stdout).unwrap();
    records
        .iter()
        .map(|r| r["value"].as_str().unwrap().to_string())
        .filter(|v| !v.starts_with("--"))
        .collect()
}

#[test]
fn nu_returns_the_same_candidates_as_bash() {
    let case = seeded("completions-nu-parity");
    let sub = complete_nu(&case, &[""]);
    for name in ["start", "stop", "log", "report", "agent", "completions"] {
        assert!(sub.contains(&name.to_string()), "missing {name} in {sub:?}");
    }
    assert_eq!(
        complete_nu(&case, &["start", "--project", ""]),
        complete(&case, 3, &["start", "--project", ""])
    );
    assert_eq!(
        complete_nu(&case, &["agent", "begin", "alpha", ""]),
        complete(&case, 4, &["agent", "begin", "alpha", ""])
    );
    assert_eq!(
        complete_nu(&case, &["agent", "begin", ""]),
        complete(&case, 3, &["agent", "begin", ""])
    );
    assert_eq!(
        complete_nu(&case, &["agent", "begin", "alpha", "-", ""]),
        complete(&case, 5, &["agent", "begin", "alpha", "-", ""])
    );
}

#[test]
fn nu_whitespace_span_completes_like_an_empty_span() {
    let case = seeded("completions-nu-space");
    assert_eq!(
        complete_nu(&case, &["start", "--project", " "]),
        ["alpha", "beta", "gamma"]
    );
}

#[test]
fn subcommand_names_complete_at_the_first_word() {
    let case = Case::new("completions-subcommands");
    let got = complete(&case, 1, &[""]);
    for name in ["start", "stop", "log", "report", "agent", "completions"] {
        assert!(got.contains(&name.to_string()), "missing {name} in {got:?}");
    }
}

#[test]
fn project_flag_completes_from_store_and_open_marks() {
    let case = seeded("completions-projects");
    assert_eq!(
        complete(&case, 3, &["start", "--project", ""]),
        ["alpha", "beta", "gamma"]
    );
}

#[test]
fn issue_positional_is_scoped_to_the_typed_project() {
    let case = seeded("completions-issues-scoped");
    assert_eq!(
        complete(&case, 4, &["agent", "begin", "alpha", ""]),
        ["-", "10", "11"]
    );
    assert_eq!(
        complete(&case, 4, &["agent", "begin", "gamma", ""]),
        ["-", "30"]
    );
}

#[test]
fn issue_positional_falls_back_to_every_issue_without_a_project() {
    let case = seeded("completions-issues-unscoped");
    let got = complete(&case, 4, &["agent", "begin", "", ""]);
    assert_eq!(got, ["-", "10", "11", "20", "30"]);
}

#[test]
fn phase_positional_completes_the_fixed_vocabulary() {
    let case = seeded("completions-phases");
    assert_eq!(
        complete(&case, 5, &["agent", "begin", "alpha", "10", ""]),
        [
            "plan", "impl", "qa", "review", "docs", "spike", "explore", "ops"
        ]
    );
}

#[test]
fn a_completion_run_leaves_the_store_and_its_lock_untouched() {
    let case = seeded("completions-no-store-write");
    let store = case.data_dir().join("data.json");
    let lock = case.data_dir().join("data.lock");
    let before = fs::metadata(&store).unwrap().modified().unwrap();
    let body_before = fs::read_to_string(&store).unwrap();

    complete(&case, 3, &["start", "--project", ""]);
    complete(&case, 4, &["agent", "begin", "alpha", ""]);
    complete_nu(&case, &["agent", "begin", "alpha", ""]);

    assert_eq!(fs::metadata(&store).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read_to_string(&store).unwrap(), body_before);
    assert!(
        !lock.exists(),
        "a completion run must not create the store lock"
    );
}

#[test]
fn the_completions_subcommand_prints_the_same_hook_as_the_env_protocol() {
    let case = Case::new("completions-subcommand-hook");
    for shell in SHELLS {
        let sub = case.run_bare(&["completions", shell]);
        sub.assert_status(0);
        let env = case.run_bare_with_env(&[], &[("COMPLETE", shell)]);
        env.assert_status(0);
        assert!(
            sub.stdout.contains("COMPLETE"),
            "{shell}: no COMPLETE in hook"
        );
        assert_eq!(sub.stdout, env.stdout, "{shell}: the two surfaces diverge");
    }
}

#[test]
fn an_unknown_shell_is_an_error_naming_the_choices() {
    let case = Case::new("completions-unknown-shell");
    let run = case.run_bare_with_env(&["completions"], &[("SHELL", "/bin/tcsh")]);
    assert_ne!(run.status, Some(0));
    assert!(
        run.stderr
            .contains("bash, elvish, fish, nu, powershell, zsh"),
        "{}",
        run.stderr
    );
}

#[test]
fn the_dynamic_hook_registers_for_every_shell() {
    let case = Case::new("completions-hook");
    for shell in SHELLS {
        let run = case.run_bare_with_env(&[], &[("COMPLETE", shell)]);
        run.assert_status(0);
        assert!(run.stdout.contains("tt"), "{shell} hook does not name tt");
    }
}
