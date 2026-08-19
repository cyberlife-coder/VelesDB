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
#[path = "text_search_tests.rs"]
mod tests;
