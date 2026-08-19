# timetracker-rs

A personal time tracking CLI built in Rust. Track your working hours directly from the terminal.

## Installation

```sh
cargo install --git https://github.com/linus-skold/timetracker-rs
```

## Commands

### `tt start <description>`

Start tracking a new task. Only one task can be active at a time.  
Tags can be embedded inline using `#tag` syntax.

```sh
tt start Working on login page
tt start Fixing auth bug #backend #bugfix
```

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

## Duration Format

Durations are used with `tt log -t`. Supported formats:

| Input   | Meaning         |
|---------|-----------------|
| `2h`    | 2 hours         |
| `45m`   | 45 minutes      |
| `1h30m` | 1 hour 30 min   |
| `90`    | 90 minutes      |

---

## Tags

Tags can be added to any entry in two ways:

1. **Inline** in the description using `#` prefix: `tt start Working on #frontend`
2. **Explicit flag** with `tt log --tags tagA,tagB,tagC`

Both methods can be combined and duplicates are automatically removed.

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
```

`TT_MAX_GAP_MINUTES` and `TT_MAX_UNVOUCHED_MINUTES` override the two `[agent]`
settings for a single invocation.

`TT_DATA_DIR` overrides the directory holding `data.json` and `data.lock`, and
`TT_CONFIG_FILE` overrides the config file path itself. An empty value for either
counts as unset, leaving the defaults above in force.
