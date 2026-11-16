//! ORDER BY execution for the WASM VelesQL executor (S4-13, #8).
//!
//! Supports ordering on any payload column (not just `id` / `score`),
//! multi-key sort, ASC/DESC per key, core's value order
//! (`velesql::compare_json_values`: nulls last in ASC, first in DESC), and arithmetic
//! expressions over the per-row score + payload.
//!
//! Sorting operates on core [`SearchResult`] values so the same rows can be
//! projected through velesdb-core's projection engine afterwards (#3b). A
//! score variable (`score`, `vector_score`, ...) and an arithmetic key resolve
//! through core's own [`velesdb_core::velesql::ScoreContext`] — the same
//! resolution order (LET, search score, component scores, `id`, payload) as
//! core's ORDER BY, so no name reads differently in the browser.
//!
//! `similarity(field, $v)` scores each row as core's ordering does: `vector`
//! the row's own vector, another field the payload's named vector
//! ([`velesdb_core::velesql::payload_vector`]), a row without one taking the
//! worst key so it sorts last under `DESC`. It needs the rows' collection
//! ([`OrderVectors`]); where the rows are no collection's points (groups) it
//! is refused, as is an aggregate ORDER BY outside an aggregation.

use std::cmp::Ordering;

use velesdb_core::point::SearchResult;
use velesdb_core::DistanceMetric;

use crate::store_state::StoreState;
use crate::velesql_value::Params;
use velesdb_core::velesql::{
    evaluate_arithmetic, is_builtin_score_variable, ArithmeticExpr, OrderByExpr, ScoreContext,
    SelectOrderBy, SelectStatement,
};

/// Sorts a result set in place according to the SELECT's ORDER BY clause.
///
/// Returns `Err` for ORDER BY shapes WASM cannot evaluate (named-vector
/// `similarity(field, $v)` or aggregate expressions outside aggregation), so
/// the executor fails loud instead of returning scan order (#8a).
/// `higher_is_better` is the collection metric's direction: `similarity()`
/// sorts most similar first under `DESC` whatever the metric, as in core
/// (`SelectOrderBy::directed`).
/// What `ORDER BY similarity(field, $v)` scores rows against: the query's
/// parameters and the collection the rows are points of.
pub(crate) struct OrderVectors<'a> {
    pub(crate) params: &'a Params,
    pub(crate) store: &'a StoreState,
}

/// What an ORDER BY reads besides the rows: the metric's direction, and the
/// rows' collection where they are its points (`None`: groups).
pub(crate) struct OrderContext<'a> {
    pub(crate) higher_is_better: bool,
    pub(crate) vectors: Option<OrderVectors<'a>>,
}

impl OrderContext<'_> {
    /// An ordering over rows that are no collection's points.
    pub(crate) const fn without_vectors(higher_is_better: bool) -> Self {
        Self {
            higher_is_better,
            vectors: None,
        }
    }
}

pub(crate) fn sort_rows(
    stmt: &SelectStatement,
    rows: &mut [SearchResult],
    ctx: &OrderContext<'_>,
) -> Result<(), String> {
    let Some(specs) = stmt.order_by.as_ref() else {
        return Ok(());
    };
    if specs.is_empty() {
        return Ok(());
    }
    validate_order_by(specs, ctx.vectors.is_some())?;
    let keys = similarity_keys(specs, rows, ctx)?;
    if keys.iter().all(Option::is_none) {
        rows.sort_by(|a, b| compare_rows(a, b, specs, ctx.higher_is_better, &[], 0, 0));
        return Ok(());
    }
    // Decorate-sort-undecorate: the precomputed keys follow their row.
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&i, &j| {
        compare_rows(&rows[i], &rows[j], specs, ctx.higher_is_better, &keys, i, j)
    });
    let sorted: Vec<SearchResult> = order.iter().map(|&i| rows[i].clone()).collect();
    rows.clone_from_slice(&sorted);
    Ok(())
}

/// The `similarity(field, $v)` key of every row, per ORDER BY item (`None`
/// for the other items).
fn similarity_keys(
    specs: &[SelectOrderBy],
    rows: &[SearchResult],
    ctx: &OrderContext<'_>,
) -> Result<Vec<Option<Vec<f32>>>, String> {
    specs
        .iter()
        .map(|spec| match (&spec.expr, &ctx.vectors) {
            (OrderByExpr::Similarity(sim), Some(vectors)) => {
                let query = crate::velesql_value::resolve_vector(&sim.vector, vectors.params)?;
                let worst = if ctx.higher_is_better {
                    f32::NEG_INFINITY
                } else {
                    f32::INFINITY
                };
                let own = OwnVectors::new(vectors.store);
                Ok(Some(
                    rows.iter()
                        .map(|row| {
                            field_similarity(row, &sim.field, &query, &own, vectors.store.metric)
                                .unwrap_or(worst)
                        })
                        .collect(),
                ))
            }
            _ => Ok(None),
        })
        .collect()
}

/// Row ids to their row in a store, for reading a row's own vector.
struct OwnVectors<'a> {
    store: &'a StoreState,
    rows: std::collections::HashMap<u64, usize>,
}

impl<'a> OwnVectors<'a> {
    fn new(store: &'a StoreState) -> Self {
        let rows = store
            .ids
            .iter()
            .enumerate()
            .map(|(row, &id)| (id, row))
            .collect();
        Self { store, rows }
    }

    fn get(&self, id: u64) -> Option<&'a [f32]> {
        let row = *self.rows.get(&id)?;
        let dim = self.store.dimension;
        (dim > 0).then(|| &self.store.data[row * dim..(row + 1) * dim])
    }
}

/// A row's similarity to `query` by the vector `field` names: its own for
/// `vector`, else the payload's named vector; `None` without one of the
/// query's dimension.
fn field_similarity(
    row: &SearchResult,
    field: &str,
    query: &[f32],
    own: &OwnVectors<'_>,
    metric: DistanceMetric,
) -> Option<f32> {
    let named;
    let vector: &[f32] = if field == velesdb_core::velesql::PRIMARY_VECTOR_FIELD {
        own.get(row.point.id)?
    } else {
        named =
            velesdb_core::velesql::payload_vector(row.point.payload.as_ref()?, field).ok()??;
        &named
    };
    (vector.len() == query.len() && !vector.is_empty()).then(|| metric.calculate(vector, query))
}

/// Rejects ORDER BY expressions WASM cannot evaluate before sorting begins.
fn validate_order_by(specs: &[SelectOrderBy], has_vectors: bool) -> Result<(), String> {
    for spec in specs {
        match &spec.expr {
            OrderByExpr::Similarity(_) if !has_vectors => {
                return Err(
                    "ORDER BY similarity(field, $vec) needs the rows of a collection: it is not \
                     supported over grouped rows. Use ORDER BY similarity() (the search score) \
                     or a column."
                        .to_string(),
                );
            }
            OrderByExpr::Similarity(_) => {}
            OrderByExpr::Aggregate(_) => {
                return Err(
                    "ORDER BY aggregate is only valid in an aggregation (GROUP BY) query"
                        .to_string(),
                );
            }
            OrderByExpr::Field(_) | OrderByExpr::SimilarityBare | OrderByExpr::Arithmetic(_) => {}
            // `OrderByExpr` is non_exhaustive: any future variant is unproven
            // on the WASM surface, so reject rather than silently no-op.
            _ => return Err("ORDER BY expression is not supported in WASM".to_string()),
        }
    }
    Ok(())
}

/// Strict total order used by `sort_by` over a multi-key ORDER BY. `keys`
/// holds the precomputed `similarity(field, $v)` keys, indexed by row
/// (`ia`, `ib` for `a`, `b`).
fn compare_rows(
    a: &SearchResult,
    b: &SearchResult,
    specs: &[SelectOrderBy],
    higher_is_better: bool,
    keys: &[Option<Vec<f32>>],
    ia: usize,
    ib: usize,
) -> Ordering {
    for (idx, spec) in specs.iter().enumerate() {
        let ascending = match keys.get(idx).and_then(Option::as_ref) {
            Some(scores) => scores[ia].total_cmp(&scores[ib]),
            None => compare_with_spec(a, b, spec),
        };
        let ord = spec.directed(ascending, higher_is_better);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    // Deterministic tie-break by ascending id, matching core's ORDER BY.
    a.point.id.cmp(&b.point.id)
}

/// Compares two rows on one ORDER BY item, ascending.
fn compare_with_spec(a: &SearchResult, b: &SearchResult, spec: &SelectOrderBy) -> Ordering {
    match &spec.expr {
        OrderByExpr::Field(name) => compare_field(a, b, name),
        OrderByExpr::SimilarityBare => compare_scores(a, b),
        OrderByExpr::Arithmetic(expr) => {
            let va = eval_arithmetic(expr, a);
            let vb = eval_arithmetic(expr, b);
            compare_f32(va, vb)
        }
        // Similarity / Aggregate were rejected by `validate_order_by`; treat
        // as equal defensively (the non_exhaustive enum may add variants).
        _ => Ordering::Equal,
    }
}

/// Compares a pair of rows by the given column: `id` is the point id, a
/// score variable resolves as core resolves it, any other name is a payload
/// field.
fn compare_field(a: &SearchResult, b: &SearchResult, name: &str) -> Ordering {
    if name == "id" {
        return a.point.id.cmp(&b.point.id);
    }
    if is_builtin_score_variable(name) {
        return compare_f32(
            score_context(a).resolve_variable(name),
            score_context(b).resolve_variable(name),
        );
    }
    let va = extract_payload_field(a, name);
    let vb = extract_payload_field(b, name);
    velesdb_core::velesql::compare_json_values(va.as_ref(), vb.as_ref())
}

/// Compares two rows by search score (NaN-safe, NaN sorts last in ASC).
fn compare_scores(a: &SearchResult, b: &SearchResult) -> Ordering {
    compare_f32(a.score, b.score)
}

/// NaN-safe f32 comparison (NaN sorts last in ASC).
fn compare_f32(a: f32, b: f32) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// Pulls a column from the payload: the key itself first (a group row stores
/// `first_meta.title` flat), then the dot-nested path — core's `row_field`.
fn extract_payload_field(result: &SearchResult, column: &str) -> Option<serde_json::Value> {
    result
        .point
        .payload
        .as_ref()
        .and_then(|p| velesdb_core::velesql::row_field(p, column).cloned())
}

/// Evaluates an arithmetic ORDER BY expression for a single row through
/// core's evaluator.
fn eval_arithmetic(expr: &ArithmeticExpr, result: &SearchResult) -> f32 {
    evaluate_arithmetic(expr, &score_context(result))
}

/// What a row's score variables read: its search score, its component
/// breakdown (a NEAR row carries `vector_score`, as in core) and its id.
fn score_context(result: &SearchResult) -> ScoreContext<'_> {
    ScoreContext::with_let_bindings(
        result.score,
        result.point.payload.as_ref(),
        result.component_scores.as_deref(),
        None,
    )
    .for_point(result.point.id)
}

#[cfg(test)]
#[path = "velesql_orderby_tests.rs"]
mod tests;
