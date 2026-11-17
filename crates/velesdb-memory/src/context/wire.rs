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
//! hands its input straight to serde. The string-to-number walks
//! (`parse_id_fields`, `parse_fragment_id_strings`) are deprecated.

use serde_json::Value;

/// An id that is not a decimal `u64` string: the error of the deprecated
/// [`parse_id_fields`] and [`parse_fragment_id_strings`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid id '{text}' (expected a decimal u64 string)")]
pub struct InvalidId {
    /// The offending text, as received.
    pub text: String,
}

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

/// The inverse of [`stringify_id_fields`]: recursively rewrite every
/// [`ID_KEYS`] field given in decimal-string form back into the numeric form
/// the domain types deserialize. Non-string id values pass through
/// untouched (serde reports them with its own error).
///
/// Deliberately stricter than serde: an [`ID_KEYS`]-named field with a
/// non-numeric string is rejected here even where serde would have dropped
/// it as an unknown field — a rejected typo beats a silently ignored one,
/// and an id key can never be user data on these wire shapes.
///
/// # Errors
/// Returns [`InvalidId`] if an [`ID_KEYS`] field holds a string that
/// does not parse as a decimal `u64`.
#[deprecated(
    since = "0.18.1",
    note = "every context type deserializes its ids from a number or a decimal string; hand the JSON to serde as is"
)]
pub fn parse_id_fields(value: &mut Value) -> Result<(), InvalidId> {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if ID_KEYS.contains(&key.as_str()) {
                    parse_ids_in(entry)?;
                } else {
                    parse_id_fields(entry)?;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                parse_id_fields(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Rewrite one id value (or an array of them) from decimal string to number.
fn parse_ids_in(value: &mut Value) -> Result<(), InvalidId> {
    match value {
        Value::String(text) => {
            *value = Value::Number(parse_u64(text)?.into());
        }
        Value::Array(items) => {
            for item in items {
                parse_ids_in(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Accept `fragments[].id` in decimal-string form by rewriting it to the
/// numeric wire form. The other [`ID_KEYS`] never appear in a compile
/// *request*, only in the output — a blanket rule over every `id` key would
/// corrupt caller metadata that happens to use that name.
///
/// # Errors
/// Returns [`InvalidId`] if a fragment's `id` does not parse as a
/// decimal `u64`.
#[deprecated(
    since = "0.18.1",
    note = "every context type deserializes its ids from a number or a decimal string; hand the JSON to serde as is"
)]
pub fn parse_fragment_id_strings(request: &mut Value) -> Result<(), InvalidId> {
    let Some(fragments) = request.get_mut("fragments").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for fragment in fragments {
        let Some(id) = fragment.get_mut("id") else {
            continue;
        };
        if let Value::String(text) = id {
            *id = Value::Number(parse_u64(text)?.into());
        }
    }
    Ok(())
}

pub(crate) fn parse_u64(text: &str) -> Result<u64, InvalidId> {
    text.parse().map_err(|_| InvalidId {
        text: text.to_owned(),
    })
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
