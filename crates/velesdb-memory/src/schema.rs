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
