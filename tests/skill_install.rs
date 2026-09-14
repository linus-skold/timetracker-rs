//! `tt skill install` — what it writes, and where it decides to write it.
//!
//! The hooks half is never exercised here: it edits Claude Code's own global
//! `settings.json`, which no sandbox can redirect.

mod common;

use common::Case;

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
