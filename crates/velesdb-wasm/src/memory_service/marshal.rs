//! Id, metadata, link and filter marshalling between JS values and the memory types.

use super::errors::{invalid_input, to_js_err};
use serde_json::Value;
use velesdb_memory::limits::{self, whole_count};
use velesdb_memory::{ColumnFilter, ColumnOp, FusionOptions, MemoryError, Metadata};
use wasm_bindgen::{JsCast, JsValue};

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

// --- Count arguments ---------------------------------------------------
//
// Every count crosses the boundary as a JS number (`f64`), never as a
// `usize`/`u64` parameter: `wasm-bindgen` coerces those silently (`Infinity`
// and `NaN` → 0, `-1` → 4294967295, `2.9` → 2, a `-1n` TTL → `u64::MAX`,
// "never expires"). Each one is checked by the validator the Node binding
// calls too, `velesdb_memory::limits::whole_count`, and refused as
// `INVALID_INPUT`. These helpers stop before any `JsValue`, so they are
// testable natively (`marshal_tests.rs`).

/// A count that predates this rule as a `u64` parameter (`ttlSeconds`,
/// `reserveTokens`), which JavaScript had to pass as a `BigInt`: it now takes
/// a number, as the Node binding does, and still a `BigInt`, so callers
/// written for the old signature (the TypeScript SDK among them) keep
/// working. Either is read as `f64` for [`whole_count`] — a `BigInt` past
/// 2^53 loses precision there, but it is past `u32::MAX` already and refused
/// the same. `undefined`/`null` is omitted; any other type is refused.
pub(super) fn number_or_bigint(name: &str, value: Option<JsValue>) -> Result<Option<f64>, JsValue> {
    let Some(value) = value.filter(|v| !v.is_undefined() && !v.is_null()) else {
        return Ok(None);
    };
    if let Some(number) = value.as_f64() {
        return Ok(Some(number));
    }
    let refused = || invalid_input(format!("{name} must be a number or a BigInt"));
    let big = value.dyn_into::<js_sys::BigInt>().map_err(|_| refused())?;
    let digits = String::from(big.to_string(10).map_err(|_| refused())?);
    digits.parse::<f64>().map(Some).map_err(|_| refused())
}

/// `k` of a recall: default 10, capped at the recall limit.
pub(super) fn recall_limit(k: Option<f64>) -> Result<usize, MemoryError> {
    let k = whole_count("k", k)?.map_or(10, |k| k as usize);
    Ok(limits::clamp_recall_limit(k))
}

/// `maxHops` of `why`: default [`limits::DEFAULT_WHY_HOPS`], capped.
pub(super) fn why_hops(max_hops: Option<f64>) -> Result<usize, MemoryError> {
    let hops = whole_count("maxHops", max_hops)?.map_or(limits::DEFAULT_WHY_HOPS, |h| h as usize);
    Ok(limits::clamp_hops(hops))
}

/// `ttlSeconds` of `remember`; omitted means permanent.
pub(super) fn ttl_seconds(ttl: Option<f64>) -> Result<Option<u64>, MemoryError> {
    Ok(whole_count("ttlSeconds", ttl)?.map(u64::from))
}

/// `fragmentIndex` of `explainCompilation`.
pub(super) fn fragment_index(index: Option<f64>) -> Result<Option<usize>, MemoryError> {
    Ok(whole_count("fragmentIndex", index)?.map(|i| i as usize))
}

/// `reserveTokens` of `suggestBudget`: default 0.
pub(super) fn reserve_tokens(reserve: Option<f64>) -> Result<u64, MemoryError> {
    Ok(u64::from(
        whole_count("reserveTokens", reserve)?.unwrap_or(0),
    ))
}

/// `{hops?, graphBoost?, pool?}` → [`FusionOptions`], through
/// [`FusionOptions::from_knobs`] — the constructor the Node, Python and MCP
/// surfaces use — so the defaults and clamps live in one place.
pub(super) fn to_fusion_input(
    hops: Option<f64>,
    graph_boost: Option<f64>,
    pool: Option<f64>,
) -> Result<FusionOptions, MemoryError> {
    Ok(FusionOptions::from_knobs(
        whole_count("hops", hops)?.map(|h| h as usize),
        graph_boost,
        whole_count("pool", pool)?.map(|p| p as usize),
    ))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FusionOptionsInput {
    hops: Option<f64>,
    graph_boost: Option<f64>,
    pool: Option<f64>,
}

pub(super) fn to_fusion_options(value: JsValue) -> Result<FusionOptions, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(FusionOptions::default());
    }
    let input: FusionOptionsInput = serde_wasm_bindgen::from_value(value)
        .map_err(|e| invalid_input(format!("invalid fusion options: {e}")))?;
    to_fusion_input(input.hops, input.graph_boost, input.pool).map_err(to_js_err)
}

#[cfg(test)]
#[path = "marshal_tests.rs"]
mod tests;
