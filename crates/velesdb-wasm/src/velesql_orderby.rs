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

use crate::exec_error::ExecError;
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
/// parameters, the metric, and where a row's own vector is read.
pub(crate) struct OrderVectors<'a> {
    pub(crate) params: &'a Params,
    pub(crate) metric: DistanceMetric,
    /// The collection the rows are points of, its vector read by row id; or
    /// `None` when each row carries its own (`point.vector`), as a compound's
    /// rows from several collections do (ids are collection-local).
    pub(crate) store: Option<&'a StoreState>,
}

impl<'a> OrderVectors<'a> {
    /// The rows of one collection, `store`.
    pub(crate) fn of(params: &'a Params, store: &'a StoreState) -> Self {
        Self {
            params,
            metric: store.metric,
            store: Some(store),
        }
    }
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
) -> Result<(), ExecError> {
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
    write_back_similarity(specs, &keys, &order, rows);
    Ok(())
}

/// Writes the first similarity ORDER BY item's key back as each row's score,
/// as core's ORDER BY does (`write_back_similarity_scores`). `similarity()`
/// reads the score itself, so only a recomputed `similarity(field, $v)` key
/// changes it; a row with no component breakdown is then tagged with it as
/// its `vector_score`, and a row it could not score (a null key) keeps its
/// score, untagged. `order[pos]` is the pre-sort index of the row now at
/// `pos`, the index `keys` is laid out by.
fn write_back_similarity(
    specs: &[SelectOrderBy],
    keys: &[SimilarityKeys],
    order: &[usize],
    rows: &mut [SearchResult],
) {
    let first = specs.iter().position(|spec| {
        matches!(
            spec.expr,
            OrderByExpr::Similarity(_) | OrderByExpr::SimilarityBare
        )
    });
    let Some(scores) = first.and_then(|idx| keys.get(idx)).and_then(Option::as_ref) else {
        return;
    };
    for (row, &orig) in rows.iter_mut().zip(order) {
        let Some(score) = scores[orig].filter(|score| score.is_finite()) else {
            continue;
        };
        row.score = score;
        if row.component_scores.is_none() {
            row.component_scores = Some(std::iter::once(("vector_score", score)).collect());
        }
    }
}

/// One ORDER BY item's `similarity(field, $v)` key of every row, `None` for
/// a row it cannot score (a null key); `None` for the other items.
type SimilarityKeys = Option<Vec<Option<f32>>>;

/// The `similarity(field, $v)` key of every row, per ORDER BY item.
fn similarity_keys(
    specs: &[SelectOrderBy],
    rows: &[SearchResult],
    ctx: &OrderContext<'_>,
) -> Result<Vec<SimilarityKeys>, ExecError> {
    specs
        .iter()
        .map(|spec| match (&spec.expr, &ctx.vectors) {
            (OrderByExpr::Similarity(sim), Some(vectors)) => {
                let query = velesdb_core::velesql::bind_vector(&sim.vector, vectors.params)?;
                let own = OwnVectors::new(vectors.store);
                Ok(Some(
                    rows.iter()
                        .map(|row| field_similarity(row, &sim.field, &query, &own, vectors.metric))
                        .collect(),
                ))
            }
            _ => Ok(None),
        })
        .collect()
}

/// Where a row's own vector is read: its collection's store by id, or the
/// row itself when the rows carry their vectors.
struct OwnVectors<'a> {
    store: Option<(&'a StoreState, std::collections::HashMap<u64, usize>)>,
}

impl<'a> OwnVectors<'a> {
    fn new(store: Option<&'a StoreState>) -> Self {
        let store = store.map(|store| {
            let rows = store
                .ids
                .iter()
                .enumerate()
                .map(|(row, &id)| (id, row))
                .collect();
            (store, rows)
        });
        Self { store }
    }

    fn get<'r>(&self, row: &'r SearchResult) -> Option<&'r [f32]>
    where
        'a: 'r,
    {
        let Some((store, rows)) = &self.store else {
            return (!row.point.vector.is_empty()).then_some(row.point.vector.as_slice());
        };
        let at = *rows.get(&row.point.id)?;
        let dim = store.dimension;
        (dim > 0).then(|| &store.data[at * dim..(at + 1) * dim])
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
        own.get(row)?
    } else {
        named =
            velesdb_core::velesql::payload_vector(row.point.payload.as_ref()?, field).ok()??;
        &named
    };
    metric.checked_calculate(vector, query)
}

/// Rejects ORDER BY expressions WASM cannot evaluate before sorting begins.
fn validate_order_by(specs: &[SelectOrderBy], has_vectors: bool) -> Result<(), ExecError> {
    for spec in specs {
        match &spec.expr {
            OrderByExpr::Similarity(_) if !has_vectors => {
                return Err(ExecError::refused(
                    "ORDER BY similarity(field, $vec) needs the rows of a collection: it is not \
                     supported over grouped rows. Use ORDER BY similarity() (the search score) \
                     or a column.",
                ));
            }
            OrderByExpr::Similarity(_) => {}
            // Core sorts such a key as a tie; WASM refuses it rather than
            // silently ignore it. No core rule to raise.
            OrderByExpr::Aggregate(_) => {
                return Err(ExecError::refused(
                    "ORDER BY aggregate is only valid in an aggregation (GROUP BY) query",
                ));
            }
            OrderByExpr::Field(_) | OrderByExpr::SimilarityBare | OrderByExpr::Arithmetic(_) => {}
            // `OrderByExpr` is non_exhaustive: any future variant is unproven
            // on the WASM surface, so reject rather than silently no-op.
            _ => {
                return Err(ExecError::refused(
                    "ORDER BY expression is not supported in WASM",
                ))
            }
        }
    }
    Ok(())
}

/// Strict total order used by `sort_by` over a multi-key ORDER BY. `keys`
/// holds the precomputed `similarity(field, $v)` keys, indexed by row
/// (`ia`, `ib` for `a`, `b`), a null key ordered as core orders it
/// (`SelectOrderBy::directed_score`).
fn compare_rows(
    a: &SearchResult,
    b: &SearchResult,
    specs: &[SelectOrderBy],
    higher_is_better: bool,
    keys: &[SimilarityKeys],
    ia: usize,
    ib: usize,
) -> Ordering {
    for (idx, spec) in specs.iter().enumerate() {
        let ord = match keys.get(idx).and_then(Option::as_ref) {
            Some(scores) => spec.directed_score(scores[ia], scores[ib], higher_is_better),
            None => spec.directed(compare_with_spec(a, b, spec), higher_is_better),
        };
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
