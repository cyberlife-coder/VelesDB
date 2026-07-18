//! JSON-tree helpers for the id wire contract shared by every JS-facing
//! binding (Node, WASM) of the `context` types: a `u64` id crosses as a
//! decimal string, because JS `number` loses precision above 2^53.
//!
//! Node and WASM independently need the exact same tree walk over a
//! serialized [`CompiledContext`](super::CompiledContext) — one to turn
//! outgoing ids into strings, the other to turn incoming strings back into
//! numbers before deserializing. Living here once (instead of copy-pasted
//! per binding) means [`ID_KEYS`] has a single source of truth: a future id
//! field added to a `context` type only needs updating in one place, not
//! silently missed in whichever binding a copy-paste forgot.
//!
//! Deliberately `String`-erred, not binding-specific: this crate depends on
//! neither `napi` nor `wasm-bindgen`, so each binding maps the `String`
//! error to its own error type at the call site.

use serde_json::Value;

/// Object keys whose `u64` values (or arrays of them) must cross to JS as
/// decimal strings. Token counts stay numbers: they are bounded far below
/// 2^53 by the budget caps.
pub const ID_KEYS: &[&str] = &["fragment_id", "content_hash", "memory_id", "fragment_ids"];

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
/// Returns the offending text if an [`ID_KEYS`] field holds a string that
/// does not parse as a decimal `u64`.
pub fn parse_id_fields(value: &mut Value) -> Result<(), String> {
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
fn parse_ids_in(value: &mut Value) -> Result<(), String> {
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
/// Returns the offending text if a fragment's `id` does not parse as a
/// decimal `u64`.
pub fn parse_fragment_id_strings(request: &mut Value) -> Result<(), String> {
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

fn parse_u64(text: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("invalid id '{text}' (expected a decimal u64 string)"))
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
