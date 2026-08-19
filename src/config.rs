/// User-configurable settings, loaded from a TOML file outside the binary:
///
/// - Linux/macOS: `~/.config/timetracker-rs/config.toml`
/// - Windows: `%APPDATA%\timetracker-rs\config.toml`
///
/// A config file may pull in another file with a top-level `include = "path"`
/// key; the included file is loaded first and this file's own values are
/// then layered on top of it, so a local file can selectively override a
/// shared/base config kept elsewhere.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    include: Option<String>,
    theme: Option<ThemeConfig>,
    icons: Option<IconsConfig>,
    duration: Option<DurationConfig>,
    list: Option<ListConfig>,
    agent: Option<AgentConfig>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct ThemeConfig {
    pub accent: Option<String>,
    pub active: Option<String>,
    pub inactive: Option<String>,
    pub header_bg: Option<String>,
    pub selected_bg: Option<String>,
    pub highlight: Option<String>,
    pub duration_high: Option<String>,
    pub duration_med: Option<String>,
    pub duration_low: Option<String>,
    pub border: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct IconsConfig {
    pub active: Option<String>,
    pub stopped: Option<String>,
    pub logged: Option<String>,
    pub warning: Option<String>,
    pub calendar: Option<String>,
    pub list: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct DurationConfig {
    pub entry_high_hours: Option<i64>,
    pub entry_med_hours: Option<i64>,
    pub day_high_hours: Option<i64>,
    pub day_med_hours: Option<i64>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct ListConfig {
    pub default_limit: Option<usize>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct AgentConfig {
    pub max_gap_minutes: Option<i64>,
    pub max_unvouched_minutes: Option<i64>,
}

/// Fully-resolved config (after merging any `include` chain).
#[derive(Debug, Default, Clone)]
pub struct Config {
    pub theme: ThemeConfig,
    pub icons: IconsConfig,
    pub duration: DurationConfig,
    pub list: ListConfig,
    pub agent: AgentConfig,
}

/// Loads the user's config file, if any. Missing file -> defaults. A
/// present-but-invalid file prints a warning to stderr and falls back to
/// defaults, rather than failing the whole command.
pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    if !path.exists() {
        return Config::default();
    }

    match load_raw(&path, &mut HashSet::new()) {
        Ok(raw) => Config {
            theme: raw.theme.unwrap_or_default(),
            icons: raw.icons.unwrap_or_default(),
            duration: raw.duration.unwrap_or_default(),
            list: raw.list.unwrap_or_default(),
            agent: raw.agent.unwrap_or_default(),
        },
        Err(e) => {
            eprintln!("Warning: failed to load config from {}: {e:#}", path.display());
            Config::default()
        }
    }
}

fn config_path() -> Option<PathBuf> {
    resolve_config_path(std::env::var_os("TT_CONFIG_FILE"), default_config_path())
}

/// The env-free half of [`config_path`], taking the default config file.
fn resolve_config_path(config_file: Option<OsString>, default: Option<PathBuf>) -> Option<PathBuf> {
    match config_file {
        Some(file) if !file.is_empty() => Some(PathBuf::from(file)),
        _ => default,
    }
}

#[cfg(windows)]
fn default_config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("timetracker-rs").join("config.toml"))
}

#[cfg(not(windows))]
fn default_config_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|base| {
        base.home_dir()
            .join(".config")
            .join("timetracker-rs")
            .join("config.toml")
    })
}

fn load_raw(path: &Path, visited: &mut HashSet<PathBuf>) -> Result<RawConfig> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        anyhow::bail!("circular include chain at {}", path.display());
    }

    let text = fs::read_to_string(path).with_context(|| format!("reading config file {}", path.display()))?;
    let raw: RawConfig =
        toml::from_str(&text).with_context(|| format!("parsing config file {}", path.display()))?;

    let Some(include) = raw.include.clone() else {
        return Ok(raw);
    };

    let include_path = resolve_include_path(&include, path);
    let base = load_raw(&include_path, visited)
        .with_context(|| format!("including {} from {}", include_path.display(), path.display()))?;
    Ok(merge_raw(base, raw))
}

fn resolve_include_path(raw: &str, including_file: &Path) -> PathBuf {
    let expanded = expand_tilde(raw);
    if expanded.is_absolute() {
        expanded
    } else {
        including_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(expanded)
    }
}

fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        if let Some(base) = directories::BaseDirs::new() {
            return base.home_dir().join(rest);
        }
    }
    PathBuf::from(raw)
}

fn merge_raw(base: RawConfig, overlay: RawConfig) -> RawConfig {
    RawConfig {
        include: overlay.include.or(base.include),
        theme: merge_opt(base.theme, overlay.theme, merge_theme),
        icons: merge_opt(base.icons, overlay.icons, merge_icons),
        duration: merge_opt(base.duration, overlay.duration, merge_duration),
        list: merge_opt(base.list, overlay.list, merge_list),
        agent: merge_opt(base.agent, overlay.agent, merge_agent),
    }
}

fn merge_opt<T>(base: Option<T>, overlay: Option<T>, merge: impl FnOnce(T, T) -> T) -> Option<T> {
    match (base, overlay) {
        (Some(b), Some(o)) => Some(merge(b, o)),
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    }
}

fn merge_theme(b: ThemeConfig, o: ThemeConfig) -> ThemeConfig {
    ThemeConfig {
        accent: o.accent.or(b.accent),
        active: o.active.or(b.active),
        inactive: o.inactive.or(b.inactive),
        header_bg: o.header_bg.or(b.header_bg),
        selected_bg: o.selected_bg.or(b.selected_bg),
        highlight: o.highlight.or(b.highlight),
        duration_high: o.duration_high.or(b.duration_high),
        duration_med: o.duration_med.or(b.duration_med),
        duration_low: o.duration_low.or(b.duration_low),
        border: o.border.or(b.border),
        title: o.title.or(b.title),
    }
}

fn merge_icons(b: IconsConfig, o: IconsConfig) -> IconsConfig {
    IconsConfig {
        active: o.active.or(b.active),
        stopped: o.stopped.or(b.stopped),
        logged: o.logged.or(b.logged),
        warning: o.warning.or(b.warning),
        calendar: o.calendar.or(b.calendar),
        list: o.list.or(b.list),
        agent: o.agent.or(b.agent),
    }
}

fn merge_duration(b: DurationConfig, o: DurationConfig) -> DurationConfig {
    DurationConfig {
        entry_high_hours: o.entry_high_hours.or(b.entry_high_hours),
        entry_med_hours: o.entry_med_hours.or(b.entry_med_hours),
        day_high_hours: o.day_high_hours.or(b.day_high_hours),
        day_med_hours: o.day_med_hours.or(b.day_med_hours),
    }
}

fn merge_list(b: ListConfig, o: ListConfig) -> ListConfig {
    ListConfig {
        default_limit: o.default_limit.or(b.default_limit),
    }
}

fn merge_agent(b: AgentConfig, o: AgentConfig) -> AgentConfig {
    AgentConfig {
        max_gap_minutes: o.max_gap_minutes.or(b.max_gap_minutes),
        max_unvouched_minutes: o.max_unvouched_minutes.or(b.max_unvouched_minutes),
    }
}

/// Parses a "#RRGGBB" hex string into (r, g, b). Returns `None` on anything
/// else so callers can fall back to a default color rather than erroring.
pub fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tt_config_file_replaces_the_default_config_file() {
        let default = || Some(PathBuf::from("default"));
        assert_eq!(resolve_config_path(None, default()), default());
        // An empty `TT_CONFIG_FILE` is no setting at all.
        assert_eq!(resolve_config_path(Some("".into()), default()), default());
        assert_eq!(
            resolve_config_path(Some("elsewhere".into()), default()),
            Some(PathBuf::from("elsewhere"))
        );
        assert_eq!(
            resolve_config_path(None, None),
            None,
            "no default, no config file"
        );
        assert_eq!(
            resolve_config_path(Some("elsewhere".into()), None),
            Some(PathBuf::from("elsewhere"))
        );
    }
}
