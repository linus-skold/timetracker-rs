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

/// One display row. Every label already carries its indentation, so the renderer
/// only decides colour and where the value sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// `key        value` — the value in the detail popover's value column,
    /// aligned with every other field on the popover.
    Field { label: String, value: String },
    /// A label with nothing beside it: a list's name, or a bare bullet standing
    /// over the fields of one list item.
    Heading(String),
    /// `- value` — a list item, its value tight against the bullet rather than
    /// out in the value column, so a list reads as a list. The label is the
    /// indented dash; the renderer puts one space between the two.
    Bullet { label: String, value: String },
}

/// Two spaces per level of nesting, the same step the bullets are indented by.
const INDENT: &str = "  ";

/// Flatten the object into display rows, in the order the data was written.
///
/// Nested objects join their keys with `.`, so an object stays one row per leaf.
/// A list instead gets a heading of its own and one indented `- value` bullet
/// per item, which reads as a list rather than as `name[0]`, `name[1]`, …
pub fn rows(data: &Value) -> Vec<Row> {
    let mut out = Vec::new();
    // The top level is walked here rather than through `flatten`, so an empty
    // object is no rows at all instead of one `{}` row with no key.
    match data {
        Value::Object(map) => {
            for (key, child) in map {
                flatten(key.clone(), child, 0, &mut out);
            }
        }
        other => flatten(String::new(), other, 0, &mut out),
    }
    out
}

fn field(label: String, value: String) -> Row {
    Row::Field { label, value }
}

fn flatten(path: String, value: &Value, depth: usize, out: &mut Vec<Row>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", path, key)
                };
                flatten(path, child, depth, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            // A list nested straight inside a list item has no name of its own;
            // the bullet above it is heading enough, so its own bullets sit at
            // that depth rather than under an empty heading.
            let depth = if path.is_empty() {
                depth.saturating_sub(1)
            } else {
                out.push(Row::Heading(label(&format!("{}:", path), depth)));
                depth
            };
            for child in items {
                match child {
                    // A nested item gets a bare bullet, with its own rows
                    // indented under it, so `-` always starts exactly one item.
                    Value::Object(_) | Value::Array(_) => {
                        out.push(Row::Heading(label("-", depth + 1)));
                        flatten(String::new(), child, depth + 2, out);
                    }
                    // A scalar item is the bullet itself.
                    _ => out.push(Row::Bullet {
                        label: label("-", depth + 1),
                        value: scalar(child),
                    }),
                }
            }
        }
        // A leaf, plus the two empty containers — shown as themselves rather
        // than vanishing from the listing.
        _ => out.push(field(label(&path, depth), scalar(value))),
    }
}

fn label(text: &str, depth: usize) -> String {
    format!("{}{}", INDENT.repeat(depth), text)
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

    /// Rows as text: `label|value` for an aligned field, `label value` for a
    /// bullet, so both the indentation and which shape each row took are visible.
    fn shown(value: &Value) -> Vec<String> {
        rows(value)
            .into_iter()
            .map(|row| match row {
                Row::Field { label, value } => format!("{}|{}", label, value),
                Row::Heading(label) => label,
                Row::Bullet { label, value } => format!("{} {}", label, value),
            })
            .collect()
    }

    /// Objects stay one row per leaf, keyed by their dotted path.
    #[test]
    fn rows_flatten_objects_into_one_line_each() {
        assert_eq!(
            shown(&json!({"pr": 42, "review": {"by": "linus", "approved": true}})),
            vec!["pr|42", "review.by|linus", "review.approved|true"]
        );
    }

    /// Written order, not alphabetical: the keys above come back b-before-a.
    #[test]
    fn rows_keep_the_order_the_data_was_written_in() {
        let value: Value = serde_json::from_str(r#"{"zebra": 1, "apple": 2}"#).unwrap();
        assert_eq!(shown(&value), vec!["zebra|1", "apple|2"]);
    }

    /// A list is a heading plus one bullet per item, never `name[0]`.
    #[test]
    fn rows_render_an_array_as_a_heading_and_bullets() {
        assert_eq!(
            shown(&json!({"files": ["a.rs", "b.rs"], "pr": 42})),
            vec!["files:", "  - a.rs", "  - b.rs", "pr|42"]
        );
    }

    /// An item that is itself a container gets a bare bullet, with its own rows
    /// under it, so one `-` always starts exactly one item.
    #[test]
    fn rows_indent_the_fields_of_a_nested_list_item() {
        assert_eq!(
            shown(&json!({"reviews": [{"by": "linus", "ok": true}, "skipped"]})),
            vec![
                "reviews:",
                "  -",
                "    by|linus",
                "    ok|true",
                "  - skipped",
            ]
        );
    }

    #[test]
    fn rows_indent_a_list_inside_a_list() {
        assert_eq!(
            shown(&json!({"groups": [["a", "b"]]})),
            vec!["groups:", "  -", "    - a", "    - b"]
        );
    }

    #[test]
    fn empty_containers_still_get_a_row() {
        assert_eq!(shown(&json!({"a": {}, "b": []})), vec!["a|{}", "b|[]"]);
        assert!(rows(&json!({})).is_empty(), "nothing to show at all");
    }
}
