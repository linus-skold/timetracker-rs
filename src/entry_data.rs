//! The optional `data` field on an entry: free-form JSON that users and agents
//! can hang extra information on. Parsed once at every boundary — `--data`, the
//! TUI form — so nothing downstream ever holds a string that isn't valid JSON.

use serde_json::Value;

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
    if !value.is_object() {
        return Err(format!(
            "expected a JSON object like {{\"key\": \"value\"}}, got {}",
            kind(&value)
        ));
    }
    Ok(Some(value))
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
