//! The parts of the install that are decisions rather than file writes: which
//! rows a request picks, and how rows fold onto directories.

use super::*;

fn target(name: &str) -> &'static Target {
    target_named(name).expect("the table should hold this agent")
}

/// The three agents that share `.agents/skills` must produce one write, not
/// three — and the shared entry must name all of them.
#[test]
fn agents_sharing_a_directory_fold_into_one_destination() {
    let home = Path::new("/home/someone");
    let chosen = vec![target("claude"), target("codex"), target("copilot")];

    let destinations = destinations(&chosen, home);

    assert_eq!(destinations.len(), 2, "two distinct directories");
    assert_eq!(destinations[0].dir, home.join(".claude/skills"));
    assert_eq!(destinations[0].labels, vec!["Claude Code"]);
    assert_eq!(destinations[1].dir, home.join(".agents/skills"));
    assert_eq!(destinations[1].labels, vec!["Codex CLI", "GitHub Copilot"]);
}

/// Every row must be reachable by the name `--agent` takes, or the table and
/// the flag have drifted apart.
#[test]
fn every_target_is_reachable_by_its_own_name() {
    for entry in TARGETS {
        let found = target_named(entry.name).expect("a row should answer to its name");
        assert_eq!(found.label, entry.label);
    }
    assert!(target_named("nosuchagent").is_none());
}

#[test]
fn a_named_agent_wins_over_detection() {
    let home = Path::new("/home/someone");
    let request = Request {
        dir: None,
        agents: vec!["gemini".to_string()],
        all: false,
        hooks: true,
    };

    let chosen = chosen_targets(&request, home).unwrap();

    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].name, "gemini", "no directory under home exists");
}

#[test]
fn an_unknown_agent_is_an_error_naming_the_known_ones() {
    let request = Request {
        dir: None,
        agents: vec!["clippy".to_string()],
        all: false,
        hooks: true,
    };

    let error = chosen_targets(&request, Path::new("/home/someone"))
        .expect_err("an unknown agent should not silently install nothing");

    let message = format!("{error}");
    assert!(message.contains("clippy"), "{message}");
    assert!(message.contains("copilot"), "{message}");
}

#[test]
fn all_selects_every_row_without_detecting_anything() {
    let request = Request {
        dir: None,
        agents: Vec::new(),
        all: true,
        hooks: true,
    };

    let chosen = chosen_targets(&request, Path::new("/home/someone")).unwrap();

    assert_eq!(chosen.len(), TARGETS.len());
}

/// Detection is what makes the bare command install where the agents are.
#[test]
fn detection_picks_the_agents_whose_directories_exist() {
    let home = std::env::temp_dir().join("tt-skill-detect");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".gemini")).unwrap();
    std::fs::create_dir_all(home.join(".claude")).unwrap();

    let request = Request {
        dir: None,
        agents: Vec::new(),
        all: false,
        hooks: true,
    };

    let chosen = chosen_targets(&request, &home).unwrap();
    let names: Vec<_> = chosen.iter().map(|target| target.name).collect();

    assert_eq!(
        names,
        vec!["claude", "gemini"],
        "codex and copilot are absent"
    );
}

/// With nothing installed the command must still leave a working skill behind,
/// at the path the Claude Code hooks expect.
#[test]
fn no_agent_at_all_falls_back_to_claude_code() {
    let home = std::env::temp_dir().join("tt-skill-detect-empty");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();

    let request = Request {
        dir: None,
        agents: Vec::new(),
        all: false,
        hooks: true,
    };

    let chosen = chosen_targets(&request, &home).unwrap();

    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].name, "claude");
}
