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

/// Cap on how deep [`inline_ref_only_properties`] chases nested `$ref`s.
/// The deepest real input schema needs 3 (`working` → `items` of
/// `ContextFact` → its `source`); 8 leaves headroom while keeping a
/// mutually-recursive type from expanding without bound even if the
/// [`InlineChain`] guard were ever bypassed.
#[cfg(feature = "mcp")]
const MAX_INLINE_DEPTH: usize = 8;

/// The `$defs` names currently being inlined, as a PATH stack (pushed on
/// descent, popped on the way out) — not a global visited-set: a global set
/// would leave the second and third `Vec<ContextFact>` of a
/// [`WorkingContext`](crate::context::WorkingContext) un-inlined just
/// because a sibling got there first. A name already on the stack means a
/// reference cycle, and its `$ref` is left intact.
#[cfg(feature = "mcp")]
type InlineChain = Vec<String>;

/// Inline every schema slot that is a bare `$ref` (or a single-element
/// `allOf` wrapping one) into the referenced `$defs` entry, RECURSIVELY and
/// through `items`, so each slot carries a DIRECT `type` keyword.
///
/// Real MCP client harnesses (observed 2026-07-24 with Claude Code) degrade
/// a `$ref`-only parameter to "untyped" and then serialize the argument as a
/// JSON-encoded string — `save_working_context`'s `working` object arrived
/// as `"{\"goal\": ...}"` and failed with `invalid type: string, expected
/// struct WorkingContext`. Same wire-contract class as the #1468 float-lossy
/// id fix: the advertised schema must be harness-proof, not merely
/// spec-correct.
///
/// Chasing only ONE level (the 2026-07-24 shape) left the identical hole one
/// step deeper, and it cost four round trips of deserialization errors on
/// 2026-07-26: `working.decisions` advertised `items: {"$ref":
/// "#/$defs/ContextDecisionRef"}`, so a `$defs`-blind harness saw "array of
/// anything" and never learned that `rule_id` (or `SourceReference`'s
/// `handle`) is required. Hence the same tree walk as
/// [`widen_id_properties`] — `properties`, `items`, then a generic descent —
/// bounded by [`MAX_INLINE_DEPTH`] and an [`InlineChain`] cycle guard.
///
/// Sibling keywords on the slot (e.g. `description`) override the inlined
/// definition's; a slot that already exposes a `type` is not inlined, but is
/// still descended into — that is exactly the `{"type": "array", "items":
/// {"$ref": …}}` case. `$defs` is left in place and is NOT expanded on the
/// spot: spec-correct clients still resolve through it, it stays a
/// resolvable target for any `$ref` a bound leaves behind, and expanding it
/// would only inflate the advertised schema.
///
/// `mcp`-gated: the advertised tool schemas are its only consumer.
#[cfg(feature = "mcp")]
pub(crate) fn inline_ref_only_properties(map: &mut Map<String, Value>) {
    let Some(Value::Object(defs)) = map.get("$defs").cloned() else {
        return;
    };
    let mut chain = InlineChain::new();
    inline_in_map(map, &defs, &mut chain, 0);
    prune_unreferenced_defs(map);
}

/// Drop every `$defs` entry no `$ref` can still reach.
///
/// Inlining copies a definition to each site that referenced it, so its
/// `$defs` entry usually becomes dead weight — measured at **55 % of the
/// published schema bytes** (83 KB of 148 KB across the 18 tools) once both
/// the input and output sides were inlined. Keeping it costs every client
/// that reads `tools/list`, and the whole point of inlining was that a
/// `$defs`-blind client never looks there.
///
/// Only *unreachable* entries go: 65 `$ref`s legitimately survive (a bound a
/// cycle guard stopped, an `anyOf` arm), and a definition still reachable —
/// directly or through another definition — is kept. The fixpoint below is
/// what makes that transitive: dropping one entry can orphan another.
#[cfg(feature = "mcp")]
fn prune_unreferenced_defs(map: &mut Map<String, Value>) {
    let Some(Value::Object(defs)) = map.get("$defs") else {
        return;
    };
    let names: Vec<String> = defs.keys().cloned().collect();
    let defs = defs.clone();

    let mut live: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // Seed with everything referenced outside `$defs` itself.
    let mut outside = map.clone();
    outside.remove("$defs");
    collect_refs(&Value::Object(outside), &mut live);

    // Fixpoint: a live definition's own `$ref`s keep their targets alive.
    loop {
        let mut grown = false;
        for name in &names {
            if !live.contains(name) {
                continue;
            }
            if let Some(def) = defs.get(name) {
                let before = live.len();
                collect_refs(def, &mut live);
                grown |= live.len() != before;
            }
        }
        if !grown {
            break;
        }
    }

    if let Some(Value::Object(defs)) = map.get_mut("$defs") {
        defs.retain(|name, _| live.contains(name));
        if defs.is_empty() {
            map.remove("$defs");
        }
    }
}

/// Every `#/$defs/<name>` target reachable from `value`.
#[cfg(feature = "mcp")]
fn collect_refs(value: &Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get("$ref") {
                if let Some(name) = target.strip_prefix("#/$defs/") {
                    out.insert(name.to_owned());
                }
            }
            for sub in map.values() {
                collect_refs(sub, out);
            }
        }
        Value::Array(entries) => {
            for sub in entries {
                collect_refs(sub, out);
            }
        }
        _ => {}
    }
}

/// One schema node: inline its `properties` and `items` slots, then descend
/// generically into every other keyword — `$defs` excepted (see
/// [`inline_ref_only_properties`]).
#[cfg(feature = "mcp")]
fn inline_in_map(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    if depth >= MAX_INLINE_DEPTH {
        return;
    }
    inline_property_slots(map, defs, chain, depth);
    inline_item_slots(map, defs, chain, depth);
    inline_union_branches(map, defs, chain, depth);
    inline_remaining_keywords(map, defs, chain, depth);
}

/// The keywords `inline_in_map` walks as argument paths, each with its own
/// pass — listed once here so the generic descent cannot walk them twice.
#[cfg(feature = "mcp")]
const SLOT_KEYWORDS: [&str; 6] = [
    "$defs",
    "properties",
    "items",
    "anyOf",
    "oneOf",
    "prefixItems",
];

#[cfg(feature = "mcp")]
fn inline_property_slots(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for slot in properties.values_mut() {
            inline_slot(slot, defs, chain, depth);
        }
    }
}

#[cfg(feature = "mcp")]
fn inline_item_slots(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    match map.get_mut("items") {
        // Tuple form: `items` is an array of per-position schemas.
        Some(Value::Array(entries)) => {
            for entry in entries {
                inline_slot(entry, defs, chain, depth);
            }
        }
        Some(single) => inline_slot(single, defs, chain, depth),
        None => {}
    }
}

/// Union branches are argument paths too.
///
/// `Option<T>` is the common case: schemars renders it
/// `anyOf: [{"$ref": …}, {"type": "null"}]`, and a `$defs`-blind harness that
/// cannot resolve the first branch degrades the WHOLE slot to "anything" —
/// which is how `source`, `goal` and `memory_id` reached callers as a bare
/// `{}` (2026-07-28). Treating each branch as a slot is what makes an
/// optional field as self-describing as a required one.
///
/// `allOf` stays out on purpose: [`ref_only_target`] already reads its
/// single-element form as a wrapper around the slot itself, and inlining it
/// here as a branch would fight that.
#[cfg(feature = "mcp")]
fn inline_union_branches(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    for keyword in ["anyOf", "oneOf", "prefixItems"] {
        if let Some(Value::Array(branches)) = map.get_mut(keyword) {
            for branch in branches {
                inline_slot(branch, defs, chain, depth);
            }
        }
    }
}

/// Generic descent for everything the dedicated passes did not claim.
#[cfg(feature = "mcp")]
fn inline_remaining_keywords(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    for (key, value) in map.iter_mut() {
        if !SLOT_KEYWORDS.contains(&key.as_str()) {
            inline_in_value(value, defs, chain, depth);
        }
    }
}

#[cfg(feature = "mcp")]
fn inline_in_value(
    value: &mut Value,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    match value {
        Value::Object(map) => inline_in_map(map, defs, chain, depth),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| inline_in_value(item, defs, chain, depth)),
        _ => {}
    }
}

/// Inline one slot when it is `$ref`-only and its target is not already
/// being inlined higher up the path, then recurse into the resulting node
/// either way.
#[cfg(feature = "mcp")]
fn inline_slot(slot: &mut Value, defs: &Map<String, Value>, chain: &mut InlineChain, depth: usize) {
    let Value::Object(prop) = slot else {
        return;
    };
    if !prop.contains_key("type") {
        if let Some(name) = ref_only_target(prop) {
            if !chain.contains(&name) {
                if let Some(Value::Object(definition)) = defs.get(&name) {
                    let mut merged = definition.clone();
                    for (key, value) in prop.iter() {
                        if key != "$ref" && key != "allOf" {
                            merged.insert(key.clone(), value.clone());
                        }
                    }
                    *prop = merged;
                    chain.push(name);
                    inline_in_map(prop, defs, chain, depth + 1);
                    chain.pop();
                    return;
                }
            }
        }
    }
    inline_in_map(prop, defs, chain, depth + 1);
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

/// Les champs d'id qui traversent le fil sous forme entiere OU chaine
/// decimale. Definis ici et non dans `context::wire` : `schema.rs` est gate
/// sur `mcp`, alors que `context::wire` l'est sur `context` — et les outils
/// de `mcp.rs` en ont besoin meme quand `context` est absente, ce qui est le
/// cas de `--features http`.
#[cfg(feature = "mcp")]
pub(crate) const WIRE_ID_KEYS: &[&str] =
    &["fragment_id", "content_hash", "memory_id", "fragment_ids"];

/// Le schema de sortie ANNONCE d'un outil : ids elargis, puis `$ref` inlines
/// et `$defs` inatteignables elagues.
///
/// Vit ici plutot que dans `mcp/context_tools.rs` parce que les outils de
/// `mcp.rs` l'appellent aussi : le laisser dans un module gate sur `context`
/// cassait `--features http`, qui active `mcp` sans `context`.
#[cfg(feature = "mcp")]
pub(crate) fn wire_safe_output_schema<T: schemars::JsonSchema + std::any::Any>(
) -> std::sync::Arc<rmcp::model::JsonObject> {
    let schema = rmcp::handler::server::tool::schema_for_output::<T>().unwrap_or_else(|e| {
        panic!(
            "Invalid output schema for {}: {e}",
            std::any::type_name::<T>()
        )
    });
    let mut map = (*schema).clone();
    widen_id_properties(&mut map, WIRE_ID_KEYS);
    inline_ref_only_properties(&mut map);
    std::sync::Arc::new(map)
}
