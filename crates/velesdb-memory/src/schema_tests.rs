//! Tests for [`strip_int_formats`](super::strip_int_formats).

use schemars::{schema_for, JsonSchema};
use serde_json::json;

use super::strip_int_formats;

#[test]
fn removes_rust_int_formats_but_keeps_standard_ones() {
    let mut schema: schemars::Schema = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "format": "uint64", "minimum": 0 },
            "ids": { "type": "array", "items": { "type": "integer", "format": "uint" } },
            "when": { "type": "string", "format": "date-time" }
        }
    }))
    .expect("valid schema");

    strip_int_formats(&mut schema);

    let value = serde_json::to_value(&schema).expect("serializable");
    assert!(value["properties"]["id"].get("format").is_none());
    assert!(value["properties"]["ids"]["items"].get("format").is_none());
    // The integer constraint survives; only the non-standard format is dropped.
    assert_eq!(value["properties"]["id"]["type"], "integer");
    assert_eq!(value["properties"]["id"]["minimum"], 0);
    // Standard formats are preserved.
    assert_eq!(value["properties"]["when"]["format"], "date-time");
}

#[derive(JsonSchema)]
#[schemars(transform = strip_int_formats)]
#[allow(dead_code)]
struct Sample {
    id: u64,
    hop: usize,
}

#[test]
fn derived_schema_has_no_int_format() {
    let schema = schema_for!(Sample);
    let text = serde_json::to_string(&schema).expect("serializable");
    assert!(
        !text.contains("\"format\""),
        "derived schema still carries an int format: {text}"
    );
}
