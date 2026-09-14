//! Installs the `tt-time-logging` agent skill, and Claude Code's hooks for it.
//!
//! The skill files are embedded at build time, so an installed `tt` places them
//! with no network access and no `npx skills` — one less thing between a user
//! and a working contract. `npx skills add` still works, and reaches agents
//! whose skills directory this command does not know; it is a second route to
//! the same files, not the only one.

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

/// Claude Code's per-user directory, `~/.claude`.
pub fn claude_home() -> Option<PathBuf> {
    paths::home_dir().map(|home| home.join(".claude"))
}

/// Where the skill goes when the caller names no directory: `$TT_SKILL_DIR`,
/// else Claude Code's `~/.claude/skills`.
pub fn default_skills_dir() -> Option<PathBuf> {
    paths::env_or(
        std::env::var_os("TT_SKILL_DIR"),
        claude_home().map(|claude| claude.join("skills")),
    )
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

/// Installs the skill, and unless `hooks` is false, Claude Code's hooks for it.
pub fn install(skills_dir: Option<PathBuf>, hooks: bool) -> Result<()> {
    let skills_dir = match skills_dir.or_else(default_skills_dir) {
        Some(dir) => dir,
        None => anyhow::bail!(
            "Couldn't resolve a skills directory. Pass one with `tt skill install --dir <path>`."
        ),
    };

    let dest = write_skill_files(&skills_dir)?;
    println!("Installed the {SKILL_NAME} skill into {}.", dest.display());

    if hooks {
        install_claude_hooks(&dest);
    } else {
        println!("Skipped the Claude Code hooks (--no-hooks).");
    }

    println!(
        "\nFor an agent that reads its skills from somewhere else, install there too:\n  \
         tt skill install --dir <that agent's skills directory>\n  \
         npx skills add {SKILL_SLUG}"
    );
    Ok(())
}
