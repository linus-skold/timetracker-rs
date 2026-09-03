# Usage

## Commands

### `tt start <description>`

Start tracking a new task. Only one task can be active at a time.  
Tags can be embedded inline using `#tag` syntax.

```sh
tt start Working on login page
tt start Fixing auth bug #backend #bugfix
tt start Reviewing the PR --data '{"pr": 42}'
```

`--data` hangs a JSON object on the entry — see [Custom data](#custom-data).

---

### `tt stop`

Stop the currently active task and record its duration.

```sh
tt stop
```

---

### `tt log -d <description> -t <duration> [--tags <tags>]`

Log a completed task with an explicit duration. Useful for recording work after the fact.

- `-d` / `--description` — description of the task
- `-t` / `--time` — duration (see [Duration Format](#duration-format))
- `--tags` — comma-separated list of tags *(optional)*
- `--data` — a JSON object of custom data *(optional; see [Custom data](#custom-data))*

Tags can also be embedded inline in the description using `#tag` syntax. Both styles can be combined.

```sh
tt log -d "Code review" -t 45m
tt log -d "Deploy to staging" -t 1h30m --tags devops,deployment
tt log -d "Fix #frontend bug" -t 2h --tags bugfix
```

---

### `tt today`

Show all entries logged today along with a total.

```sh
tt today
```

---

### `tt report [--week|--all|--since <date>] [--until <date>] [--project <name>] [--json]`

Roll up logged time by project and item, with a per-phase breakdown.

- `--week` — this week, from Monday
- `--all` — every entry, unbounded
- `--since <YYYY-MM-DD>` — from that date onwards
- `--until <YYYY-MM-DD>` — up to and including that date; narrows one of the three
  scopes above and is a usage error without one
- `--project <name>` — only entries whose project is `<name>`
- `--json` — machine-readable output

With no scope it reports today. Project totals come from each entry's **project
field** (`--project` on `tt start` / `tt log`), never from its tags.

A trailing `*` marks an item with a running entry. When spans overlap, the report
says so: `tt log` back-dates from now, so entries logged in a batch claim
overlapping slots — the totals stay right, the timeline does not.

```sh
tt report
tt report --week
tt report --since 2026-08-01 --until 2026-08-05
tt report --project timetracker-rs --json
```

---

### `tt list [-n <limit>]`

Show the most recent entries across all days (defaults to the last 20).

```sh
tt list
tt list -n 50
```

---

### `tt status`

Show the currently active task, when it started, and how long it has been running.

```sh
tt status
```

---

### `tt active`

Prints `true` if a task is currently being tracked, `false` otherwise. Useful for scripts and prompt integrations.

```sh
tt active
```

---

### `tt tui`

Open the interactive terminal UI for browsing and managing your entries.

```sh
tt tui
```

---

### `tt update [--check] [-y|--yes]`

Check GitHub for a newer release and, unless `--check` is passed, download
and install it in place.

```sh
tt update           # install the latest release, with a confirmation prompt
tt update --check   # only report whether a newer version exists
tt update -y        # skip the confirmation prompt
```

`tt` also checks for a newer release on startup, at most once a day, with a
short network timeout so a slow or offline connection never makes a command
feel slow. When one is found, the CLI prints a one-line note to stderr and
the TUI shows it in the status bar.

Turn the automatic check off with `auto_check_updates = false` under
`[general]` in the config file (see [Configuration](#configuration)), or by
setting `TT_SKIP_UPDATE_CHECK=1`. It's also skipped automatically whenever a
`CI` environment variable is set.

If you installed the Flatpak build, update with `flatpak update` instead —
`tt update` won't try to replace a binary it can't write to.

### `tt completions [shell]`

Print the shell completion hook for `bash`, `zsh`, `fish`, `powershell`,
`elvish` or `nu`. With no argument the shell is detected from `$SHELL`; pass one to
override. The installers print this suggestion for your shell after
installing. Evaluate it at shell startup:

```sh
eval "$(tt completions zsh)"      # ~/.zshrc
eval "$(tt completions bash)"     # ~/.bashrc
tt completions fish | source      # ~/.config/fish/config.fish
```

Completion then covers subcommands and flags, and completes live values from
your own store: `--project`, and the project, issue and phase positionals of
`tt agent`. Issues are scoped to the project already typed on the line.

Evaluate the hook at startup rather than saving its output: it embeds the
absolute path of the `tt` binary that produced it, so a saved copy breaks the
moment the binary moves or is upgraded in place.

Nushell is the exception: it has no `eval`, so save the script once into the
autoload directory. It calls `tt` by PATH name, so the saved copy stays valid
across upgrades. Requires nushell 0.108 or newer (the `@complete` attribute).

```nu
mkdir $nu.user-autoload-dirs.0
tt completions nu | save -f ($nu.user-autoload-dirs.0 | path join tt-completer.nu)
```

The hook is verified to load and register in bash 3.2 and zsh 5.9, and in
nushell 0.115.1 (the `@complete` wrapper form, which passes `tt tui` and
`tt report` through unchanged). For fish, powershell and elvish only the generator's output is checked: no shell has
parsed those scripts here, so treat them as untested.

---

## Duration Format

Durations are used with `tt log -t`. Supported formats:

| Input   | Meaning         |
|---------|-----------------|
| `2h`    | 2 hours         |
| `45m`   | 45 minutes      |
| `1h30m` | 1 hour 30 min   |
| `1h 30m`| 1 hour 30 min   |
| `90`    | 90 minutes      |

`h`/`m` may be upper or lower case. Anything outside these forms — `hello`,
`1x30`, `-45m`, `1h30` — is a usage error, not a zero-length or partially
honoured entry.

---

## Tags

Tags can be added to any entry in two ways:

1. **Inline** in the description using `#` prefix: `tt start Working on #frontend`
2. **Explicit flag** with `tt log --tags tagA,tagB,tagC`

Both methods can be combined and duplicates are automatically removed.

---

## Custom data

Any entry can carry a JSON object of its own, for information tags and a
description have nowhere to put — a PR number, a build id, whatever the caller
wants to read back later.

```sh
tt log -d "Code review" -t 45m --data '{"pr": 42, "repo": "timetracker-rs"}'
tt agent end tt 69 impl "json data field" --data '{"pr": 74}'
```

The value must be a **JSON object**. Invalid JSON, or valid JSON that is not an
object (a bare number, a string, an array), is a usage error — the entry is
never recorded with the field silently dropped.

`--data` is accepted by `tt start`, `tt log`, `tt agent item` and `tt agent
end`. In the TUI, the entry form's **Data** field edits the same value as one
line of compact JSON, and refuses to save what it cannot parse; the entry detail
popover (`Enter`) lists it as key/value rows under a **Data** header, with
nested keys flattened to `review.by` and array elements to `files[0]`.

Trimming an entry copies its data onto every piece.

---

## Data storage

Entries are stored as JSON in your OS's standard data directory (via the `directories` crate), e.g. `~/.local/share/tt/data.json` on Linux or `%APPDATA%\tt\data\data.json` on Windows.

## Configuration

Theme colors, icons, duration-color thresholds, and the default `tt list` limit can be overridden with a TOML config file at:

- Linux/macOS: `~/.config/timetracker-rs/config.toml`
- Windows: `%APPDATA%\timetracker-rs\config.toml`

All fields are optional; anything left unset falls back to the built-in default.

```toml
# Optionally pull in settings from another file first; this file's own
# values (below) then override anything the included file sets.
include = "~/dotfiles/timetracker-theme.toml"

[theme]
accent = "#8ab4f8"
active = "#81c78a"
inactive = "#909090"
header_bg = "#303030"
selected_bg = "#424242"
highlight = "#ffd54f"
duration_high = "#ef9a9a"
duration_med = "#ffe082"
duration_low = "#a5d6a7"
border = "#585858"
title = "#bababa"

[icons]
active = "▶️"
stopped = "⏹️"
logged = "📝"
warning = "⚠️"
calendar = "📅"
list = "📋"
agent = "🤖"

[duration]
entry_high_hours = 4   # single-entry duration coloring
entry_med_hours = 2
day_high_hours = 8     # daily-total duration coloring (week view)
day_med_hours = 4

[list]
default_limit = 20     # default for `tt list` when -n isn't passed

[agent]
max_gap_minutes = 45        # silence between heartbeats that `tt agent end` refuses on
max_unvouched_minutes = 120 # how long a phase with no heartbeat at all may run

[layout]
show_projects = true   # whether the Projects, Agent, Summary, and Tags panels
show_agents = false    # start open. Their toggle keys (P/A/S/T) always work
show_summary = false   # regardless of these defaults.
show_tags = true

[general]
onboarding = true          # shown until answered; the app then sets this to false
auto_check_updates = true  # startup check for a newer release; see `tt update`
```

`[layout]` and `[general].onboarding` are written automatically the first time
the TUI runs and its onboarding popup is answered (`s` to move on, `Esc` to
skip); neither needs to be hand-edited, though both can be. Onboarding's
second screen offers to run `npx skills add linus-skold/timetracker-rs`,
installing the `AGENTS.md` time-logging contract as a skill for whatever
coding agent you use.

The popup shows once, then the app sets `onboarding = false` so it stays
quiet. Set it back to `true` (or delete the key) to see it again.

`TT_MAX_GAP_MINUTES` and `TT_MAX_UNVOUCHED_MINUTES` override the two `[agent]`
settings for a single invocation.

`TT_DATA_DIR` overrides the directory holding `data.json` and `data.lock`, and
`TT_CONFIG_FILE` overrides the config file path itself. An empty value for either
counts as unset, leaving the defaults above in force.
