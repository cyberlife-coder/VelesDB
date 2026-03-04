//! Shared text extraction utilities for BM25 indexing.
//!
//! This module is the single source of truth for JSON payload → text conversion.
//! Previously duplicated between `collection/types.rs` (as `Collection::extract_text_from_payload`)
//! and `engine/payload.rs` (as a free `extract_text` function).

/// Extracts all string leaf values from a JSON payload, joined by spaces.
///
/// Used by BM25 indexing in both `PayloadEngine` and `Collection`.
pub(crate) fn extract_text(value: &serde_json::Value) -> String {
    let mut texts = Vec::new();
    collect_strings(value, &mut texts);
    texts.join(" ")
}

fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_strings(item, out);
            }
        }
        serde_json::Value::Object(obj) => {
            for v in obj.values() {
                collect_strings(v, out);
            }
        }
        _ => {}
    }
}
