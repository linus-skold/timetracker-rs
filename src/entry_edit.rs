//! The JSON view of one entry for `tt show`, and the partial patch `tt edit`
//! applies to it. Both speak the same shape, so a shown entry can be edited back.

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Local};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::entry_data;
use crate::tracker::{IdleInterval, TimeData, TimeEntry};

/// Keys that choose the entry. They never change it.
const SELECTOR_KEYS: [&str; 2] = ["id", "uuid"];
const EDITABLE_KEYS: [&str; 7] = [
    "description",
    "project",
    "tags",
    "start_time",
    "end_time",
    "idle",
    "data",
];

/// The entry as pretty JSON. `data` is always present, as `null` when unset.
pub fn entry_json(entry: &TimeEntry) -> Result<String> {
    let mut value = serde_json::to_value(entry)?;
    if let Some(object) = value.as_object_mut() {
        object.entry("data").or_insert(Value::Null);
    }
    Ok(serde_json::to_string_pretty(&value)?)
}

/// Parse the `--json` argument into a patch object.
pub fn parse_patch(raw: &str) -> Result<Map<String, Value>> {
    match serde_json::from_str(raw).map_err(|e| anyhow!("invalid JSON: {e}"))? {
        Value::Object(patch) => Ok(patch),
        _ => bail!("expected a JSON object with the keys to change"),
    }
}

/// Apply `patch` to the entry that `reference` or the patch's `id`/`uuid` names.
/// Nothing is stored unless every key is valid. Returns the entry's `id`.
pub fn edit_entry(
    data: &mut TimeData,
    reference: Option<&str>,
    patch: &Map<String, Value>,
) -> Result<u64> {
    reject_unknown_keys(patch)?;
    let index = resolve_target(data, reference, patch)?;

    let mut edited = data.entries[index].clone();
    for (key, value) in patch {
        apply_key(&mut edited, key, value)?;
    }
    let touches_span_or_idle = ["start_time", "end_time", "idle"]
        .iter()
        .any(|key| patch.contains_key(*key));
    check_edited(data, index, &edited, touches_span_or_idle)?;

    let id = edited.id;
    data.entries[index] = edited;
    Ok(id)
}

fn reject_unknown_keys(patch: &Map<String, Value>) -> Result<()> {
    let known = |key: &str| SELECTOR_KEYS.contains(&key) || EDITABLE_KEYS.contains(&key);
    match patch.keys().find(|key| !known(key)) {
        Some(key) => bail!(
            "unknown key \"{key}\"; editable keys: {}",
            EDITABLE_KEYS.join(", ")
        ),
        None => Ok(()),
    }
}

/// Every selector given must name the same entry.
fn resolve_target(
    data: &TimeData,
    reference: Option<&str>,
    patch: &Map<String, Value>,
) -> Result<usize> {
    let mut selectors: Vec<(&str, String)> = Vec::new();
    if let Some(reference) = reference {
        selectors.push(("reference", reference.to_string()));
    }
    for key in SELECTOR_KEYS {
        match patch.get(key) {
            None => {}
            Some(Value::String(text)) => selectors.push((key, text.clone())),
            Some(Value::Number(number)) => selectors.push((key, number.to_string())),
            Some(_) => bail!("\"{key}\" must be a string or a number"),
        }
    }
    if selectors.is_empty() {
        bail!("name the entry: pass a reference, or an \"id\" or \"uuid\" key in the JSON");
    }

    let mut target: Option<usize> = None;
    for (label, text) in &selectors {
        let index = data.position_by_ref(text).ok_or_else(|| {
            anyhow!("no entry matches {label} \"{text}\"; id and uuid only select an entry, they cannot change it")
        })?;
        match target {
            Some(previous) if previous != index => {
                bail!("the id, uuid and reference point at different entries")
            }
            _ => target = Some(index),
        }
    }
    Ok(target.expect("at least one selector"))
}

fn apply_key(entry: &mut TimeEntry, key: &str, value: &Value) -> Result<()> {
    match key {
        "description" => entry.description = parse_value(key, value)?,
        "project" => entry.project = parse_value(key, value)?,
        "tags" => entry.tags = parse_value(key, value)?,
        "start_time" => entry.start_time = parse_value::<DateTime<Local>>(key, value)?,
        "end_time" => entry.end_time = parse_value(key, value)?,
        "idle" => entry.idle = parse_value::<Vec<IdleInterval>>(key, value)?,
        "data" => entry.data = parse_data(value)?,
        // Selectors were checked when the target was resolved.
        _ => {}
    }
    Ok(())
}

fn parse_value<T: DeserializeOwned>(key: &str, value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|e| anyhow!("bad value for \"{key}\": {e}"))
}

fn parse_data(value: &Value) -> Result<Option<Value>> {
    if value.is_null() {
        return Ok(None);
    }
    entry_data::require_object(value.clone())
        .map(Some)
        .map_err(|e| anyhow!("bad value for \"data\": {e}"))
}

/// `tt log --idle` can store idle outside the span, so an edit that leaves
/// the span and idle alone must not fail on it.
fn check_idle_bounds(edited: &TimeEntry) -> Result<()> {
    for gap in &edited.idle {
        let inside =
            gap.start >= edited.start_time && edited.end_time.is_none_or(|end| gap.end <= end);
        if gap.end < gap.start || !inside {
            bail!("idle interval must have end after start and lie inside the entry");
        }
    }
    Ok(())
}

fn check_edited(
    data: &TimeData,
    index: usize,
    edited: &TimeEntry,
    touches_span_or_idle: bool,
) -> Result<()> {
    if let Some(end) = edited.end_time
        && end < edited.start_time
    {
        bail!("end_time is before start_time");
    }
    if touches_span_or_idle {
        check_idle_bounds(edited)?;
    }
    let other_active = data
        .entries
        .iter()
        .enumerate()
        .any(|(i, e)| i != index && e.is_active());
    if edited.is_active() && other_active {
        bail!("another entry is already active; at most one entry can be active");
    }
    Ok(())
}
