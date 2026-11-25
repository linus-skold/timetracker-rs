//! `tt export` and `tt import`: move time entries between machines as JSON.
//!
//! Only entries travel. Ids are machine-local, so an import gives each new
//! entry a local id and matches entries by `uuid`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::tracker::{SCHEMA_VERSION, TimeData, TimeEntry};
use crate::{naming, storage};

const FORMAT_MARKER: &str = "tt-export";
const FORMAT_VERSION: u64 = 1;
/// Older stores have no uuids, so a merge cannot match their entries.
const FIRST_SCHEMA_WITH_UUIDS: u32 = 2;
/// Path argument that means standard input.
const STDIN_PATH: &str = "-";

#[derive(Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u64,
    exported_at: DateTime<Local>,
    schema_version: u32,
    entries: Vec<TimeEntry>,
}

/// Only the fields that decide whether the rest can be read.
#[derive(Deserialize)]
struct ExportHeader {
    format: Option<String>,
    version: Option<u64>,
    schema_version: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImportSummary {
    pub added: usize,
    pub skipped: usize,
    /// Local entries that `--replace` removed.
    pub removed: usize,
    pub backup: Option<PathBuf>,
}

/// `tt export`: JSON to stdout, or to `output`.
pub fn export(output: Option<&Path>) -> Result<()> {
    if let Some(path) = output {
        refuse_live_store(path)?;
    }
    let data = storage::load_data()?;
    let json = render_export(&data, Local::now())?;
    match output {
        Some(path) => {
            std::fs::write(path, format!("{json}\n"))
                .with_context(|| format!("Could not write {}", path.display()))?;
            println!(
                "Exported {} entries to {}",
                data.entries.len(),
                path.display()
            );
        }
        None => writeln!(std::io::stdout(), "{json}")?,
    }
    Ok(())
}

/// `tt import`: read `source` (`-` is stdin) and merge or replace.
pub fn import(source: &str, replace: bool) -> Result<()> {
    let text = read_source(source)?;
    // Parse and check everything before the store lock is taken.
    let incoming = parse_export(&text)?;
    let summary = storage::with_data(|data| apply_import(data, incoming, replace))?;
    println!("{}", describe(&summary, replace));
    Ok(())
}

/// An export over `data.json` would destroy the store.
fn refuse_live_store(output: &Path) -> Result<()> {
    let store = storage::get_data_path()?;
    let same = match (output.canonicalize(), store.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => output == store,
    };
    if same {
        bail!("Refusing to overwrite the live store {}", store.display());
    }
    Ok(())
}

fn render_export(data: &TimeData, exported_at: DateTime<Local>) -> Result<String> {
    let file = ExportFile {
        format: FORMAT_MARKER.to_string(),
        version: FORMAT_VERSION,
        exported_at,
        schema_version: data.schema_version,
        entries: data.entries.clone(),
    };
    Ok(serde_json::to_string_pretty(&file)?)
}

fn read_source(source: &str) -> Result<String> {
    let bytes = if source == STDIN_PATH {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        bytes
    } else {
        std::fs::read(source).with_context(|| format!("Could not read {source}"))?
    };
    decode_source(bytes)
}

/// UTF-8 text without a BOM. Windows PowerShell 5.1 `>` writes UTF-16.
fn decode_source(bytes: Vec<u8>) -> Result<String> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        bail!(
            "The file is UTF-16, not UTF-8 (PowerShell `>` does this). \
             Export with `tt export --output <file>` instead."
        );
    }
    let text = String::from_utf8(bytes).context("The file is not valid UTF-8")?;
    Ok(text
        .strip_prefix('\u{FEFF}')
        .map(str::to_string)
        .unwrap_or(text))
}

/// The entries of a valid export file, normalised like a loaded store.
fn parse_export(text: &str) -> Result<Vec<TimeEntry>> {
    let header: ExportHeader =
        serde_json::from_str(text).context("Not a tt export file: invalid JSON")?;
    if header.format.as_deref() != Some(FORMAT_MARKER) {
        bail!("Not a tt export file: the \"format\" key must be \"{FORMAT_MARKER}\"");
    }
    match header.version {
        Some(FORMAT_VERSION) => {}
        Some(other) => bail!(
            "Unsupported export version {other}: this tt reads version {FORMAT_VERSION}. \
             Update tt, or export again with a matching tt."
        ),
        None => bail!("Not a tt export file: the \"version\" key is missing"),
    }
    match header.schema_version {
        Some(version) if version > SCHEMA_VERSION => bail!(
            "The file uses store schema {version}, but this tt reads schemas {FIRST_SCHEMA_WITH_UUIDS} to {SCHEMA_VERSION}. \
             Update tt."
        ),
        // Only one schema is in range. A schema bump must migrate incoming entries.
        Some(version) if version >= FIRST_SCHEMA_WITH_UUIDS => {}
        _ => bail!(
            "The file uses a store schema older than {FIRST_SCHEMA_WITH_UUIDS}, which has no \
             uuids to match on. Export again with a current tt."
        ),
    }
    let file: ExportFile = serde_json::from_str(text).context("Invalid export file")?;
    let mut data = TimeData {
        entries: file.entries,
        ..TimeData::default()
    };
    naming::normalize_store(&mut data);
    check_identities(&data.entries)?;
    Ok(data.entries)
}

/// Every entry needs its own uuid, or a merge cannot tell entries apart.
fn check_identities(entries: &[TimeEntry]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        if entry.uuid.is_nil() {
            bail!("Entry \"{}\" has no uuid", entry.description);
        }
        if !seen.insert(entry.uuid) {
            bail!("The uuid {} appears twice in the file", entry.uuid);
        }
    }
    Ok(())
}

/// Changes `data` only after every check passes.
fn apply_import(
    data: &mut TimeData,
    incoming: Vec<TimeEntry>,
    replace: bool,
) -> Result<ImportSummary> {
    if replace {
        replace_entries(data, incoming)
    } else {
        merge_entries(data, incoming)
    }
}

fn merge_entries(data: &mut TimeData, incoming: Vec<TimeEntry>) -> Result<ImportSummary> {
    let total = incoming.len();
    let known: std::collections::HashSet<_> = data.entries.iter().map(|e| e.uuid).collect();
    let fresh: Vec<TimeEntry> = incoming
        .into_iter()
        .filter(|e| !known.contains(&e.uuid))
        .collect();
    check_running(&fresh, data.active_entry().is_some())?;
    let added = fresh.len();
    append_with_new_ids(data, fresh);
    Ok(ImportSummary {
        added,
        skipped: total - added,
        removed: 0,
        backup: None,
    })
}

fn replace_entries(data: &mut TimeData, incoming: Vec<TimeEntry>) -> Result<ImportSummary> {
    check_running(&incoming, false)?;
    let backup = backup_store()?;
    let removed = data.entries.len();
    data.entries.clear();
    let added = incoming.len();
    append_with_new_ids(data, incoming);
    Ok(ImportSummary {
        added,
        skipped: 0,
        removed,
        backup,
    })
}

/// The store allows one running entry. Stopping one would invent an end time.
fn check_running(incoming: &[TimeEntry], local_is_running: bool) -> Result<()> {
    let running = incoming.iter().filter(|e| e.is_active()).count();
    if running > 1 || (running == 1 && local_is_running) {
        bail!(
            "The import would leave more than one running entry. \
             Run `tt stop` here, or stop the entry on the other machine, then import again."
        );
    }
    Ok(())
}

fn append_with_new_ids(data: &mut TimeData, entries: Vec<TimeEntry>) {
    for mut entry in entries {
        entry.id = data.next_id;
        data.next_id += 1;
        data.entries.push(entry);
    }
}

/// A copy of the store file, made inside the store lock.
fn backup_store() -> Result<Option<PathBuf>> {
    let path = storage::get_data_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    for attempt in 0u32.. {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let backup = path.with_extension(format!("json.bak-{stamp}{suffix}"));
        // create_new: never overwrite an earlier backup.
        let mut target = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Could not write backup {}", backup.display()));
            }
        };
        let mut source = std::fs::File::open(&path)?;
        std::io::copy(&mut source, &mut target)
            .with_context(|| format!("Could not write backup {}", backup.display()))?;
        return Ok(Some(backup));
    }
    unreachable!("the attempt counter does not end")
}

fn describe(summary: &ImportSummary, replace: bool) -> String {
    let mut text = if replace {
        format!(
            "Replaced {} local entries with {} imported entries",
            summary.removed, summary.added
        )
    } else {
        format!(
            "Added {}, skipped {} (already present)",
            summary.added, summary.skipped
        )
    };
    if let Some(backup) = &summary.backup {
        text.push_str(&format!("\nBackup: {}", backup.display()));
    }
    text
}

#[cfg(test)]
mod tests;
