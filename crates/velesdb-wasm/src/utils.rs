//! Utility functions for WASM bindings.

use wasm_bindgen::prelude::*;

/// Validates vector dimension with custom prefix for error message.
#[inline]
pub fn validate_dimension_with_prefix(
    actual: usize,
    expected: usize,
    prefix: &str,
) -> Result<(), JsValue> {
    if actual != expected {
        return Err(JsValue::from_str(&format!(
            "{} dimension mismatch: expected {}, got {}",
            prefix, expected, actual
        )));
    }
    Ok(())
}

/// Converts search results to JsValue.
pub fn results_to_jsvalue<T: serde::Serialize>(results: &T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(results).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Formats search results with payload into JSON array.
pub fn format_payload_results(
    results: Vec<(u64, f32, Option<&serde_json::Value>)>,
) -> Vec<serde_json::Value> {
    results
        .into_iter()
        .map(|(id, score, payload)| {
            serde_json::json!({"id": id, "score": score, "payload": payload})
        })
        .collect()
}
