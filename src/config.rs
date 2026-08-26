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
use serde::{Deserialize, Serialize};
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
    layout: Option<LayoutConfig>,
    general: Option<GeneralConfig>,
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
    /// How long an activity window may sit unaccounted for before
    /// `tt agent audit --auto-log` writes a fallback `#auto` entry for it.
    /// Unset (the default) disables auto-logging entirely — see
    /// docs/decisions/0002-auto-logging-unaccounted-activity.md.
    pub auto_log_after_minutes: Option<i64>,
    /// Have the `Stop` hook's `tt agent activity check` call auto-log this
    /// session's own unaccounted window, the same way `--auto-log` would.
    /// Requires `auto_log_after_minutes` to already be set — see
    /// docs/decisions/0003-auto-log-on-stop.md. A `true` value with no
    /// `auto_log_after_minutes` is a misconfiguration: [`load`] warns and
    /// disables it rather than silently no-op'ing.
    pub auto_log_on_stop: Option<bool>,
}

/// Which collapsible TUI surfaces start open, independent of whether
/// onboarding has run — see [`GeneralConfig`].
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub show_projects: Option<bool>,
    pub show_agents: Option<bool>,
    pub show_summary: Option<bool>,
    pub show_tags: Option<bool>,
}

/// Cross-cutting settings. The first-run popup shows until it has run once,
/// at which point [`save_onboarding`] sets `onboarding = false` so it stays
/// quiet; set it back to `true` (or delete the key) to see it again.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub onboarding: Option<bool>,
    /// Whether `tt` checks GitHub for a newer release on startup. Shows
    /// unless explicitly opted out — see [`updates_enabled`].
    pub auto_check_updates: Option<bool>,
}

/// Fully-resolved config (after merging any `include` chain).
#[derive(Debug, Default, Clone)]
pub struct Config {
    pub theme: ThemeConfig,
    pub icons: IconsConfig,
    pub duration: DurationConfig,
    pub list: ListConfig,
    pub agent: AgentConfig,
    pub layout: LayoutConfig,
    pub general: GeneralConfig,
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
            agent: validate_agent(raw.agent.unwrap_or_default()),
            layout: raw.layout.unwrap_or_default(),
            general: raw.general.unwrap_or_default(),
        },
        Err(e) => {
            eprintln!(
                "Warning: failed to load config from {}: {e:#}",
                path.display()
            );
            Config::default()
        }
    }
}

/// `auto_log_on_stop = true` without `auto_log_after_minutes` already set is a
/// config error, not a silent no-op — see docs/decisions/0003-auto-log-on-stop.md
/// decision 1. A loud warning and `auto_log_on_stop` reset to `None`, mirroring
/// how a file that fails to parse warns and falls back rather than erroring the
/// whole command.
fn validate_agent(agent: AgentConfig) -> AgentConfig {
    if agent.auto_log_on_stop == Some(true) && agent.auto_log_after_minutes.is_none() {
        eprintln!(
            "Warning: agent.auto_log_on_stop is set but agent.auto_log_after_minutes is not — \
             auto_log_on_stop requires it to be configured first (see \
             docs/decisions/0003-auto-log-on-stop.md). Ignoring auto_log_on_stop."
        );
        return AgentConfig {
            auto_log_on_stop: None,
            ..agent
        };
    }
    agent
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
    std::env::var_os("APPDATA").map(|dir| {
        PathBuf::from(dir)
            .join("timetracker-rs")
            .join("config.toml")
    })
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

    let text = fs::read_to_string(path)
        .with_context(|| format!("reading config file {}", path.display()))?;
    let raw: RawConfig =
        toml::from_str(&text).with_context(|| format!("parsing config file {}", path.display()))?;

    let Some(include) = raw.include.clone() else {
        return Ok(raw);
    };

    let include_path = resolve_include_path(&include, path);
    let base = load_raw(&include_path, visited).with_context(|| {
        format!(
            "including {} from {}",
            include_path.display(),
            path.display()
        )
    })?;
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
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\"))
        && let Some(base) = directories::BaseDirs::new()
    {
        return base.home_dir().join(rest);
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
        layout: merge_opt(base.layout, overlay.layout, merge_layout),
        general: merge_opt(base.general, overlay.general, merge_general),
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
        auto_log_after_minutes: o.auto_log_after_minutes.or(b.auto_log_after_minutes),
        auto_log_on_stop: o.auto_log_on_stop.or(b.auto_log_on_stop),
    }
}

fn merge_layout(b: LayoutConfig, o: LayoutConfig) -> LayoutConfig {
    LayoutConfig {
        show_projects: o.show_projects.or(b.show_projects),
        show_agents: o.show_agents.or(b.show_agents),
        show_summary: o.show_summary.or(b.show_summary),
        show_tags: o.show_tags.or(b.show_tags),
    }
}

fn merge_general(b: GeneralConfig, o: GeneralConfig) -> GeneralConfig {
    GeneralConfig {
        onboarding: o.onboarding.or(b.onboarding),
        auto_check_updates: o.auto_check_updates.or(b.auto_check_updates),
    }
}

/// A minutes-valued setting: the environment wins over the config file, which
/// wins over the built-in default. Set-but-empty and unparseable read as unset.
pub fn resolve_minutes(env_var: &str, configured: Option<i64>, default: i64) -> i64 {
    std::env::var(env_var)
        .ok()
        .and_then(|value| value.parse().ok())
        .or(configured)
        .unwrap_or(default)
}

/// Shows unless explicitly opted out.
pub fn should_onboard(general: &GeneralConfig) -> bool {
    general.onboarding != Some(false)
}

/// Runs unless explicitly opted out (`auto_check_updates = false`).
pub fn updates_enabled(general: &GeneralConfig) -> bool {
    general.auto_check_updates != Some(false)
}

/// Persists `[layout]` (replaced wholesale) and sets `[general].onboarding =
/// false` so the popup stays quiet after running once; other sections pass
/// through untouched. A parse failure warns and starts fresh rather than
/// erroring, since an `Err` here would skip the TUI's terminal restore on
/// the way out.
pub fn save_onboarding(layout: &LayoutConfig) -> Result<()> {
    let Some(path) = config_path() else {
        anyhow::bail!("no config path available (no home/APPDATA directory found)");
    };

    let mut doc: toml::Value = if path.exists() {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        match toml::from_str(&text) {
            Ok(doc) => doc,
            Err(e) => {
                eprintln!(
                    "Warning: config file {} failed to parse while saving onboarding's answer \
                     ({e:#}); other sections in it will be lost.",
                    path.display()
                );
                toml::Value::Table(Default::default())
            }
        }
    } else {
        toml::Value::Table(Default::default())
    };

    let table = match doc.as_table_mut() {
        Some(table) => table,
        None => {
            // Valid TOML but not a table at the top level: same treatment
            // as a parse failure — warn and start fresh.
            eprintln!(
                "Warning: config file {} is not a TOML table; its contents will be replaced.",
                path.display()
            );
            doc = toml::Value::Table(Default::default());
            doc.as_table_mut().expect("just constructed as a table")
        }
    };
    table.insert(
        "layout".to_string(),
        toml::Value::try_from(layout).context("serializing layout config")?,
    );
    match table
        .entry("general".to_string())
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
    {
        Some(general) => {
            general.insert("onboarding".to_string(), toml::Value::Boolean(false));
        }
        None => {
            // `general` exists but is not itself a table — replace just that
            // key rather than failing the whole save.
            table.insert(
                "general".to_string(),
                toml::Value::try_from(GeneralConfig {
                    onboarding: Some(false),
                    ..Default::default()
                })
                .context("serializing general config")?,
            );
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating config directory {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(&doc).context("serializing config file")?;
    // Temp file then rename, so a reader never sees a torn config file.
    let temp_path = path.with_extension("toml.tmp");
    fs::write(&temp_path, text)
        .with_context(|| format!("writing config file {}", temp_path.display()))?;
    fs::rename(&temp_path, &path)
        .with_context(|| format!("renaming into place over {}", path.display()))?;
    Ok(())
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

    #[test]
    fn should_onboard_shows_unless_explicitly_opted_out() {
        // Never onboarded: no explicit setting either way.
        assert!(should_onboard(&GeneralConfig::default()));
        // Opted back in by hand.
        assert!(should_onboard(&GeneralConfig {
            onboarding: Some(true),
            ..Default::default()
        }));
        // Opted out, whether by hand or by a prior onboarding run.
        assert!(!should_onboard(&GeneralConfig {
            onboarding: Some(false),
            ..Default::default()
        }));
    }

    #[test]
    fn updates_enabled_runs_unless_explicitly_opted_out() {
        assert!(updates_enabled(&GeneralConfig::default()));
        assert!(updates_enabled(&GeneralConfig {
            auto_check_updates: Some(true),
            ..Default::default()
        }));
        assert!(!updates_enabled(&GeneralConfig {
            auto_check_updates: Some(false),
            ..Default::default()
        }));
    }

    /// `save_onboarding` must survive a config file whose other sections
    /// are broken, since it runs from inside the TUI's event loop where an
    /// `Err` bubbling out skips the terminal restore.
    #[test]
    fn save_onboarding_tolerates_a_config_file_that_fails_to_parse() {
        let _guard = crate::storage::env_guard();
        let dir = crate::storage::env_sandbox("save-onboarding-bad-toml");
        let path = config_path().expect("a config path");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not valid = = toml").unwrap();

        let result = save_onboarding(&LayoutConfig {
            show_projects: Some(true),
            show_agents: Some(false),
            show_summary: Some(false),
            show_tags: Some(true),
        });
        assert!(
            result.is_ok(),
            "a broken file must not fail the save: {result:?}"
        );

        let saved = load();
        assert_eq!(saved.layout.show_projects, Some(true));
        assert_eq!(saved.general.onboarding, Some(false));
        drop(dir);
    }

    /// A hand-set `onboarding = false` must survive `save_onboarding` writing
    /// into the same `[general]` table (it writes the same value, but the
    /// point is the key isn't dropped or clobbered by something else).
    #[test]
    fn save_onboarding_preserves_an_existing_opt_out() {
        let _guard = crate::storage::env_guard();
        let dir = crate::storage::env_sandbox("save-onboarding-preserve-opt-out");
        let path = config_path().expect("a config path");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[general]\nonboarding = false\n").unwrap();

        save_onboarding(&LayoutConfig::default()).unwrap();

        let saved = load();
        assert_eq!(saved.general.onboarding, Some(false));
        drop(dir);
    }

    #[test]
    fn auto_log_on_stop_without_the_threshold_is_dropped_with_a_warning() {
        let dropped = validate_agent(AgentConfig {
            auto_log_on_stop: Some(true),
            auto_log_after_minutes: None,
            ..Default::default()
        });
        assert_eq!(dropped.auto_log_on_stop, None);
    }

    #[test]
    fn auto_log_on_stop_with_the_threshold_set_is_kept() {
        let kept = validate_agent(AgentConfig {
            auto_log_on_stop: Some(true),
            auto_log_after_minutes: Some(480),
            ..Default::default()
        });
        assert_eq!(kept.auto_log_on_stop, Some(true));
    }

    #[test]
    fn auto_log_on_stop_unset_is_untouched_either_way() {
        let agent = validate_agent(AgentConfig {
            auto_log_on_stop: None,
            auto_log_after_minutes: None,
            ..Default::default()
        });
        assert_eq!(agent.auto_log_on_stop, None);
    }

    /// Unrelated sections (e.g. `[theme]`) must survive a round trip through
    /// `save_onboarding`, which only ever touches `[layout]` and `[general]`.
    #[test]
    fn save_onboarding_preserves_unrelated_sections() {
        let _guard = crate::storage::env_guard();
        let dir = crate::storage::env_sandbox("save-onboarding-preserve-theme");
        let path = config_path().expect("a config path");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[theme]\naccent = \"#abcdef\"\n").unwrap();

        save_onboarding(&LayoutConfig::default()).unwrap();

        let saved = load();
        assert_eq!(saved.theme.accent.as_deref(), Some("#abcdef"));
        drop(dir);
    }
}
