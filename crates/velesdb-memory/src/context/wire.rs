//! JSON-tree helpers for the id wire contract shared by every JS-facing
//! binding (Node, WASM) of the `context` types: a `u64` id crosses as a
//! decimal string, because JS `number` loses precision above 2^53.
//!
//! Node and WASM need the exact same tree walk over a serialized
//! [`CompiledContext`](crate::context::CompiledContext) to turn outgoing ids
//! into strings. Living here once (instead of copy-pasted per binding) means
//! [`crate::context::wire::ID_KEYS`] has a single source of truth.
//!
//! The way in needs no walk: every id a binding reads back (a
//! `WorkingContext`, a `CompileRequest`'s `fragments[].id`) deserializes
//! from a number or a decimal string through the crate-wide id rule
//! (`crate::model::deserialize_id` and its `Option` sibling), so a binding
//! hands its input straight to serde.

use serde_json::Value;

/// Object keys whose `u64` values (or arrays of them) must cross to JS as
/// decimal strings. Token counts stay numbers: they are bounded far below
/// 2^53 by the budget caps.
///
/// Re-exported from [`crate::schema::WIRE_ID_KEYS`] rather than redeclared
/// (#1685): the two used to be independent constants with identical
/// contents, so a future id field added to one could silently miss the
/// other. `schema.rs` holds the single declaration because it compiles
/// unconditionally, while this module only compiles under the `context`
/// feature.
pub use crate::schema::WIRE_ID_KEYS as ID_KEYS;

/// Recursively rewrite every [`ID_KEYS`] field of a serialized `context`
/// wire value into its decimal-string form.
pub fn stringify_id_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if ID_KEYS.contains(&key.as_str()) {
                    stringify_ids_in(entry);
                } else {
                    stringify_id_fields(entry);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(stringify_id_fields),
        _ => {}
    }
}

/// Rewrite one id value (or an array of them) into decimal strings.
fn stringify_ids_in(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(id) = number.as_u64() {
                *value = Value::String(id.to_string());
            }
        }
        Value::Array(items) => items.iter_mut().for_each(stringify_ids_in),
        _ => {}
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
