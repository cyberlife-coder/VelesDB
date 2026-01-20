//! Text search utilities for `VelesDB` WASM.
//!
//! Provides simple substring-based text search on JSON payloads.

use serde_json::Value;

/// Checks if payload contains text in specified field or any string field.
///
/// # Arguments
///
/// * `payload` - JSON payload to search
/// * `query` - Lowercase query string to find
/// * `field` - Optional specific field to search in
///
/// # Returns
///
/// `true` if the query is found in the payload.
pub fn payload_contains_text(payload: &Value, query: &str, field: Option<&str>) -> bool {
    if let Some(field_name) = field {
        if let Some(value) = payload.get(field_name) {
            return value_contains_text(value, query);
        }
        false
    } else {
        search_all_fields(payload, query)
    }
}

/// Recursively searches all string fields in a JSON value.
///
/// # Arguments
///
/// * `value` - JSON value to search
/// * `query` - Lowercase query string to find
///
/// # Returns
///
/// `true` if the query is found in any string field.
pub fn search_all_fields(value: &Value, query: &str) -> bool {
    match value {
        Value::String(s) => s.to_lowercase().contains(query),
        Value::Object(obj) => obj.values().any(|v| search_all_fields(v, query)),
        Value::Array(arr) => arr.iter().any(|v| search_all_fields(v, query)),
        _ => false,
    }
}

/// Checks if a value contains the query text.
///
/// # Arguments
///
/// * `value` - JSON value to check
/// * `query` - Lowercase query string to find
///
/// # Returns
///
/// `true` if the query is found in the value.
pub fn value_contains_text(value: &Value, query: &str) -> bool {
    match value {
        Value::String(s) => s.to_lowercase().contains(query),
        Value::Array(arr) => arr.iter().any(|v| value_contains_text(v, query)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_payload_contains_text_specific_field() {
        let payload = json!({"title": "Hello World", "content": "Some text"});
        assert!(payload_contains_text(&payload, "hello", Some("title")));
        assert!(!payload_contains_text(&payload, "hello", Some("content")));
    }

    #[test]
    fn test_payload_contains_text_all_fields() {
        let payload = json!({"title": "Hello", "content": "World"});
        assert!(payload_contains_text(&payload, "hello", None));
        assert!(payload_contains_text(&payload, "world", None));
    }

    #[test]
    fn test_search_all_fields_nested() {
        let payload = json!({
            "metadata": {
                "author": "John Doe",
                "tags": ["rust", "wasm"]
            }
        });
        assert!(search_all_fields(&payload, "john"));
        assert!(search_all_fields(&payload, "rust"));
    }

    #[test]
    fn test_value_contains_text_array() {
        let value = json!(["apple", "banana", "cherry"]);
        assert!(value_contains_text(&value, "banana"));
        assert!(!value_contains_text(&value, "orange"));
    }

    #[test]
    fn test_case_insensitive() {
        let payload = json!({"name": "VelesDB"});
        assert!(payload_contains_text(&payload, "velesdb", None));
        assert!(payload_contains_text(
            &payload,
            "VELESDB".to_lowercase().as_str(),
            None
        ));
    }
}
