use anyhow::{Context, Result};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::tracker;

/// The per-OS directory `data.json` (and anything that wants to live beside
/// it, e.g. the update-check cache) lives in.
pub fn get_data_dir() -> Result<PathBuf> {
    let data_dir = crate::paths::env_or(std::env::var_os("TT_DATA_DIR"), crate::paths::data_dir())
        .context("Could not determine config directory")?;
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

pub fn get_data_path() -> Result<PathBuf> {
    Ok(get_data_dir()?.join("data.json"))
}

/// A cheap staleness fingerprint of a path — a file or a directory alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl PathStamp {
    /// `None` when `path` does not exist yet, or cannot be stat'd.
    pub fn read(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    /// Whether a caller may skip re-reading the path. Two missing paths are
    /// unchanged; an existing one must also be settled.
    pub fn unchanged(previous: Option<Self>, current: Option<Self>) -> bool {
        match current {
            Some(stamp) => current == previous && stamp.is_settled(),
            None => current == previous,
        }
    }

    /// False while the mtime is inside the current second, where one-second
    /// granularity can hide a same-second write from every later comparison.
    pub fn is_settled(&self) -> bool {
        let Some(modified) = self.modified else {
            return false;
        };
        match SystemTime::now().duration_since(modified) {
            Ok(age) => age >= Duration::from_secs(1),
            // mtime in the future (clock skew): compare stamps rather than
            // reload on every tick until the clock catches up.
            Err(_) => true,
        }
    }
}

/// The store file's stamp. `None` when the store does not exist yet.
pub fn store_stamp() -> Option<PathStamp> {
    PathStamp::read(&get_data_path().ok()?)
}

fn get_lock_path() -> Result<PathBuf> {
    Ok(get_data_path()?.with_extension("lock"))
}

/// Take an exclusive lock on the store, held until the returned file is dropped
fn lock_data() -> Result<File> {
    let path = get_lock_path()?;
    let lock = File::create(&path).context("Could not open lock file")?;
    lock.lock().context("Could not lock data file")?;
    Ok(lock)
}

pub fn load_data() -> Result<tracker::TimeData> {
    let path = get_data_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(tracker::TimeData::default())
    }
}

pub fn save_data(data: &tracker::TimeData) -> Result<()> {
    let path = get_data_path()?;
    let content = serde_json::to_string_pretty(data)?;

    // Temp file then rename, so a reader never sees a torn store
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, content)?;
    fs::rename(&temp_path, &path)?;
    Ok(())
}

/// Load the data, apply `edit`, and save the result under one exclusive lock —
/// one store transaction per mutation, which must not be split.
pub fn with_data<T>(edit: impl FnOnce(&mut tracker::TimeData) -> Result<T>) -> Result<T> {
    let _lock = lock_data()?;
    let mut data = load_data()?;
    let result = edit(&mut data)?;
    save_data(&data)?;
    Ok(result)
}

/// The one lock every test that repoints an env var (`HOME`, `TT_MARK_DIR`)
/// serialises against — env is process-wide, so do not add a second.
#[cfg(test)]
pub(crate) fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Points `HOME`, `TT_DATA_DIR`, `TT_MARK_DIR`, `TT_ACTIVITY_DIR` and
/// `TT_CONFIG_FILE` at a fresh scratch directory named after `name`. Callers
/// hold [`env_guard`].
#[cfg(test)]
pub(crate) fn env_sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tt-sandbox-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    unsafe { std::env::set_var("TT_DATA_DIR", dir.join("store")) };
    unsafe { std::env::set_var("TT_MARK_DIR", dir.join("marks")) };
    unsafe { std::env::set_var("TT_ACTIVITY_DIR", dir.join("activity")) };
    // Left unwritten: config::load() then falls back to defaults.
    unsafe { std::env::set_var("TT_CONFIG_FILE", dir.join("config.toml")) };
    let path = get_data_path().unwrap();
    assert!(
        path.starts_with(&dir),
        "sandbox TT_DATA_DIR not in effect: {path:?}"
    );
    let marks = crate::marks::mark_dir().expect("a mark dir");
    assert!(
        marks.starts_with(&dir),
        "sandbox TT_MARK_DIR not in effect: {marks:?}"
    );
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch path — never the real store or the real mark directory.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-stamp-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_stamp_taken_within_the_same_second_is_unsettled() {
        let dir = sandbox("settled");
        let file = dir.join("data.json");
        fs::write(&file, "{}").unwrap();

        // Freshly written, so the next write could leave this stamp identical.
        let fresh = PathStamp::read(&file).unwrap();
        assert!(!fresh.is_settled(), "a stamp of a just-written path");
        assert!(
            !PathStamp::unchanged(Some(fresh), Some(fresh)),
            "two identical unsettled stamps still mean 'reload'"
        );

        // The same stamp, once the second it was taken in has passed.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let settled = PathStamp::read(&file).unwrap();
        assert_eq!(fresh, settled, "the path did not change");
        assert!(settled.is_settled(), "a stamp older than a second");
        assert!(PathStamp::unchanged(Some(fresh), Some(settled)));
    }

    #[test]
    fn a_missing_path_stamps_as_none_and_compares_equal_to_itself() {
        let dir = sandbox("missing");
        let file = dir.join("nope.json");
        assert_eq!(PathStamp::read(&file), None);
        assert!(
            PathStamp::unchanged(None, None),
            "still missing is still unchanged"
        );

        fs::write(&file, "{}").unwrap();
        assert!(
            !PathStamp::unchanged(None, PathStamp::read(&file)),
            "appearing is a change"
        );
    }

    // `TT_DATA_DIR`'s override rule is `paths::env_or`, tested there. This
    // module composes no default of its own beyond `paths::data_dir`, so there
    // is nothing left here that was not a second copy of that test.
}
