//! JSON Schema post-processing shared by the domain model and the MCP DTOs.
//!
//! `schemars` annotates Rust integer types with a `format` keyword (`"uint64"`
//! for `u64`, `"uint"` for `usize`, …). Those values are not standard JSON
//! Schema formats, so strict MCP clients log `unknown format "uint64" ignored`
//! for every integer field. The `type: integer` (plus the `minimum: 0` schemars
//! already emits for unsigned types) carries the constraint on its own, so the
//! non-standard `format` is pure noise — this transform strips it.

use schemars::Schema;
use serde_json::{Map, Value};

/// A `schemars` container transform that recursively removes Rust integer
/// `format` keywords from a generated schema. Apply with
/// `#[schemars(transform = crate::schema::strip_int_formats)]`.
pub(crate) fn strip_int_formats(schema: &mut Schema) {
    if let Some(object) = schema.as_object_mut() {
        strip_in_map(object);
    }
}

fn strip_in_map(map: &mut Map<String, Value>) {
    let drop_format = matches!(map.get("format"), Some(Value::String(f)) if is_rust_int_format(f));
    if drop_format {
        map.remove("format");
    }
    for value in map.values_mut() {
        strip_in_value(value);
    }
}

fn strip_in_value(value: &mut Value) {
    match value {
        Value::Object(map) => strip_in_map(map),
        Value::Array(items) => items.iter_mut().for_each(strip_in_value),
        _ => {}
    }
}

/// Recursively widen every property named in `keys` (resolving the `items`
/// of array-typed ones) from `integer` to `["integer", "string"]`, across
/// the whole schema tree — `$defs` included.
///
/// The advertised-schema counterpart of the `context::wire` id contract:
/// under `CompilePolicy::ids_as_strings` a response id field crosses as a
/// decimal string, and `fragments[].id` accepts one on input — and the
/// official MCP SDKs validate `structuredContent` against the advertised
/// `outputSchema` (spec 2025-06-18), so a schema typing those fields
/// `integer` only would make every opted-in response fail validation for
/// exactly the clients the option exists for. Same shape of tree walk as
/// [`strip_int_formats`], but keyed: only the named properties widen.
///
/// `mcp`-gated: the advertised tool schemas are its only consumer.
#[cfg(feature = "mcp")]
pub(crate) fn widen_id_properties(map: &mut Map<String, Value>, keys: &[&str]) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for (name, subschema) in properties.iter_mut() {
            if keys.contains(&name.as_str()) {
                widen_id_schema(subschema);
            }
        }
    }
    for value in map.values_mut() {
        widen_in_value(value, keys);
    }
}

#[cfg(feature = "mcp")]
fn widen_in_value(value: &mut Value, keys: &[&str]) {
    match value {
        Value::Object(map) => widen_id_properties(map, keys),
        Value::Array(items) => items.iter_mut().for_each(|item| widen_in_value(item, keys)),
        _ => {}
    }
}

/// Widen one id property's schema: `integer` → `["integer", "string"]`
/// (keeping any `null` of an optional field), recursing into `items` for an
/// array of ids. `minimum: 0` may stay — JSON Schema numeric keywords apply
/// to numbers only, so the string form is unaffected.
#[cfg(feature = "mcp")]
fn widen_id_schema(schema: &mut Value) {
    let Value::Object(map) = schema else {
        return;
    };
    match map.get("type").cloned() {
        Some(Value::String(kind)) if kind == "integer" => {
            map.insert(
                "type".to_owned(),
                Value::Array(vec![
                    Value::String("integer".to_owned()),
                    Value::String("string".to_owned()),
                ]),
            );
        }
        Some(Value::String(kind)) if kind == "array" => {
            if let Some(items) = map.get_mut("items") {
                widen_id_schema(items);
            }
        }
        Some(Value::Array(mut kinds)) => {
            let has_integer = kinds.iter().any(|kind| kind == "integer");
            let has_string = kinds.iter().any(|kind| kind == "string");
            if has_integer && !has_string {
                let after = kinds
                    .iter()
                    .position(|kind| kind == "integer")
                    .map_or(kinds.len(), |position| position + 1);
                kinds.insert(after, Value::String("string".to_owned()));
                map.insert("type".to_owned(), Value::Array(kinds));
            }
        }
        _ => {}
    }
}

fn is_rust_int_format(format: &str) -> bool {
    matches!(
        format,
        "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uint128"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "int128"
    )
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
