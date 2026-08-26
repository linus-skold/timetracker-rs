//! The self-update mechanism: a throttled, best-effort startup check plus the
//! `tt update` command itself. Nothing here may ever turn a broken network
//! into a failed command — a check that can't complete just stays silent and
//! retries on a later run.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::storage;

pub const REPO_OWNER: &str = "linus-skold";
pub const REPO_NAME: &str = "timetracker-rs";
const BIN_NAME: &str = "tt";

/// How long a cached "no update yet" result is trusted before the next
/// invocation is willing to hit the network again.
const CHECK_INTERVAL: chrono::Duration = chrono::Duration::hours(24);

/// The network fetch gets this long to answer before an invocation gives up
/// and moves on — a broken or slow connection must never make `tt` feel slow.
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Cache {
    last_checked: Option<DateTime<Utc>>,
    latest_version: Option<String>,
}

fn cache_path() -> Result<PathBuf> {
    Ok(storage::get_data_dir()?.join("update-check.json"))
}

fn load_cache() -> Cache {
    (|| -> Result<Cache> {
        let path = cache_path()?;
        if !path.exists() {
            return Ok(Cache::default());
        }
        let body = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&body).unwrap_or_default())
    })()
    .unwrap_or_default()
}

/// Never hard-fails: a cache write that can't land just means the next run
/// checks again, which is harmless.
fn save_cache(cache: &Cache) {
    let _ = (|| -> Result<()> {
        let path = cache_path()?;
        let body = serde_json::to_string_pretty(cache)?;
        let temp_path = path.with_extension("json.tmp");
        std::fs::write(&temp_path, body)?;
        std::fs::rename(&temp_path, &path)?;
        Ok(())
    })();
}

/// The gate: everything that can turn the check off entirely, independent of
/// the throttle. `TT_SKIP_UPDATE_CHECK` is the explicit opt-out (and what the
/// integration test harness sets, so `cargo test` never touches the network);
/// `CI` is the standard convention most CI runners already export.
fn should_run_at_all() -> bool {
    if std::env::var_os("TT_SKIP_UPDATE_CHECK").is_some() {
        return false;
    }
    if std::env::var_os("CI").is_some() {
        return false;
    }
    crate::config::updates_enabled(&crate::config::load().general)
}

/// Whether the cache is stale enough to justify a real network call.
fn should_check_now(cache: &Cache, now: DateTime<Utc>) -> bool {
    match cache.last_checked {
        None => true,
        Some(last) => now - last >= CHECK_INTERVAL,
    }
}

/// True inside a Flatpak sandbox, where `/app` is a read-only bind-mount and
/// self-replacement can't work — updates there go through `flatpak update`.
pub fn running_as_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

/// A version strictly newer than `current`, or `None` if unparsable/not newer.
fn newer_version(candidate: &str, current: &str) -> Option<String> {
    let candidate_semver = semver::Version::parse(candidate.trim_start_matches('v')).ok()?;
    let current_semver = semver::Version::parse(current.trim_start_matches('v')).ok()?;
    (candidate_semver > current_semver).then(|| candidate.to_string())
}

/// The one function `main` calls: at most once per [`CHECK_INTERVAL`], and
/// bounded to [`FETCH_TIMEOUT`], looks for a newer release and returns its
/// version if there is one worth telling the user about. Never returns an
/// error — a broken network is silent, not a startup failure.
pub fn maybe_check_and_notify(current_version: &str) -> Option<String> {
    if !should_run_at_all() {
        return None;
    }

    let mut cache = load_cache();
    let now = Utc::now();

    if !should_check_now(&cache, now) {
        return cache
            .latest_version
            .as_deref()
            .and_then(|v| newer_version(v, current_version));
    }

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(fetch_latest_release_tag());
    });

    match rx.recv_timeout(FETCH_TIMEOUT) {
        // Reaching GitHub at all — even to learn there's nothing published
        // yet — is a completed check: cache it so the next invocation is
        // throttled, same as when a real version comes back.
        Ok(Ok(latest)) => {
            cache.last_checked = Some(now);
            cache.latest_version = latest.clone();
            save_cache(&cache);
            latest.and_then(|v| newer_version(&v, current_version))
        }
        // Timed out, or the fetch itself failed (offline, DNS, etc.): leave
        // `last_checked` alone so the very next invocation still retries.
        _ => None,
    }
}

/// `Ok(None)` means the fetch succeeded but no release exists yet — a
/// completed check, distinct from `Err` (the fetch itself failed).
fn fetch_latest_release_tag() -> Result<Option<String>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()?
        .fetch()?;
    Ok(releases.first().map(|r| r.version.clone()))
}

/// The CTA `tt update` (or `flatpak update`, inside the sandbox) that a
/// startup notice or `tt update --check` points the user at.
pub fn update_hint() -> &'static str {
    if running_as_flatpak() {
        "flatpak update"
    } else {
        "tt update"
    }
}

/// `tt update` itself.
pub fn perform_update(check_only: bool, yes: bool) -> Result<()> {
    if running_as_flatpak() && !check_only {
        println!("You're running the Flatpak build — update with `flatpak update` instead.");
        return Ok(());
    }

    let current_version = env!("CARGO_PKG_VERSION");

    if check_only {
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .build()?
            .fetch()?;
        // A manual check is a completed check too — cache it so the passive
        // startup notice (and the next invocation's throttle) reflect it.
        save_cache(&Cache {
            last_checked: Some(Utc::now()),
            latest_version: releases.first().map(|r| r.version.clone()),
        });
        return match releases.first() {
            Some(latest) if newer_version(&latest.version, current_version).is_some() => {
                println!(
                    "tt {} is available (you have {current_version}). Run `tt update` to upgrade.",
                    latest.version
                );
                Ok(())
            }
            Some(latest) => {
                println!(
                    "tt {current_version} is up to date (latest: {}).",
                    latest.version
                );
                Ok(())
            }
            None => {
                println!("No releases have been published yet.");
                Ok(())
            }
        };
    }

    let status = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(current_version)
        .show_download_progress(true)
        .no_confirm(yes)
        .build()?
        .update()?;

    match status {
        self_update::Status::UpToDate(v) => println!("tt {v} is already up to date."),
        self_update::Status::Updated(v) => println!("Updated to tt {v}."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{env_guard, env_sandbox};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn should_check_now_when_never_checked() {
        assert!(should_check_now(&Cache::default(), at(1_000_000)));
    }

    #[test]
    fn should_not_check_within_the_interval() {
        let cache = Cache {
            last_checked: Some(at(1_000_000)),
            latest_version: None,
        };
        assert!(!should_check_now(&cache, at(1_000_000 + 23 * 3600)));
    }

    #[test]
    fn should_check_once_the_interval_has_passed() {
        let cache = Cache {
            last_checked: Some(at(1_000_000)),
            latest_version: None,
        };
        assert!(should_check_now(&cache, at(1_000_000 + 24 * 3600)));
    }

    #[test]
    fn newer_version_requires_a_strictly_greater_semver() {
        assert_eq!(newer_version("0.2.0", "0.1.0"), Some("0.2.0".to_string()));
        assert_eq!(newer_version("v0.2.0", "0.1.0"), Some("v0.2.0".to_string()));
        assert_eq!(newer_version("0.1.0", "0.1.0"), None);
        assert_eq!(newer_version("0.1.0", "0.2.0"), None);
        assert_eq!(newer_version("not-a-version", "0.1.0"), None);
    }

    #[test]
    fn the_skip_env_var_disables_the_check_regardless_of_config() {
        let _guard = env_guard();
        env_sandbox("update-skip-env");
        unsafe { std::env::set_var("TT_SKIP_UPDATE_CHECK", "1") };
        let result = should_run_at_all();
        unsafe { std::env::remove_var("TT_SKIP_UPDATE_CHECK") };
        assert!(!result);
    }

    #[test]
    fn cache_round_trips_through_save_and_load() {
        let _guard = env_guard();
        env_sandbox("update-cache-roundtrip");
        let cache = Cache {
            last_checked: Some(at(42)),
            latest_version: Some("1.2.3".to_string()),
        };
        save_cache(&cache);
        let loaded = load_cache();
        assert_eq!(loaded.last_checked, cache.last_checked);
        assert_eq!(loaded.latest_version, cache.latest_version);
    }

    #[test]
    fn a_missing_cache_file_loads_as_default() {
        let _guard = env_guard();
        env_sandbox("update-cache-missing");
        let loaded = load_cache();
        assert!(loaded.last_checked.is_none());
        assert!(loaded.latest_version.is_none());
    }
}
