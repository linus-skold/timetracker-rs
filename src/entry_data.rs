//! The optional `data` field on an entry: free-form JSON that users and agents
//! can hang extra information on. Parsed once at every boundary — `--data`, the
//! TUI form — so nothing downstream ever holds a string that isn't valid JSON.

use serde_json::{Map, Value, json};

/// Parse one `--data` / form value. Blank means "no data"; anything else must be
/// a JSON **object**, since the detail view renders it as key/value rows and a
/// bare scalar or array has no keys to render.
pub fn parse(raw: &str) -> Result<Option<Value>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {}", first_line(&e)))?;
    require_object(value).map(Some)
}

/// The shared shape check: data is an object or it is nothing. `tt edit` uses it
/// for a JSON value that is already parsed.
pub fn require_object(value: Value) -> Result<Value, String> {
    if !value.is_object() {
        return Err(format!(
            "expected a JSON object like {{\"key\": \"value\"}}, got {}",
            kind(&value)
        ));
    }
    Ok(value)
}

/// Stamp `{"agent": {"label": <label>}}` into the caller's data, keeping every key
/// it already wrote — its own `agent.label` included.
pub fn with_agent_label(data: Option<Value>, label: &str) -> Result<Value, String> {
    with_agent_key(data, "label", label)
}

/// Stamp `{"agent": {"session": <id>}}` the same way. This is what makes an entry
/// cover the activity session it was logged for and no other.
pub fn with_agent_session(data: Option<Value>, session: &str) -> Result<Value, String> {
    with_agent_key(data, "session", session)
}

/// The activity session an entry was logged for, or `None` for one that names no
/// session — which the audit reads as covering every same-project session.
pub fn agent_session(data: Option<&Value>) -> Option<&str> {
    data?.get("agent")?.get("session")?.as_str()
}

/// One string key under the `agent` namespace, without disturbing anything else.
/// A key the caller already wrote wins.
fn with_agent_key(data: Option<Value>, key: &str, value: &str) -> Result<Value, String> {
    // Anything other than an object is unreachable: `parse` is the only producer.
    let mut object = match data {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let namespace = object.entry("agent").or_insert_with(|| json!({}));
    let Value::Object(namespace) = namespace else {
        return Err(format!(
            "expected \"agent\" to be a JSON object, got {}",
            kind(namespace)
        ));
    };
    namespace
        .entry(key)
        .or_insert_with(|| Value::String(value.to_string()));
    Ok(Value::Object(object))
}

/// serde_json errors are single-line already, but stay defensive: a message with
/// a newline in it would break the one-line form error row.
fn first_line(error: &serde_json::Error) -> String {
    error.to_string().lines().next().unwrap_or("").to_string()
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The stored data as compact JSON, for the single-line editor. Empty when unset,
/// so an untouched field round-trips back through [`parse`] as `None`.
pub fn to_edit_string(data: Option<&Value>) -> String {
    data.map(|v| v.to_string()).unwrap_or_default()
}

/// Flatten the object into display rows, one `(key, value)` per leaf. Nested
/// objects join their keys with `.` and arrays index with `[i]`, so every row is
/// one line however deep the data goes.
pub fn rows(data: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // The top level is walked here rather than through `flatten`, so an empty
    // object is no rows at all instead of one `{}` row with no key.
    match data {
        Value::Object(map) => {
            for (key, child) in map {
                flatten(key.clone(), child, &mut out);
            }
        }
        other => flatten(String::new(), other, &mut out),
    }
    out
}

fn flatten(prefix: String, value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                flatten(path, child, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (i, child) in items.iter().enumerate() {
                flatten(format!("{}[{}]", prefix, i), child, out);
            }
        }
        // A leaf, plus the two empty containers — shown as themselves rather
        // than vanishing from the listing.
        _ => out.push((prefix, scalar(value))),
    }
}

/// A leaf as it reads on screen: strings unquoted, everything else as JSON.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_blank_value_is_no_data_not_an_error() {
        assert_eq!(parse(""), Ok(None));
        assert_eq!(parse("   "), Ok(None));
    }

    #[test]
    fn an_object_parses_to_itself() {
        assert_eq!(
            parse(r#"{"pr": 42, "reviewed": true}"#),
            Ok(Some(json!({"pr": 42, "reviewed": true})))
        );
    }

    #[test]
    fn malformed_json_is_an_error_not_a_dropped_field() {
        for bad in [r#"{"a": }"#, "{", "not json", r#"{"a": 1,}"#] {
            assert!(parse(bad).is_err(), "`{bad}` parsed instead of failing");
        }
    }

    /// A scalar or array is valid JSON but has no keys to render.
    #[test]
    fn a_non_object_is_rejected_by_name() {
        assert_eq!(
            parse("[1, 2]"),
            Err("expected a JSON object like {\"key\": \"value\"}, got an array".to_string())
        );
        assert!(parse("42").unwrap_err().ends_with("got a number"));
        assert!(parse("\"hi\"").unwrap_err().ends_with("got a string"));
    }

    #[test]
    fn an_edit_string_round_trips_through_parse() {
        let value = json!({"issue": 69, "tags": ["a", "b"]});
        let edited = to_edit_string(Some(&value));
        assert_eq!(parse(&edited), Ok(Some(value)));
        assert_eq!(to_edit_string(None), "");
    }

    #[test]
    fn a_label_stamps_the_agent_namespace_onto_no_data_at_all() {
        assert_eq!(
            with_agent_label(None, "code"),
            Ok(json!({"agent": {"label": "code"}}))
        );
    }

    #[test]
    fn a_label_keeps_the_callers_own_keys_inside_and_outside_the_namespace() {
        assert_eq!(
            with_agent_label(
                Some(json!({
                    "pr": 42,
                    "agent": {"model": "opus", "tokens": {"input": 1200}},
                })),
                "code"
            ),
            Ok(json!({
                "pr": 42,
                "agent": {"label": "code", "model": "opus", "tokens": {"input": 1200}},
            }))
        );
    }

    #[test]
    fn a_label_the_caller_already_wrote_wins() {
        assert_eq!(
            with_agent_label(Some(json!({"agent": {"label": "mine"}})), "code"),
            Ok(json!({"agent": {"label": "mine"}}))
        );
    }

    #[test]
    fn a_namespace_that_is_not_an_object_is_rejected_by_name() {
        assert_eq!(
            with_agent_label(Some(json!({"agent": "code"})), "code"),
            Err("expected \"agent\" to be a JSON object, got a string".to_string())
        );
        assert!(
            with_agent_label(Some(json!({"agent": [1]})), "code")
                .unwrap_err()
                .ends_with("got an array")
        );
    }

    #[test]
    fn a_session_stamps_the_same_namespace_beside_the_label() {
        let labelled = with_agent_label(None, "code").unwrap();
        assert_eq!(
            with_agent_session(Some(labelled), "sess-1"),
            Ok(json!({"agent": {"label": "code", "session": "sess-1"}}))
        );
    }

    #[test]
    fn a_session_reads_back_out_of_the_namespace() {
        let stamped = with_agent_session(None, "sess-1").unwrap();
        assert_eq!(agent_session(Some(&stamped)), Some("sess-1"));
        assert_eq!(agent_session(None), None);
        assert_eq!(agent_session(Some(&json!({"pr": 42}))), None);
        assert_eq!(
            agent_session(Some(&json!({"agent": {"label": "code"}}))),
            None,
            "a labelled entry that names no session covers every session"
        );
    }

    #[test]
    fn rows_flatten_nesting_into_one_line_each() {
        let value = json!({
            "pr": 42,
            "review": {"by": "linus", "approved": true},
            "files": ["a.rs", "b.rs"],
        });
        assert_eq!(
            rows(&value),
            vec![
                ("files[0]".to_string(), "a.rs".to_string()),
                ("files[1]".to_string(), "b.rs".to_string()),
                ("pr".to_string(), "42".to_string()),
                ("review.approved".to_string(), "true".to_string()),
                ("review.by".to_string(), "linus".to_string()),
            ],
            "serde_json orders object keys alphabetically"
        );
    }

    #[test]
    fn empty_containers_still_get_a_row() {
        assert_eq!(
            rows(&json!({"a": {}, "b": []})),
            vec![
                ("a".to_string(), "{}".to_string()),
                ("b".to_string(), "[]".to_string()),
            ]
        );
        assert!(rows(&json!({})).is_empty(), "nothing to show at all");
    }
}
