# tt-time-logging — setup

Human-facing setup notes for this skill. The contract the agent actually
follows is [`SKILL.md`](SKILL.md); nothing here belongs in it.

## Prerequisite

The workflow is `tt` itself. Install it from the
[timetracker-rs](https://github.com/linus-skold/timetracker-rs) readme, or from
source:

```sh
cargo install --git https://github.com/linus-skold/timetracker-rs
```

## Enforcing this in Claude Code

`npx skills add` is tool-agnostic — it only copies this directory into place; it
knows nothing about Claude Code hooks, and prose alone gets skipped under context
pressure. If you're using Claude Code, run this once after installing to wire in
real enforcement (a `SessionStart` hook that injects the contract into every
session, a `UserPromptSubmit` hook that re-injects it on every prompt so the
discipline survives context getting pushed out in a long session, and a `Stop`
hook that warns about marks left open):

```sh
node <wherever the skill landed>/scripts/install-hooks.mjs
```

This writes to your **global** `~/.claude/settings.json`, not a project-local
one — the hooks are meant to fire in every session, in every project, not just
the one you happened to run the installer from. It also copies `SKILL.md` and
the stop-check script into `~/.claude/hooks/tt-time-logging/`, so the hooks
keep working regardless of where the skill itself is installed. It requires
Claude Code to have already been run at least once (so `~/.claude` exists) —
if it hasn't, the script says so and exits without writing anything.

Safe to re-run. Then open `/hooks` once (or restart) so Claude Code picks up the
new `~/.claude/settings.json`.

## Keeping the mirror honest

`SKILL.md` mirrors `AGENTS.md` in the timetracker-rs source repo. The two are
kept in sync when either changes.
