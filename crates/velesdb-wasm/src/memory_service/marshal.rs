//! Id, metadata, link and filter marshalling between JS values and the memory types.

use super::errors::invalid_input;
use serde_json::Value;
use velesdb_memory::{ColumnFilter, ColumnOp, FusionOptions, Metadata};
use wasm_bindgen::JsValue;

// --- Id / metadata / filter marshalling ---------------------------------

pub(super) fn id_to_string(id: u64) -> String {
    id.to_string()
}

pub(super) fn parse_id(s: &str) -> Result<u64, JsValue> {
    s.parse::<u64>()
        .map_err(|_| invalid_input(format!("invalid id '{s}' (expected a decimal u64 string)")))
}

/// Recursively rewrite every `context` id field (see
/// [`velesdb_memory::context::wire::ID_KEYS`]) of an outgoing JSON tree into
/// its decimal-string form. Shared with the Node binding via
/// `velesdb_memory::context::wire`, not duplicated here.
pub(super) fn stringify_id_fields(value: &mut Value) {
    velesdb_memory::context::wire::stringify_id_fields(value);
}

/// The inverse of [`stringify_id_fields`]: recursively rewrite every
/// `context` id field given in the binding's decimal-string form back into
/// the numeric form the domain types deserialize (used by
/// [`WasmMemoryService::save_working_context`](super::WasmMemoryService::save_working_context), the same helper the Node
/// binding applies before deserializing a `WorkingContext`). Shared with the
/// Node binding via `velesdb_memory::context::wire`, not duplicated here.
pub(super) fn parse_id_fields(value: &mut Value) -> Result<(), JsValue> {
    velesdb_memory::context::wire::parse_id_fields(value)
        .map_err(|error| invalid_input(error.to_string()))
}

/// Accept `fragments[].id` in decimal-string form (the Node binding's
/// contract, mirrored) by rewriting it to the numeric wire form.
pub(super) fn parse_fragment_id_strings(request: &mut Value) -> Result<(), JsValue> {
    velesdb_memory::context::wire::parse_fragment_id_strings(request)
        .map_err(|error| invalid_input(error.to_string()))
}

/// `undefined`/`null` → `None`; a plain object → `Some(Metadata)`; anything
/// else is a caller error.
pub(super) fn to_metadata(value: JsValue) -> Result<Option<Metadata>, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let parsed: Value = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid metadata/filter: {e}")))?;
    match parsed {
        Value::Object(map) => Ok(Some(map)),
        _ => Err(invalid_input("metadata/filter must be an object")),
    }
}

#[derive(serde::Deserialize)]
struct LinkInput {
    target: String,
    relation: String,
}

pub(super) fn to_links(value: JsValue) -> Result<Vec<velesdb_memory::Link>, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    let inputs: Vec<LinkInput> = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid links: {e}")))?;
    inputs
        .into_iter()
        .map(|l| {
            Ok(velesdb_memory::Link {
                target: parse_id(&l.target)?,
                relation: l.relation,
            })
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct ColumnFilterInput {
    field: String,
    op: String,
    value: Value,
}

fn parse_op(op: &str) -> Result<ColumnOp, JsValue> {
    match op {
        "eq" => Ok(ColumnOp::Eq),
        "ne" => Ok(ColumnOp::Ne),
        "lt" => Ok(ColumnOp::Lt),
        "le" => Ok(ColumnOp::Le),
        "gt" => Ok(ColumnOp::Gt),
        "ge" => Ok(ColumnOp::Ge),
        other => Err(invalid_input(format!(
            "invalid op '{other}' (expected eq|ne|lt|le|gt|ge)"
        ))),
    }
}

pub(super) fn to_filters(value: JsValue) -> Result<Vec<ColumnFilter>, JsValue> {
    let inputs: Vec<ColumnFilterInput> = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid filters: {e}")))?;
    inputs
        .into_iter()
        .map(|f| {
            Ok(ColumnFilter {
                field: f.field,
                op: parse_op(&f.op)?,
                value: f.value,
            })
        })
        .collect()
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FusionOptionsInput {
    hops: Option<usize>,
    graph_boost: Option<f64>,
    pool: Option<usize>,
}

pub(super) fn to_fusion_options(value: JsValue) -> Result<FusionOptions, JsValue> {
    let defaults = FusionOptions::default();
    if value.is_undefined() || value.is_null() {
        return Ok(defaults);
    }
    let input: FusionOptionsInput = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid fusion options: {e}")))?;
    Ok(FusionOptions {
        hops: velesdb_memory::limits::clamp_hops(input.hops.unwrap_or(defaults.hops)),
        graph_boost: input.graph_boost.unwrap_or(defaults.graph_boost),
        pool: input.pool.or(defaults.pool),
    })
}
