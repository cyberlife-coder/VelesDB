//! `similarity()` threshold evaluation for WASM WHERE clauses (S4-13).
//!
//! Supports `WHERE similarity(field, $q) <op> <threshold>` inside a SELECT,
//! read as core reads it ([`velesdb_core::velesql::similarity_holds`]).
//!
//! This module only decides `is this row's similarity <op> threshold?` —
//! the actual vector NEAR ranking stays in `velesql_select::vector_path`.
//!
//! Every leaf is scored on its own ([`SimilarityEvaluator`]): several
//! `similarity()` under `AND` each filter, a named vector
//! `similarity(embedding, $v)` reads the payload's `embedding` array, and a
//! row with no vector a leaf can score makes that leaf UNKNOWN. The shapes no
//! engine runs (several `similarity()` under one `OR`) are refused upstream by
//! `velesdb_core::velesql::validate_vector_predicates`, as in core.

use velesdb_core::velesql::{
    payload_vector, CompareOp, Condition, SimilarityCondition, VectorExpr, PRIMARY_VECTOR_FIELD,
};

use crate::database::DatabaseInner;
use crate::store_state::StoreState;
use crate::velesql_value::{resolve_vector, Params};

/// The condition with every `similarity()` leaf removed.
///
/// `cond` must already be De-Morgan-normalized ([`push_not_inward`](crate::velesql_logic::push_not_inward)), so the
/// strip pass only sees bare `Similarity(_)` leaves — a `NOT` wrapping one
/// has been pushed into its flipped operator. Without that, a compound like
/// `NOT (sim > 0.5 AND x = 1)` would leave `NOT(x = 1)` behind, i.e.
/// `sim > 0.5 AND NOT(x = 1)` instead of `sim <= 0.5 OR x != 1`.
pub(crate) fn strip_similarity(cond: Option<&Condition>) -> Option<Condition> {
    strip_condition_if(cond, &|c| matches!(c, Condition::Similarity(_)))
}

/// Recursively strips any sub-condition that satisfies `should_remove`.
///
/// Shared by both `strip_similarity` and `velesql_select::strip_vector_search`
/// so the And/Or/Not/Group fan-out logic lives in a single place. Returns
/// `None` when the whole tree would collapse to nothing after removals.
pub(crate) fn strip_condition_if<F>(
    cond: Option<&Condition>,
    should_remove: &F,
) -> Option<Condition>
where
    F: Fn(&Condition) -> bool,
{
    let cond = cond?;
    if should_remove(cond) {
        return None;
    }
    match cond {
        Condition::And(l, r) => combine_after_strip(
            strip_condition_if(Some(l), should_remove),
            strip_condition_if(Some(r), should_remove),
            LogicalOp::And,
        ),
        Condition::Or(l, r) => combine_after_strip(
            strip_condition_if(Some(l), should_remove),
            strip_condition_if(Some(r), should_remove),
            LogicalOp::Or,
        ),
        Condition::Not(inner) => {
            strip_condition_if(Some(inner), should_remove).map(|c| Condition::Not(Box::new(c)))
        }
        Condition::Group(inner) => {
            strip_condition_if(Some(inner), should_remove).map(|c| Condition::Group(Box::new(c)))
        }
        other => Some(other.clone()),
    }
}

/// Flips a comparison operator to its logical complement.
///
/// Used by [`push_not_inward`](crate::velesql_logic::push_not_inward)
/// and the similarity-specific rewrites to negate a `similarity()`
/// predicate without rebuilding its surrounding AST. `CompareOp` is
/// `#[non_exhaustive]`; unknown variants fall back to the input
/// (identity) rather than panicking — deterministic and auditable via
/// the existing similarity tests.
pub(crate) fn flip_similarity_op(op: CompareOp) -> CompareOp {
    match op {
        CompareOp::Gt => CompareOp::Lte,
        CompareOp::Gte => CompareOp::Lt,
        CompareOp::Lt => CompareOp::Gte,
        CompareOp::Lte => CompareOp::Gt,
        CompareOp::Eq => CompareOp::NotEq,
        CompareOp::NotEq => CompareOp::Eq,
        // Reason: `CompareOp` is `#[non_exhaustive]`; new variants keep
        // their original operator (identity) until explicitly mapped.
        _ => op,
    }
}

/// Which boolean operator was at this node before a branch was stripped.
///
/// Needed by [`combine_after_strip`] to apply the correct identity: a
/// stripped-out branch is logically `true` (handled externally by the
/// scoring / NEAR / similarity path), so:
/// - `And`: `true AND x = x`  — drop only the stripped side
/// - `Or` : `true OR x  = true` — whole OR is trivially satisfied
#[derive(Copy, Clone)]
pub(crate) enum LogicalOp {
    /// Conjunction. A stripped branch is the identity for `And`.
    And,
    /// Disjunction. A stripped branch is the absorbing element for `Or`.
    Or,
}

/// Combines two already-stripped branches while preserving the semantics
/// of the original logical operator.
///
/// A `None` argument means that side was stripped, i.e. logically `true`
/// once the external path (vector NEAR, similarity scorer) handles it.
/// This is the key correctness fix for finding G: under `Or`, a stripped
/// branch absorbs the whole node (`true OR x = true`), so the residual
/// post-filter must be `None` — not the surviving side.
///
/// Truth table (`a` / `b` = stripped? ; result = residual post-filter):
///
/// | op  | a / b          | result      |
/// |-----|----------------|-------------|
/// | And | (None, None)   | None        |
/// | And | (Some(x), None)| Some(x)     |
/// | And | (None, Some(y))| Some(y)     |
/// | And | (Some(x), Some(y)) | Some(x AND y) |
/// | Or  | (None, None)   | None        |
/// | Or  | (Some(_), None)| **None**    |
/// | Or  | (None, Some(_))| **None**    |
/// | Or  | (Some(x), Some(y)) | Some(x OR y) |
fn combine_after_strip(
    l: Option<Condition>,
    r: Option<Condition>,
    op: LogicalOp,
) -> Option<Condition> {
    match (op, l, r) {
        // Both sides survived → rebuild the original node.
        (LogicalOp::And, Some(a), Some(b)) => Some(Condition::And(Box::new(a), Box::new(b))),
        (LogicalOp::Or, Some(a), Some(b)) => Some(Condition::Or(Box::new(a), Box::new(b))),
        // AND with a stripped side → the surviving side becomes the
        // residual (`true AND x = x`).
        (LogicalOp::And, Some(only), None) | (LogicalOp::And, None, Some(only)) => Some(only),
        // OR with ANY stripped side → whole OR is trivially satisfied by
        // the external path (`true OR x = true`). No residual post-filter.
        (LogicalOp::Or, _, None) | (LogicalOp::Or, None, _) => None,
        // Both sides stripped out → nothing left.
        (_, None, None) => None,
    }
}

/// The scores of one `similarity(field, vector)` leaf, by row: `None` where
/// the row has no vector under `field` the query vector can be scored
/// against (absent, not numeric, another dimension) — the leaf is then
/// UNKNOWN for that row, as in core.
#[derive(Debug)]
struct LeafScores {
    field: String,
    vector: VectorExpr,
    scores: Vec<Option<f32>>,
}

/// Every `similarity()` leaf of a WHERE clause, scored once over the whole
/// collection so a leaf is an O(1) lookup in the row scan.
///
/// Each distinct `(field, query vector)` is scored on its own:
/// `similarity(vector, $v)` against the row's own vector,
/// `similarity(embedding, $v)` against the numeric array its payload holds
/// under `embedding` ([`velesdb_core::velesql::payload_vector`]).
#[derive(Debug)]
pub(crate) struct SimilarityEvaluator {
    /// One entry per distinct leaf, in the order the clause names them.
    leaves: Vec<LeafScores>,
    /// Whether the collection's metric ranks higher scores as more similar
    /// (`false` for a distance): the direction `similarity() > t` reads in.
    higher_is_better: bool,
}

impl SimilarityEvaluator {
    /// Scores every `similarity()` leaf of `cond` over `collection`; `None`
    /// when `cond` has none.
    ///
    /// # Errors
    ///
    /// An unbound or malformed query vector, and a leaf on the row's own
    /// vector over a metadata-only collection or with another dimension.
    pub(crate) fn for_condition(
        db: &DatabaseInner,
        collection: &str,
        cond: Option<&Condition>,
        params: &Params,
    ) -> Result<Option<Self>, String> {
        let mut sims = Vec::new();
        if let Some(cond) = cond {
            collect_similarity_leaves(cond, &mut sims);
        }
        if sims.is_empty() {
            return Ok(None);
        }
        let store = db.get_shared_store(collection)?;
        let borrowed = store.borrow();
        let mut leaves: Vec<LeafScores> = Vec::new();
        for sim in sims {
            if leaves
                .iter()
                .any(|l| l.field == sim.field && l.vector == sim.vector)
            {
                continue;
            }
            let query = resolve_vector(&sim.vector, params)?;
            let scores = if sim.field == PRIMARY_VECTOR_FIELD {
                own_vector_scores(&borrowed, collection, &query)?
            } else {
                payload_vector_scores(&borrowed, &sim.field, &query)
            };
            leaves.push(LeafScores {
                field: sim.field.clone(),
                vector: sim.vector.clone(),
                scores,
            });
        }
        Ok(Some(Self {
            leaves,
            higher_is_better: borrowed.metric.higher_is_better(),
        }))
    }

    /// The score row `idx` carries: its similarity to the clause's first
    /// `similarity()` leaf, as core reports it; `None` when that leaf cannot
    /// score the row.
    pub(crate) fn score_at(&self, idx: usize) -> Option<f32> {
        self.leaves
            .first()
            .and_then(|leaf| leaf.scores.get(idx).copied().flatten())
    }

    /// The three-valued truth of the leaf `cond` at row `idx`: UNKNOWN when
    /// the row has no vector the leaf can score.
    ///
    /// The operator and threshold come from the AST node, so rewrites by
    /// [`push_not_inward`](crate::velesql_logic::push_not_inward) (`>` flipped to `<=` under a NOT) surface with the
    /// right polarity.
    pub(crate) fn truth(&self, cond: &SimilarityCondition, idx: usize) -> Option<bool> {
        let leaf = self
            .leaves
            .iter()
            .find(|l| l.field == cond.field && l.vector == cond.vector)?;
        let score = leaf.scores.get(idx).copied().flatten()?;
        Some(velesdb_core::velesql::similarity_holds(
            cond.operator,
            score,
            cond.threshold,
            self.higher_is_better,
        ))
    }
}

/// Every `similarity()` leaf of `cond`, left to right.
fn collect_similarity_leaves<'c>(cond: &'c Condition, out: &mut Vec<&'c SimilarityCondition>) {
    match cond {
        Condition::Similarity(sim) => out.push(sim),
        Condition::And(l, r) | Condition::Or(l, r) => {
            collect_similarity_leaves(l, out);
            collect_similarity_leaves(r, out);
        }
        Condition::Not(inner) | Condition::Group(inner) => collect_similarity_leaves(inner, out),
        _ => {}
    }
}

/// `similarity(vector, $v)`: every row's own vector against `query`.
fn own_vector_scores(
    store: &StoreState,
    collection: &str,
    query: &[f32],
) -> Result<Vec<Option<f32>>, String> {
    if store.dimension == 0 {
        return Err(format!(
            "Collection '{collection}' is metadata-only; similarity() requires a vector collection"
        ));
    }
    if query.len() != store.dimension {
        return Err(format!(
            "similarity() query dimension mismatch: expected {}, got {}",
            store.dimension,
            query.len()
        ));
    }
    Ok(crate::vector_ops::compute_scores(
        query,
        &store.ids,
        &store.data,
        store.dimension,
        store.metric,
    )
    .into_iter()
    .map(|(_, score)| Some(score))
    .collect())
}

/// `similarity(field, $v)`: the vector each row's payload holds under
/// `field` against `query`; `None` where it holds none of that dimension.
fn payload_vector_scores(store: &StoreState, field: &str, query: &[f32]) -> Vec<Option<f32>> {
    (0..store.ids.len())
        .map(|idx| {
            let payload = store.payloads.get(idx).and_then(Option::as_ref)?;
            let vector = payload_vector(payload, field).ok().flatten()?;
            (vector.len() == query.len() && !vector.is_empty())
                .then(|| store.metric.calculate(&vector, query))
        })
        .collect()
}

/// Evaluates a WHERE condition against a row, treating `Similarity`
/// leaves as an inline dispatch to `eval` (when provided).
///
/// This keeps the boolean composition (AND / OR / NOT) semantically
/// correct even when a similarity predicate sits inside an OR or deep
/// under a De-Morgan-rewritten NOT — which the previous "strip
/// similarity out + AND with evaluator" composition got wrong (it
/// always conjoined, so `OR sim` collapsed to `AND sim`).
///
/// Non-similarity leaves delegate to [`crate::velesql_where::truth`] for
/// identical semantics, and the tree folds in SQL's three-valued logic, as
/// core's WHERE does: a row is admitted only when the whole condition is
/// known TRUE, so `NOT (x LIKE 'a%')` excludes a row without `x`. The input `cond` is expected to already
/// be De-Morgan-normalized via [`push_not_inward`](crate::velesql_logic::push_not_inward).
pub(crate) fn evaluate_where_with_similarity(
    cond: &Condition,
    id: u64,
    payload: Option<&serde_json::Value>,
    idx: usize,
    eval: Option<&SimilarityEvaluator>,
    params: &Params,
) -> Result<bool, String> {
    Ok(where_truth(cond, id, payload, idx, eval, params)? == Some(true))
}

/// The three-valued truth behind [`evaluate_where_with_similarity`].
fn where_truth(
    cond: &Condition,
    id: u64,
    payload: Option<&serde_json::Value>,
    idx: usize,
    eval: Option<&SimilarityEvaluator>,
    params: &Params,
) -> Result<Option<bool>, String> {
    use crate::velesql_where::{and, or, truth};
    match cond {
        Condition::And(l, r) => Ok(and(
            where_truth(l, id, payload, idx, eval, params)?,
            where_truth(r, id, payload, idx, eval, params)?,
        )),
        Condition::Or(l, r) => Ok(or(
            where_truth(l, id, payload, idx, eval, params)?,
            where_truth(r, id, payload, idx, eval, params)?,
        )),
        Condition::Not(inner) => {
            Ok(where_truth(inner, id, payload, idx, eval, params)?.map(|known| !known))
        }
        Condition::Group(inner) => where_truth(inner, id, payload, idx, eval, params),
        Condition::Similarity(sim) => {
            // No evaluator => no vector collection context; this should
            // never happen when the query was validated upstream, but we
            // surface a clear error rather than silently passing or
            // failing the row.
            let evaluator = eval.ok_or_else(|| {
                "similarity() predicate found without a vector-collection evaluator".to_string()
            })?;
            Ok(evaluator.truth(sim, idx))
        }
        other => truth(other, id, payload, params),
    }
}

#[cfg(test)]
#[path = "velesql_similarity_tests.rs"]
mod tests;
