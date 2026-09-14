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

Shell completion for `tt` is one line in your shell startup file,
`eval "$(tt completions <shell>)"` — see the readme's Quick start.

## Installing

`tt` carries this skill inside the binary, so the one command installs both the
skill and the Claude Code hooks — no network, no `npx`:

```sh
tt skill install
```

It installs for every agent it finds — Claude Code, Codex CLI, GitHub Copilot
and Gemini CLI — and `tt skill targets` lists them. Run it again to update. The
rest of this file describes what the hooks half of that command does, and how to
run it by hand.

## Enforcing this in Claude Code

`npx skills add` is tool-agnostic — it only copies this directory into place; it
knows nothing about Claude Code hooks, and prose alone gets skipped under context
pressure. If you installed with it, run this once afterwards to wire in
real enforcement (a `SessionStart` hook that injects the full contract once per
session, a `UserPromptSubmit` hook that re-injects the short operating card on
every prompt so the discipline survives context getting pushed out in a long
session, a `Stop` hook that warns about marks left open, and
`UserPromptSubmit`/`SubagentStop`/`Stop` hooks that renew this project's open
marks at every turn boundary so a mark that is still being worked does not
expire):

```sh
node <wherever the skill landed>/scripts/install-hooks.mjs
```

`tt skill install` runs exactly this script for you, against the copy it just
wrote.

This writes to your **global** `~/.claude/settings.json`, not a project-local
one — the hooks are meant to fire in every session, in every project, not just
the one you happened to run the installer from. It also copies `SKILL.md` and
the hook scripts into `~/.claude/hooks/tt-time-logging/`, so the hooks
keep working regardless of where the skill itself is installed. It requires
Claude Code to have already been run at least once (so `~/.claude` exists) —
if it hasn't, the script says so and exits without writing anything.

**These hooks need a `tt` that understands `tt agent activity prompt`.** They
pass a project to `tt agent activity`, which an older binary rejects — and a
hook never fails its event, so the failure is silent: no automatic beat, and no
session ends or subagent dispatches in the activity ledger. The installer probes
for the subcommand — running it with no project is a no-op — and warns if it is
missing; it still installs, since the contract injection works either way.

Safe to re-run. Then open `/hooks` once (or restart) so Claude Code picks up the
new `~/.claude/settings.json`.

**Re-run it after upgrading `tt`, too.** The automatic heartbeat lives in these
copied scripts, so an install still holding older copies gets no automatic beat
at all — and every mark then expires on the unvouched grace.

**Re-run it after editing `SKILL.md`.** The hooks read the copy under
`~/.claude/hooks/tt-time-logging/`, which is a snapshot taken at install time —
edits to the source file don't reach live sessions until you re-install.

### The card

`UserPromptSubmit` fires every turn, so injecting the whole contract there
re-spends the full document on every prompt and trains a "seen it, skip it"
response to the block. `tt-contract-hook.mjs` injects only the **operating
card** on that event: everything in `SKILL.md` above the `<!-- card:end -->`
marker — the trigger sentence alone, which names the command form, the project
rule and the phase list. The command block and the phase table sit below it.

The card is not a separate file to keep in sync; it's the head of `SKILL.md`,
cut at the marker. Keep the document's opening actionable and the card stays
correct on its own. `SessionStart` still injects the whole thing.

## Keeping the mirror honest

`SKILL.md` mirrors `AGENTS.md` in the timetracker-rs source repo. The two are
kept in sync when either changes.
