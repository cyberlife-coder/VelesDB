//! Anti-corruption marshalling between JS-facing types and `velesdb_memory`
//! domain types. This module (with [`crate::dto`]) is the only place that names
//! both worlds, so the dependency boundary is auditable by inspection.

use serde_json::Value;
use velesdb_memory::limits;
use velesdb_memory::{ColumnFilter, ColumnOp, FusionOptions, Link, Metadata};

use crate::dto::{ColumnFilterJs, FusionOptionsJs, LinkJs};
use crate::error::invalid_input;

/// Format a `u64` id as a decimal string (JS `number` loses precision >2^53).
pub fn id_to_string(id: u64) -> String {
    id.to_string()
}

/// Parse a decimal-string id back to `u64`. Never panics; rejects floats/garbage.
pub fn parse_id(s: &str) -> napi::Result<u64> {
    s.parse::<u64>()
        .map_err(|_| invalid_input(format!("invalid id '{s}' (expected a decimal u64 string)")))
}

/// JS object → engine [`Metadata`]. `null`/absent → `None`; a non-object is an
/// error (callers must pass a plain object for metadata and filters).
pub fn to_metadata(value: Option<Value>) -> napi::Result<Option<Metadata>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err(invalid_input("metadata/filter must be an object")),
    }
}

/// JS `[{target, relation}]` → engine `Vec<Link>`, parsing each id.
pub fn to_links(links: Option<Vec<LinkJs>>) -> napi::Result<Vec<Link>> {
    links
        .unwrap_or_default()
        .into_iter()
        .map(|l| {
            Ok(Link {
                target: parse_id(&l.target)?,
                relation: l.relation,
            })
        })
        .collect()
}

/// Parse the lowercase operator token (mirrors `ColumnOp`'s serde rename).
fn parse_op(op: &str) -> napi::Result<ColumnOp> {
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

/// JS `[{field, op, value}]` → engine `Vec<ColumnFilter>`.
pub fn to_filters(filters: Vec<ColumnFilterJs>) -> napi::Result<Vec<ColumnFilter>> {
    filters
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

/// JS `{hops?, graphBoost?, pool?}` → engine [`FusionOptions`]. An omitted
/// object, or an omitted field within it, falls back to
/// [`FusionOptions::default`]'s proven value. `hops` and `pool` are each
/// capped at their shared `DoS` limit ([`limits::MAX_WHY_HOPS`],
/// [`limits::MAX_RECALL_LIMIT`]) — `pool` feeds the same oversampled vector
/// search `k`/`hops` do, so an uncapped caller-supplied value is exactly as
/// much of an unbounded-scan risk as an uncapped `k` or `hops` would be.
pub fn to_fusion_options(opts: Option<FusionOptionsJs>) -> FusionOptions {
    let defaults = FusionOptions::default();
    let Some(opts) = opts else {
        return defaults;
    };
    FusionOptions {
        hops: limits::clamp_hops(opts.hops.map_or(defaults.hops, |h| h as usize)),
        graph_boost: opts.graph_boost.unwrap_or(defaults.graph_boost),
        pool: opts
            .pool
            .map(|p| limits::clamp_recall_limit(p as usize))
            .or(defaults.pool),
    }
}
