[![Build and Release](https://github.com/linus-skold/timetracker-rs/actions/workflows/build.yml/badge.svg)](https://github.com/linus-skold/timetracker-rs/actions/workflows/build.yml)
# timetracker-rs

A personal time tracking CLI built in Rust. Track your working hours directly from the terminal.

![tt example](docs/images/example.png)

## Installation

**Linux/macOS** — downloads the latest release binary and installs it as
`tt` under `~/.local/bin` (override with `TT_INSTALL_DIR`):

```sh
curl -fsSL https://raw.githubusercontent.com/linus-skold/timetracker-rs/main/install.sh | sh
```

**Windows (PowerShell)** — installs `tt.exe` under `%LOCALAPPDATA%\Programs\tt\bin`
(override with `$env:TT_INSTALL_DIR`) and adds it to your user `PATH`:

```powershell
irm https://raw.githubusercontent.com/linus-skold/timetracker-rs/main/install.ps1 | iex
```


**From source:**

```sh
cargo install --git https://github.com/linus-skold/timetracker-rs
```

## Quick start

Shell completion: `eval "$(tt completions)"` in your shell's rc file — see [docs/usage.md](docs/usage.md#tt-completions-shell).


```sh
tt start Working on login page
tt stop
tt today
```

See [docs/usage.md](docs/usage.md) for the full command reference, duration
format, tags, data storage location, and configuration file options.

## The agent skill

`tt` can teach your coding agent to log its own time — `agent begin` / `touch` /
`end`, one entry per phase of work. The contract is
[skills/tt-time-logging](skills/tt-time-logging/SKILL.md).

The installers offer this at the end, and the TUI offers it on first run. To do
it yourself:

```sh
tt skill install
```

That writes the skill into `~/.claude/skills/tt-time-logging`, then installs
Claude Code's hooks for it (see below). The skill files are built into the
binary, so the command needs no network and no `npx`.

**To update the skill, run the same command again.** It overwrites the files it
owns with the copies your current `tt` was built with, so upgrade `tt` first:

```sh
tt update
tt skill install
```

**Other agents.** Give `--dir` the skills directory your agent reads, or set
`TT_SKILL_DIR`:

```sh
tt skill install --dir ~/.config/<your agent>/skills
```

The skill is also published for the tool-agnostic `skills` CLI, which installs
it for several agents at once:

```sh
npx skills add linus-skold/timetracker-rs
```

Both routes place the same files. `npx skills add` installs no hooks, so run
`tt skill install` afterwards if you use Claude Code.

### Claude Code hooks

Prose alone gets skipped when the context gets full, so on Claude Code the
contract is enforced by hooks. `tt skill install` copies `SKILL.md` and the hook
scripts into `~/.claude/hooks/tt-time-logging/`, and registers them in your
**global** `~/.claude/settings.json`:

| Event | What it does |
|---|---|
| `SessionStart` | Injects the full contract, and opens the activity window. |
| `UserPromptSubmit` | Re-injects the short operating card, and renews open marks. |
| `SubagentStop` | Records a subagent dispatch. |
| `Stop` | Closes the activity window, then warns about marks left open. |

The hooks are Node scripts, so Node.js must be on your `PATH`. Adding them is
safe to repeat, and it is also how you update them.

Open `/hooks` in Claude Code once, or restart it, so the new settings file is
read.

To install the skill without touching Claude Code's settings:

```sh
tt skill install --no-hooks
```

To remove the hooks, delete the `tt-time-logging` entries from
`~/.claude/settings.json` and delete `~/.claude/hooks/tt-time-logging/`.

## Development

With [mise](https://mise.jdx.dev) installed:

```sh
mise run fmt         # rustfmt the tree
mise run fmt:check   # fail instead of rewriting — what CI and the hook ask
mise run lint        # clippy, with every warning an error
mise run lint:fix    # apply clippy's machine-applicable suggestions
mise run test        # cargo test --all-targets
mise run check       # all three gates, cheapest first
mise run hooks       # install the git hooks (one-off, per clone)
```

`mise run hooks` points `core.hooksPath` at [scripts/githooks](scripts/githooks),
so hooks stay under version control and reach everyone on their next pull.
The `pre-commit` hook formats the staged Rust files and re-stages them, so a
`cargo fmt` diff never lands as its own follow-up commit. A file that is both
staged and dirty is not rewritten — re-staging it would sweep the unstaged half
into the commit — so the hook reports it instead. Clippy is deliberately not in
the hook: it costs a compile, and `mise run check` before a push is the place
for it. Skip a hook once with
`git commit --no-verify`; uninstall with `git config --unset core.hooksPath`.

## Releasing

Releases are cut from `Cargo.toml`: pushing a version to `main` that has no
matching tag makes CI tag it and publish the binaries. To bump it, with
[mise](https://mise.jdx.dev) installed:

```sh
mise bump patch   # or: minor, major
```

That rewrites the version in both `Cargo.toml` and `Cargo.lock` without a
build. Commit the two files as `chore: version bump X.Y.Z` and open a PR;
merging it is what releases.
