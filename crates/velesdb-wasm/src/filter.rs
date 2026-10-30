//! Metadata filtering for `VelesDB` WASM.
//!
//! A filter is `velesdb_core::filter::Filter` — parsed and evaluated by core,
//! so `search_with_filter` selects exactly what velesdb-server selects for
//! the same JSON, every condition kind included (`eq` … `geo_bbox`). The
//! shared fixture `conformance/filter_wire_cases.json` holds one case per
//! kind; core and this crate both run it.

use serde_json::Value;

/// Parses `filter` as core's `Filter`, or returns core's own error message.
///
/// `{}` and a flat field map such as `{"category": "tech"}` are refused
/// ("Invalid filter: missing field \`condition\`"), as velesdb-server
/// refuses them with `400`; so is `null`.
///
/// # Errors
///
/// Returns core's `Invalid filter: …` message when `filter` does not
/// deserialize as a core `Filter`.
pub fn parse_filter(filter: Value) -> Result<velesdb_core::Filter, String> {
    velesdb_core::Filter::from_json_value(filter)
}

/// Gets a nested field from a JSON payload using dot notation.
///
/// # Example
///
/// ```ignore
/// let payload = json!({"user": {"name": "John"}});
/// let name = get_nested_field(&payload, "user.name");
/// assert_eq!(name, Some(&json!("John")));
/// ```
pub fn get_nested_field<'a>(payload: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = payload;
    for part in field.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
