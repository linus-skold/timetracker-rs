//! `tt skill install` — what it writes, and where it decides to write it.
//!
//! The hooks half is never exercised here: it edits Claude Code's own global
//! `settings.json`, which no sandbox can redirect.

mod common;

use common::Case;

/// The harness sets HOME, which is where `paths::home_dir` looks on Unix. On
/// Windows it reads USERPROFILE instead, and the shared harness deliberately
/// leaves that alone — setting it globally would blank `directories` for every
/// other test binary. Only these cases resolve a home directory, so they carry
/// the override themselves.
fn home_env(case: &Case) -> [(&str, &str); 1] {
    [("USERPROFILE", case.home.to_str().unwrap())]
}

/// Every file a working install needs. The hook scripts are as load-bearing as
/// the contract itself, so an install that dropped one would still look right.
const EXPECTED: &[&str] = &[
    "SKILL.md",
    "README.md",
    "scripts/install-hooks.mjs",
    "scripts/tt-activity-hook.mjs",
    "scripts/tt-contract-hook.mjs",
];

#[test]
fn install_writes_the_whole_skill_into_the_named_directory() {
    let case = Case::new("skill-install-dir");
    let skills = case.home.join("skills");

    let run = case.run_raw(
        &[
            "skill",
            "install",
            "--dir",
            skills.to_str().unwrap(),
            "--no-hooks",
        ],
        &[],
    );
    assert_eq!(run.status, Some(0), "stderr: {}", run.stderr);

    let dest = skills.join("tt-time-logging");
    for relative in EXPECTED {
        assert!(
            dest.join(relative).is_file(),
            "{relative} is missing from {}",
            dest.display()
        );
    }

    let contract = std::fs::read_to_string(dest.join("SKILL.md")).unwrap();
    assert!(
        contract.contains("tt agent begin"),
        "the installed SKILL.md is not the contract"
    );
}

/// The command is its own update path, so a second run must overwrite what a
/// first one left rather than refuse or append.
#[test]
fn installing_twice_overwrites_an_edited_file() {
    let case = Case::new("skill-install-twice");
    let skills = case.home.join("skills");
    let args = [
        "skill",
        "install",
        "--dir",
        skills.to_str().unwrap(),
        "--no-hooks",
    ];

    assert_eq!(case.run_raw(&args, &[]).status, Some(0));
    let contract = skills.join("tt-time-logging/SKILL.md");
    std::fs::write(&contract, "stale").unwrap();

    assert_eq!(case.run_raw(&args, &[]).status, Some(0));
    assert_ne!(
        std::fs::read_to_string(&contract).unwrap(),
        "stale",
        "a re-install left the old file in place"
    );
}

/// The bare command installs where the agents actually are, and the three that
/// share `~/.agents/skills` get one copy between them.
#[test]
fn the_bare_command_installs_for_every_detected_agent() {
    let case = Case::new("skill-install-detect");
    std::fs::create_dir_all(case.home.join(".claude")).unwrap();
    std::fs::create_dir_all(case.home.join(".gemini")).unwrap();

    // `--no-hooks` keeps the Node installer out of it: this asserts placement.
    let run = case.run_raw(&["skill", "install", "--no-hooks"], &home_env(&case));
    assert_eq!(run.status, Some(0), "stderr: {}", run.stderr);

    assert!(
        case.home
            .join(".claude/skills/tt-time-logging/SKILL.md")
            .is_file(),
        "Claude Code was detected but not installed for"
    );
    assert!(
        case.home
            .join(".agents/skills/tt-time-logging/SKILL.md")
            .is_file(),
        "Gemini reads the shared directory"
    );
    assert!(
        !case.home.join(".gemini/skills").exists(),
        "the shared directory is the whole install; nothing goes in ~/.gemini"
    );
}

/// An agent that is not on this machine must not have a directory made for it.
#[test]
fn an_absent_agent_gets_no_directory() {
    let case = Case::new("skill-install-absent");
    std::fs::create_dir_all(case.home.join(".claude")).unwrap();

    assert_eq!(
        case.run_raw(&["skill", "install", "--no-hooks"], &home_env(&case))
            .status,
        Some(0)
    );

    assert!(
        !case.home.join(".agents").exists(),
        "no Codex, Copilot or Gemini here, so no shared directory"
    );
}

/// `--agent` overrides detection, for installing ahead of an agent's first run.
#[test]
fn naming_an_agent_installs_for_it_regardless_of_detection() {
    let case = Case::new("skill-install-named");

    let run = case.run_raw(
        &["skill", "install", "--agent", "copilot", "--no-hooks"],
        &home_env(&case),
    );
    assert_eq!(run.status, Some(0), "stderr: {}", run.stderr);
    assert!(
        case.home
            .join(".agents/skills/tt-time-logging/SKILL.md")
            .is_file()
    );
}

#[test]
fn an_unknown_agent_is_an_error_not_a_silent_no_op() {
    let case = Case::new("skill-install-unknown");

    let run = case.run_raw(
        &["skill", "install", "--agent", "nosuchagent", "--no-hooks"],
        &home_env(&case),
    );

    assert_ne!(run.status, Some(0), "stdout: {}", run.stdout);
    assert!(!case.home.join(".agents").exists(), "nothing was written");
}

/// `TT_SKILL_DIR` is how an agent whose skills live somewhere else is reached
/// without passing `--dir` every time.
#[test]
fn the_skill_dir_variable_is_the_default_target() {
    let case = Case::new("skill-install-env");
    let skills = case.home.join("elsewhere");

    let run = case.run_raw(
        &["skill", "install", "--no-hooks"],
        &[("TT_SKILL_DIR", skills.to_str().unwrap())],
    );
    assert_eq!(run.status, Some(0), "stderr: {}", run.stderr);
    assert!(skills.join("tt-time-logging/SKILL.md").is_file());
}
