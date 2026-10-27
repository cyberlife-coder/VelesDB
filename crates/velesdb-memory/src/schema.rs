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

#[cfg(feature = "mcp")]
mod ids;
#[cfg(feature = "mcp")]
mod inline;
#[cfg(feature = "mcp")]
mod postcondition;
#[cfg(feature = "mcp")]
mod scalarize;
/// The rustdoc-link rewrite and the id widening (see the module doc).
#[cfg(feature = "mcp")]
mod walks;
#[cfg(feature = "mcp")]
mod wire;

#[cfg(feature = "mcp")]
pub(crate) use postcondition::untyped_input_slots;
#[cfg(feature = "mcp")]
pub(crate) use wire::{reharden_tool_input, wire_safe_input_schema, wire_safe_output_schema};

/// Les champs d'id qui traversent le fil sous forme entiere OU chaine
/// decimale.
///
/// Seule declaration (#1685) : [`crate::context::wire::ID_KEYS`] la
/// reexporte telle quelle plutot que de la redeclarer. Vit ici et non dans
/// `context::wire` parce que `schema.rs` n'est jamais gate par une feature
/// (voir `mod schema;` dans `lib.rs`), alors que le module `context` entier
/// l'est sur `context` — et les outils de `mcp.rs` en ont besoin meme quand
/// `context` est absente, ce qui est le cas de `--features http`. Gatee sur
/// `any(mcp, context)` et non laissee inconditionnelle : un build
/// `--features persistence` seule (verifie en isolation par la CI, voir
/// `ci.yml`) n'a besoin d'aucune des deux, et `-D warnings` transformerait
/// la constante alors inutilisee en echec de build.
#[cfg(any(feature = "mcp", feature = "context"))]
pub const WIRE_ID_KEYS: &[&str] = &["fragment_id", "content_hash", "memory_id", "fragment_ids"];

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
