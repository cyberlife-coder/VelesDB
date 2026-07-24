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

/// Inline every top-level property whose schema is a bare `$ref` (or a
/// single-element `allOf` wrapping one) into the referenced `$defs` entry,
/// so the property carries a DIRECT `type` keyword.
///
/// Real MCP client harnesses (observed 2026-07-24 with Claude Code) degrade
/// a `$ref`-only parameter to "untyped" and then serialize the argument as a
/// JSON-encoded string — `save_working_context`'s `working` object arrived
/// as `"{\"goal\": ...}"` and failed with `invalid type: string, expected
/// struct WorkingContext`. Same wire-contract class as the #1468 float-lossy
/// id fix: the advertised schema must be harness-proof, not merely
/// spec-correct. Sibling keywords on the property (e.g. `description`)
/// override the inlined definition's; properties that already expose a
/// `type` are left untouched. One level only — nested `$refs` inside the
/// inlined definition are not chased (only top-level tool parameters are
/// serialized one by one by a harness).
///
/// `mcp`-gated: the advertised tool schemas are its only consumer.
#[cfg(feature = "mcp")]
pub(crate) fn inline_ref_only_properties(map: &mut Map<String, Value>) {
    let Some(Value::Object(defs)) = map.get("$defs").cloned() else {
        return;
    };
    let Some(Value::Object(properties)) = map.get_mut("properties") else {
        return;
    };
    for subschema in properties.values_mut() {
        let Value::Object(prop) = subschema else {
            continue;
        };
        if prop.contains_key("type") {
            continue;
        }
        let Some(name) = ref_only_target(prop) else {
            continue;
        };
        let Some(Value::Object(definition)) = defs.get(&name) else {
            continue;
        };
        let mut merged = definition.clone();
        for (key, value) in prop.iter() {
            if key != "$ref" && key != "allOf" {
                merged.insert(key.clone(), value.clone());
            }
        }
        *prop = merged;
    }
}

/// Resolves the `#/$defs/<Name>` target of a `$ref`-only property schema:
/// either a direct `$ref` keyword or a single-element `allOf` wrapping one.
#[cfg(feature = "mcp")]
fn ref_only_target(prop: &Map<String, Value>) -> Option<String> {
    let reference = match (prop.get("$ref"), prop.get("allOf")) {
        (Some(Value::String(r)), _) => r.clone(),
        (None, Some(Value::Array(items))) if items.len() == 1 => match &items[0] {
            Value::Object(inner) => match inner.get("$ref") {
                Some(Value::String(r)) => r.clone(),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };
    reference.strip_prefix("#/$defs/").map(str::to_owned)
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
