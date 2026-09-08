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
