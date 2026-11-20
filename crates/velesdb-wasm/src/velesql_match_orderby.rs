//! MATCH `RETURN ... ORDER BY` sorting for the WASM executor.
//!
//! Mirrors core's `order_match_results` semantics
//! (`match_exec/order_by.rs` + `similarity.rs`): a deterministic baseline keyed
//! by the row's bound node ids, then each ORDER BY key applied as a STABLE sort
//! least-significant-first (`.rev()`), so multi-key ordering is correct and ties
//! resolve to a total, deterministic order. Sorting happens BEFORE the LIMIT
//! truncation (the caller collects the full candidate set first).
//!
//! Supported keys, as in core: `depth` (constant within a single pattern, so
//! a defined no-op), an `alias.property` path, bare `similarity()` (the row's
//! score, [`crate::velesql_match_score`]), `similarity(field, $v)` (the vector
//! of the node `alias.` names, else the row's target), and arithmetic over
//! the target (`year - 2000`, `id`, through core's `ScoreContext`). The
//! similarity keys are most-similar-first under every metric, a row they
//! cannot score a null key (core's `velesql::compare_scores`). Aggregates are
//! refused, as core refuses them.

use std::cmp::Ordering;

use velesdb_core::velesql::json_path::payload_path;
use velesdb_core::velesql::{
    compare_scores, evaluate_arithmetic, ArithmeticExpr, OrderByExpr, OrderByItem, ScoreContext,
    SimilarityOrderBy,
};

use crate::exec_error::ExecError;
use crate::velesql_match_nodes::MatchNodes;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// What a MATCH ORDER BY reads besides the rows: the metric's direction,
/// the walk's nodes (their vectors and own payloads) and the parameters.
pub(crate) struct MatchOrder<'a> {
    pub(crate) higher_is_better: bool,
    pub(crate) nodes: Option<&'a MatchNodes<'a>>,
    pub(crate) params: &'a Params,
}

/// A MATCH result row paired with the data needed to order it.
pub(crate) struct MatchCandidate {
    /// The row's bound node ids in pattern order (`[a]`, `[a, b]`, `[a, b, c]`).
    /// Each matched row has a distinct tuple, so this is a per-row-unique
    /// deterministic tie-break baseline (a TOTAL order) — mirroring core keying
    /// its baseline on the full match identity rather than the anchor alone
    /// (anchor `a` repeats across the many `b`/`c` of a star pattern).
    pub baseline: Vec<u64>,
    /// Alias-keyed row JSON (e.g. `{"a": {...}, "b": {...}}`), used to resolve
    /// `alias.property` ORDER BY keys via core's `payload_path`.
    pub value: serde_json::Value,
    /// The serialized row returned to the caller after sorting.
    pub row: QueryResultRow,
    /// What the row binds, for core-shaped results
    /// ([`crate::velesql_match_core`]).
    pub binding: MatchBinding,
    /// The row's `similarity()` score, when the MATCH computes one
    /// ([`crate::velesql_match_score`]): the metric's raw value.
    pub score: Option<f32>,
}

/// The elements one MATCH row binds, in pattern order.
#[derive(Debug, Default)]
pub(crate) struct MatchBinding {
    /// Bound nodes: explicit alias (`None` for an unnamed node), id, payload.
    pub nodes: Vec<BoundElement>,
    /// Traversed relationships: explicit alias, edge id, edge properties.
    pub edges: Vec<BoundElement>,
}

/// One bound node or relationship.
#[derive(Debug)]
pub(crate) struct BoundElement {
    pub alias: Option<String>,
    pub id: u64,
    pub payload: Option<serde_json::Value>,
}

/// Sorts MATCH candidates in place per the RETURN `ORDER BY`, then truncates to
/// `limit`. With no ORDER BY, only the limit is applied (traversal order
/// preserved) — matching the prior behavior for that case.
///
/// # Errors
///
/// Returns an error for ORDER BY forms the WASM MATCH path cannot evaluate
/// (`similarity()`, `similarity(field, $v)`, arithmetic, aggregate).
pub(crate) fn order_and_limit(
    candidates: &mut Vec<MatchCandidate>,
    order_by: Option<&[OrderByItem]>,
    limit: Option<u64>,
    order: &MatchOrder<'_>,
) -> Result<(), ExecError> {
    if let Some(items) = order_by {
        // Deterministic baseline: the bound-node-id tuple is unique per row, so
        // it gives the stable per-key sorts below a total tie-break order.
        candidates.sort_by(|a, b| a.baseline.cmp(&b.baseline));
        // Apply least-significant key first so the most-significant wins.
        for item in items.iter().rev() {
            sort_by_key(candidates, &item.expr, item.descending, order)?;
        }
    }
    if let Some(limit) = limit {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        candidates.truncate(limit);
    }
    Ok(())
}

/// Applies one STABLE ORDER BY key.
fn sort_by_key(
    candidates: &mut [MatchCandidate],
    expr: &OrderByExpr,
    descending: bool,
    order: &MatchOrder<'_>,
) -> Result<(), ExecError> {
    let higher_is_better = order.higher_is_better;
    match expr {
        OrderByExpr::Field(f) if f == "depth" => {
            // WASM patterns have a fixed depth per query, so this is a defined
            // no-op within a single pattern — kept for parity, not silently
            // dropped.
            Ok(())
        }
        OrderByExpr::Field(f) => sort_by_property(candidates, f, descending),
        OrderByExpr::SimilarityBare => {
            // Core's `sort_match_by_score`: a distance is negated so `DESC`
            // is most-similar-first under every metric; an unscored row is a
            // null key, last under `ASC` and first under `DESC`.
            sort_stable(candidates, descending, |a, b| {
                let key = |c: &MatchCandidate| {
                    c.score
                        .map(|score| if higher_is_better { score } else { -score })
                };
                compare_scores(key(a), key(b))
            });
            Ok(())
        }
        OrderByExpr::Similarity(sim) => {
            let keys = similarity_keys(candidates, sim, order)?;
            sort_by_keys(candidates, &keys, descending);
            Ok(())
        }
        OrderByExpr::Arithmetic(expr) => {
            let keys: Vec<Option<f32>> = candidates
                .iter()
                .map(|c| Some(arithmetic_key(c, expr, order.nodes)))
                .collect();
            sort_by_keys(candidates, &keys, descending);
            Ok(())
        }
        // Core's refusal of the same ORDER BY (`order_match_results`).
        _ => Err(velesdb_core::Error::GraphNotSupported(
            "MATCH ORDER BY aggregate expression is not supported (use similarity(), depth, \
             alias.property, or arithmetic over properties)"
                .to_string(),
        )
        .into()),
    }
}

/// The row's target: the last bound node (core's `MatchResult::node_id`).
fn target(candidate: &MatchCandidate) -> Option<u64> {
    candidate.binding.nodes.last().map(|node| node.id)
}

/// Core's `sort_match_by_similarity` key per row: the vector of the node
/// `alias.` names (the target for a bare field) against the query vector, a
/// distance negated so a larger key is more similar; an unscorable row a
/// null key (`None`).
fn similarity_keys(
    candidates: &[MatchCandidate],
    sim: &SimilarityOrderBy,
    order: &MatchOrder<'_>,
) -> Result<Vec<Option<f32>>, ExecError> {
    let query = velesdb_core::velesql::bind_vector(&sim.vector, order.params)?;
    let alias = sim.field.split_once('.').map(|(alias, _)| alias);
    let space = order.nodes.and_then(MatchNodes::vector_space);
    Ok(candidates
        .iter()
        .map(|c| {
            let node = match alias {
                Some(alias) => c
                    .binding
                    .nodes
                    .iter()
                    .find(|n| n.alias.as_deref() == Some(alias))
                    .map(|n| n.id),
                None => target(c),
            };
            node.zip(order.nodes)
                .and_then(|(id, nodes)| nodes.vector(id))
                .zip(space)
                .and_then(|(vector, (_, metric))| {
                    let raw = metric.checked_calculate(vector, &query)?;
                    Some(if metric.higher_is_better() { raw } else { -raw })
                })
        })
        .collect())
}

/// Core's `sort_match_by_arithmetic` key: the expression over the target's
/// own payload and id, the row's score as `score`.
fn arithmetic_key(
    candidate: &MatchCandidate,
    expr: &ArithmeticExpr,
    nodes: Option<&MatchNodes<'_>>,
) -> f32 {
    let Some(id) = target(candidate) else {
        return 0.0;
    };
    let payload = nodes
        .and_then(|n| n.get(id))
        .and_then(|n| n.payload.as_ref());
    let ctx = ScoreContext::new(candidate.score.unwrap_or(0.0), payload).for_point(id);
    evaluate_arithmetic(expr, &ctx)
}

/// Stable sort of the candidates by one precomputed key each.
fn sort_by_keys(candidates: &mut [MatchCandidate], keys: &[Option<f32>], descending: bool) {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&i, &j| {
        let ascending = compare_scores(keys[i], keys[j]);
        if descending {
            ascending.reverse()
        } else {
            ascending
        }
    });
    apply_order(candidates, &order);
}

/// Rearranges `items` so position `k` holds the item `order[k]` named.
fn apply_order<T>(items: &mut [T], order: &[usize]) {
    let mut position: Vec<usize> = vec![0; order.len()];
    for (k, &from) in order.iter().enumerate() {
        position[from] = k;
    }
    for start in 0..items.len() {
        while position[start] != start {
            let target = position[start];
            items.swap(start, target);
            position.swap(start, target);
        }
    }
}

/// Sorts by an `alias.property` JSON path (dot-nested), nulls last in ASC.
fn sort_by_property(
    candidates: &mut [MatchCandidate],
    path: &str,
    descending: bool,
) -> Result<(), ExecError> {
    if !path.contains('.') {
        // Core's refusal of the same ORDER BY (`order_match_results_by_property`).
        return Err(velesdb_core::Error::GraphNotSupported(format!(
            "MATCH ORDER BY expression '{path}' is not supported \
             (use similarity(), depth, or alias.property)"
        ))
        .into());
    }
    sort_stable(candidates, descending, |a, b| {
        let va = payload_path(&a.value, path);
        let vb = payload_path(&b.value, path);
        velesdb_core::velesql::compare_json_values(va, vb)
    });
    Ok(())
}

/// STABLE sort applying the comparison and the ASC/DESC direction.
fn sort_stable<F>(candidates: &mut [MatchCandidate], descending: bool, cmp: F)
where
    F: Fn(&MatchCandidate, &MatchCandidate) -> Ordering,
{
    candidates.sort_by(|a, b| {
        let ord = cmp(a, b);
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
#[path = "velesql_match_orderby_tests.rs"]
mod tests;
