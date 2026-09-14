//! Installs the `tt-time-logging` agent skill, and Claude Code's hooks for it.
//!
//! The skill files are embedded at build time, so an installed `tt` places them
//! with no network access and no `npx skills` — one less thing between a user
//! and a working contract. `npx skills add` still works; it is a second route
//! to the same files, not the only one.
//!
//! Supporting another agent is a row in [`TARGETS`], not an installer: every
//! agent here reads the same `SKILL.md` layout, so only the directory differs.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::paths;

/// The skill directory name, under whichever skills root we install into.
pub const SKILL_NAME: &str = "tt-time-logging";

/// The repository slug `npx skills add` takes.
pub const SKILL_SLUG: &str = "linus-skold/timetracker-rs";

/// Relative path inside the skill, and the file's contents.
///
/// Embedded rather than downloaded: the binary and the contract then always
/// agree, because they were built from the same tree.
const SKILL_FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../skills/tt-time-logging/SKILL.md"),
    ),
    (
        "README.md",
        include_str!("../skills/tt-time-logging/README.md"),
    ),
    (
        "scripts/install-hooks.mjs",
        include_str!("../skills/tt-time-logging/scripts/install-hooks.mjs"),
    ),
    (
        "scripts/tt-activity-hook.mjs",
        include_str!("../skills/tt-time-logging/scripts/tt-activity-hook.mjs"),
    ),
    (
        "scripts/tt-contract-hook.mjs",
        include_str!("../skills/tt-time-logging/scripts/tt-contract-hook.mjs"),
    ),
];

/// One agent `tt` knows how to install for.
///
/// Adding an agent is adding a row. Nothing else in this module knows an
/// agent's name, so a contribution is the row, a line in the readme, and a case
/// in the tests.
#[derive(Debug)]
pub struct Target {
    /// What `--agent` takes.
    pub name: &'static str,
    /// What the install report calls it.
    pub label: &'static str,
    /// The agent's user-level skills directory, relative to the home directory.
    pub skills_dir: &'static str,
    /// A home-relative directory that exists once the agent has run. This is
    /// what "detected" means — we install where an agent actually lives rather
    /// than scattering directories for tools that are not there.
    pub probe: &'static str,
}

/// The agents `tt skill install` knows.
///
/// `.agents/skills` is not a guess: Codex, Copilot and Gemini CLI each document
/// it as the shared, tool-agnostic location, beside their own private one. One
/// copy there serves all three, so the rows deliberately repeat the path and
/// the installer folds them back together.
pub const TARGETS: &[Target] = &[
    Target {
        name: "claude",
        label: "Claude Code",
        skills_dir: ".claude/skills",
        probe: ".claude",
    },
    Target {
        name: "codex",
        label: "Codex CLI",
        skills_dir: ".agents/skills",
        probe: ".codex",
    },
    Target {
        name: "copilot",
        label: "GitHub Copilot",
        skills_dir: ".agents/skills",
        probe: ".copilot",
    },
    Target {
        name: "gemini",
        label: "Gemini CLI",
        skills_dir: ".agents/skills",
        probe: ".gemini",
    },
];

/// The row `--agent <name>` names.
pub fn target_named(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|target| target.name == name)
}

/// Whether this agent's own directory exists under `home`.
fn is_detected(target: &Target, home: &Path) -> bool {
    home.join(target.probe).is_dir()
}

/// Claude Code's per-user directory, `~/.claude`.
pub fn claude_home() -> Option<PathBuf> {
    paths::home_dir().map(|home| home.join(".claude"))
}

/// A directory to install into, and the agents served by installing there.
///
/// Several agents share one directory, so the report says "Codex CLI, GitHub
/// Copilot, Gemini CLI" over a single path rather than writing it three times.
struct Destination {
    dir: PathBuf,
    labels: Vec<&'static str>,
}

/// Folds the chosen targets into one entry per directory, in `TARGETS` order.
fn destinations(targets: &[&'static Target], home: &Path) -> Vec<Destination> {
    let mut out: Vec<Destination> = Vec::new();
    for target in targets {
        let dir = home.join(target.skills_dir);
        match out.iter_mut().find(|existing| existing.dir == dir) {
            Some(existing) => existing.labels.push(target.label),
            None => out.push(Destination {
                dir,
                labels: vec![target.label],
            }),
        }
    }
    out
}

/// Writes the embedded skill into `<skills_dir>/tt-time-logging`, overwriting
/// the files it owns, and returns that directory.
///
/// Overwriting is the update path: re-running the command is how a user picks
/// up a newer contract.
pub fn write_skill_files(skills_dir: &Path) -> Result<PathBuf> {
    let dest = skills_dir.join(SKILL_NAME);
    for (relative, contents) in SKILL_FILES {
        let file = dest.join(relative);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&file, contents).with_context(|| format!("writing {}", file.display()))?;
    }
    Ok(dest)
}

/// Runs the skill's own hook installer, which is the single source of truth for
/// what Claude Code's `settings.json` gets. Reimplementing it here would leave
/// two definitions of the same hook set to drift apart.
///
/// Best-effort: it reports what happened and never fails the install. The hooks
/// are Node scripts, so a machine without Node cannot run them either way.
fn install_claude_hooks(skill_dir: &Path) {
    let Some(claude) = claude_home() else {
        println!("Couldn't resolve your home directory — skipped the Claude Code hooks.");
        return;
    };
    if !claude.is_dir() {
        println!(
            "No {} — skipped the Claude Code hooks. Run Claude Code once, then \
             re-run `tt skill install`.",
            claude.display()
        );
        return;
    }

    let script = skill_dir.join("scripts/install-hooks.mjs");
    println!("\nInstalling Claude Code hooks...\n");
    match Command::new("node").arg(&script).status() {
        Ok(status) if status.success() => {}
        Ok(status) => println!("\ninstall-hooks.mjs exited with {status}."),
        Err(error) => println!(
            "\nCouldn't run node ({error}). The hooks are Node scripts — install \
             Node.js, then run:\n  node {}",
            script.display()
        ),
    }
}

/// What the caller asked `install` to write, and where.
pub struct Request {
    /// A directory named outright, which overrides every agent rule.
    pub dir: Option<PathBuf>,
    /// `--agent`, empty for "whichever agents are installed".
    pub agents: Vec<String>,
    /// `--all`: every known agent, detected or not.
    pub all: bool,
    /// Whether to wire Claude Code's hooks as well.
    pub hooks: bool,
}

/// Prints the detected agents, and the directory each would be served from.
pub fn list_targets() -> Result<()> {
    let home = paths::home_dir().context("Couldn't resolve your home directory")?;
    println!(
        "{:<10}{:<16}{:<22}STATUS",
        "AGENT", "NAME", "SKILLS DIRECTORY"
    );
    for target in TARGETS {
        println!(
            "{:<10}{:<16}{:<22}{}",
            target.name,
            target.label,
            format!("~/{}", target.skills_dir),
            if is_detected(target, &home) {
                "installed"
            } else {
                "not found"
            }
        );
    }
    println!(
        "\n\"installed\" means ~/{{{}}} exists. `tt skill install` writes to those; \
         `--all` writes to every row.",
        TARGETS
            .iter()
            .map(|target| target.probe)
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(())
}

/// Installs the skill for the requested agents, and Claude Code's hooks with it.
pub fn install(request: Request) -> Result<()> {
    // An explicit directory answers the question by itself: no detection, and
    // no assumption about which agent reads it.
    if let Some(dir) = request.dir.clone().or_else(explicit_dir_from_env) {
        let dest = write_skill_files(&dir)?;
        println!("Installed the {SKILL_NAME} skill into {}.", dest.display());
        if request.hooks {
            install_claude_hooks(&dest);
        }
        print_other_agents_hint();
        return Ok(());
    }

    let home = paths::home_dir().context(
        "Couldn't resolve your home directory. Pass a directory with \
         `tt skill install --dir <path>`.",
    )?;

    let targets = chosen_targets(&request, &home)?;
    let mut claude_skill_dir = None;

    for destination in destinations(&targets, &home) {
        let dest = write_skill_files(&destination.dir)?;
        println!(
            "Installed the {SKILL_NAME} skill into {} — for {}.",
            dest.display(),
            destination.labels.join(", ")
        );
        if destination.labels.contains(&"Claude Code") {
            claude_skill_dir = Some(dest);
        }
    }

    // Only Claude Code has hooks. The other agents read the contract on their
    // own, with no enforcement layer for `tt` to install.
    match (request.hooks, claude_skill_dir) {
        (true, Some(dir)) => install_claude_hooks(&dir),
        (true, None) => println!("\nNo Claude Code install selected, so no hooks were wired."),
        (false, _) => println!("\nSkipped the Claude Code hooks (--no-hooks)."),
    }

    print_other_agents_hint();
    Ok(())
}

/// `TT_SKILL_DIR`, which pins the install the way `--dir` does.
fn explicit_dir_from_env() -> Option<PathBuf> {
    paths::env_or(std::env::var_os("TT_SKILL_DIR"), None)
}

/// The rows to install for: the named ones, every one, or the installed ones.
fn chosen_targets(request: &Request, home: &Path) -> Result<Vec<&'static Target>> {
    if !request.agents.is_empty() {
        return request
            .agents
            .iter()
            .map(|name| {
                target_named(name).with_context(|| {
                    format!(
                        "Unknown agent \"{name}\". Known agents: {}.",
                        TARGETS
                            .iter()
                            .map(|target| target.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
            })
            .collect();
    }

    if request.all {
        return Ok(TARGETS.iter().collect());
    }

    let detected: Vec<_> = TARGETS
        .iter()
        .filter(|target| is_detected(target, home))
        .collect();
    if !detected.is_empty() {
        return Ok(detected);
    }

    // A machine where no agent has run yet still gets a usable install, at the
    // path the hooks below expect. Silence here would read as a failure.
    println!(
        "No agent directory found under {} — installing for Claude Code anyway.\n",
        home.display()
    );
    Ok(TARGETS
        .iter()
        .filter(|target| target.name == "claude")
        .collect())
}

fn print_other_agents_hint() {
    println!(
        "\nFor an agent this doesn't know, name its skills directory:\n  \
         tt skill install --dir <that agent's skills directory>\n  \
         npx skills add {SKILL_SLUG}\n\
         \n`tt skill targets` lists the agents it does know."
    );
}

#[cfg(test)]
mod tests;
