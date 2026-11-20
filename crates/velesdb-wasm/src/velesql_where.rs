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

use crate::exec_error::ExecError;
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
    pub(crate) fn compile(cond: &Condition, params: &Params) -> Result<Self, ExecError> {
        reject_out_of_scope(cond)?;
        let bound = bind_condition(cond, params)?;
        // Core's `try_from_where` reports its refusal as a bare message (a
        // scope it cannot lower); it carries no core code to keep.
        Filter::try_from_where(bound)
            .map(Self)
            .map_err(ExecError::refused)
    }

    /// Whether the point `id`, with `payload` if it has one, matches. The
    /// special `id` column reads `id`; any other column a payload field,
    /// dot-nested access included.
    pub(crate) fn matches(&self, id: u64, payload: Option<&serde_json::Value>) -> bool {
        self.0.matches_point(id, payload)
    }

    /// The condition's three-valued truth for the point: `None` is UNKNOWN.
    /// For a caller combining it with leaves of its own (see [`truth`]).
    pub(crate) fn truth(&self, id: u64, payload: Option<&serde_json::Value>) -> Option<bool> {
        self.0.evaluate_point(id, payload)
    }
}

/// The three-valued truth of `cond` for one point: `None` is UNKNOWN.
///
/// For a WHERE that evaluates some leaves itself (`similarity()`) and hands
/// the others here: it must combine the answers with [`and`], [`or`] and
/// `Option::map(!)`, never with `bool` operators, or `NOT` over an UNKNOWN
/// leaf would admit the row.
///
/// # Errors
///
/// See [`WhereFilter::compile`].
pub(crate) fn truth(
    cond: &Condition,
    id: u64,
    payload: Option<&serde_json::Value>,
    params: &Params,
) -> Result<Option<bool>, ExecError> {
    Ok(WhereFilter::compile(cond, params)?.truth(id, payload))
}

/// Kleene AND: FALSE wins, then UNKNOWN.
pub(crate) fn and(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

/// Kleene OR: TRUE wins, then UNKNOWN.
pub(crate) fn or(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// The three-valued truth of `cond`, each leaf answered by `leaf`: `AND`,
/// `OR`, `NOT` and grouping folded with [`and`], [`or`] and `NOT` keeping
/// UNKNOWN — the one fold of every WHERE that evaluates some leaves itself
/// (a `similarity()` leaf) and hands the others to [`truth`].
///
/// # Errors
///
/// The first error `leaf` returns.
pub(crate) fn fold_truth(
    cond: &Condition,
    leaf: &dyn Fn(&Condition) -> Result<Option<bool>, ExecError>,
) -> Result<Option<bool>, ExecError> {
    match cond {
        Condition::And(l, r) => fold_both(l, r, leaf, and),
        Condition::Or(l, r) => fold_both(l, r, leaf, or),
        Condition::Not(inner) => fold_truth(inner, leaf).map(|truth| truth.map(|known| !known)),
        Condition::Group(inner) => fold_truth(inner, leaf),
        other => leaf(other),
    }
}

/// [`fold_truth`] of both sides of a binary node, combined by `combine`.
fn fold_both(
    left: &Condition,
    right: &Condition,
    leaf: &dyn Fn(&Condition) -> Result<Option<bool>, ExecError>,
    combine: fn(Option<bool>, Option<bool>) -> Option<bool>,
) -> Result<Option<bool>, ExecError> {
    Ok(combine(fold_truth(left, leaf)?, fold_truth(right, leaf)?))
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
) -> Result<bool, ExecError> {
    Ok(WhereFilter::compile(cond, params)?.matches(id, payload))
}

/// Refuses, depth-first, the first condition the WASM executor does not
/// evaluate as a row filter, by name.
fn reject_out_of_scope(cond: &Condition) -> Result<(), ExecError> {
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
        other => Err(out_of_scope(other)),
    }
}

/// The refusal of a leaf the WASM executor does not evaluate as a row
/// filter, naming what it is.
fn out_of_scope(cond: &Condition) -> ExecError {
    let what = match cond {
        Condition::VectorSearch(_)
        | Condition::VectorFusedSearch(_)
        | Condition::SparseVectorSearch(_) => {
            "Vector NEAR clauses are handled by the SELECT dispatcher, not by the WHERE filter"
        }
        Condition::Similarity(_) => "similarity() threshold filters are not supported in WASM",
        Condition::GraphMatch(_) => "Graph MATCH predicates are not supported in WASM",
        Condition::Match(_) => "MATCH (BM25) conditions are not supported in WASM",
        Condition::Contains(_) | Condition::ContainsText(_) => {
            "CONTAINS / CONTAINS_TEXT conditions are not supported in WASM"
        }
        Condition::GeoDistance(_) | Condition::GeoBbox(_) => {
            "Geospatial conditions are not supported in WASM"
        }
        // Defensive catch-all: `Condition` is `#[non_exhaustive]`; any new
        // variant added upstream is rejected until explicitly mapped here.
        _ => {
            return ExecError::refused(format!(
                "Unsupported condition variant in WASM WHERE clause: {cond:?}"
            ))
        }
    };
    ExecError::refused(what)
}

#[cfg(test)]
#[path = "velesql_where_tests.rs"]
mod tests;
