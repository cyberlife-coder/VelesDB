//! WHERE clause evaluation over JSON payloads for the WASM VelesQL executor.
//!
//! A WHERE condition is bound and lowered by core, then matched by core's
//! payload filter, the very one the server runs: [`bind_condition`] binds its
//! `$name` operands, [`Filter::try_from_where`] lowers it, and
//! [`Filter::matches_point`] tests a point. A predicate therefore answers in
//! the browser what it answers on the server — in SQL's three-valued logic,
//! so `x != 'a'` excludes a payload without `x` as `x = 'a'` does, LIKE honours `\` escapes and its guardrails, numbers compare
//! by exact value. This module only decides the WASM scope: the conditions
//! the executor does not evaluate as a row filter are refused by name.
//!
//! Vector/graph/fusion conditions are NOT evaluated here — the executor
//! dispatches them before calling this matcher (see `velesql_select.rs`).

use velesdb_core::filter::Filter;
use velesdb_core::velesql::{bind_condition, Condition};

use crate::velesql_value::Params;

/// A WHERE condition bound to a query's parameters and lowered to core's
/// payload filter once, to test every point of a scan against.
pub(crate) struct WhereFilter(Filter);

impl WhereFilter {
    /// Binds `cond` to `params` and lowers it to core's payload filter.
    ///
    /// # Errors
    ///
    /// Names a condition outside the WASM row-filter scope, a parameter that
    /// is missing or bound to an array or object, or a construct core cannot
    /// lower to a row filter.
    pub(crate) fn compile(cond: &Condition, params: &Params) -> Result<Self, String> {
        reject_out_of_scope(cond)?;
        let bound = bind_condition(cond, params).map_err(|e| e.to_string())?;
        Filter::try_from_where(bound).map(Self)
    }

    /// Whether the point `id`, with `payload` if it has one, matches. The
    /// special `id` column reads `id`; any other column a payload field,
    /// dot-nested access included.
    pub(crate) fn matches(&self, id: u64, payload: Option<&serde_json::Value>) -> bool {
        self.0.matches_point(id, payload)
    }
}

/// Tests whether the given `id` and optional `payload` match the condition.
///
/// Compiles `cond` for this one point: a caller testing many points against
/// the same condition compiles a [`WhereFilter`] once instead.
///
/// # Errors
///
/// See [`WhereFilter::compile`].
pub(crate) fn matches(
    cond: &Condition,
    id: u64,
    payload: Option<&serde_json::Value>,
    params: &Params,
) -> Result<bool, String> {
    Ok(WhereFilter::compile(cond, params)?.matches(id, payload))
}

/// Refuses, depth-first, the first condition the WASM executor does not
/// evaluate as a row filter, by name.
fn reject_out_of_scope(cond: &Condition) -> Result<(), String> {
    match cond {
        Condition::Comparison(_)
        | Condition::In(_)
        | Condition::Between(_)
        | Condition::Like(_)
        | Condition::IsNull(_) => Ok(()),
        Condition::And(l, r) | Condition::Or(l, r) => {
            reject_out_of_scope(l)?;
            reject_out_of_scope(r)
        }
        Condition::Not(inner) | Condition::Group(inner) => reject_out_of_scope(inner),
        Condition::VectorSearch(_)
        | Condition::VectorFusedSearch(_)
        | Condition::SparseVectorSearch(_) => Err(
            "Vector NEAR clauses are handled by the SELECT dispatcher, not by the WHERE filter"
                .to_string(),
        ),
        Condition::Similarity(_) => {
            Err("similarity() threshold filters are not supported in WASM".to_string())
        }
        Condition::GraphMatch(_) => {
            Err("Graph MATCH predicates are not supported in WASM".to_string())
        }
        Condition::Match(_) => Err("MATCH (BM25) conditions are not supported in WASM".to_string()),
        Condition::Contains(_) | Condition::ContainsText(_) => {
            Err("CONTAINS / CONTAINS_TEXT conditions are not supported in WASM".to_string())
        }
        Condition::GeoDistance(_) | Condition::GeoBbox(_) => {
            Err("Geospatial conditions are not supported in WASM".to_string())
        }
        // Defensive catch-all: `Condition` is `#[non_exhaustive]`; any new
        // variant added upstream is rejected until explicitly mapped here.
        _ => Err(format!(
            "Unsupported condition variant in WASM WHERE clause: {cond:?}"
        )),
    }
}

#[cfg(test)]
#[path = "velesql_where_tests.rs"]
mod tests;
