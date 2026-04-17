//! `similarity()` threshold evaluation for WASM WHERE clauses (S4-13).
//!
//! Supports `WHERE similarity(vector, $q) <op> <threshold>` inside a SELECT
//! when the target is a vector collection. The similarity is computed on
//! the fly against the row's current vector; non-matching rows are skipped.
//!
//! This module only decides "is this row's similarity <op> threshold?" —
//! the actual vector NEAR ranking stays in `velesql_select::vector_path`.
//!
//! # Limitations (WASM pre-seed scope)
//!
//! - **Single query vector per statement.** [`SimilarityEvaluator`]
//!   pre-computes scores against one query vector for the whole row scan.
//!   WHERE clauses that mix `similarity()` predicates against different
//!   query vectors are rejected via
//!   [`assert_single_similarity_vector`]. Use the core
//!   (persistence-enabled) backend for multi-vector queries.

use velesdb_core::velesql::{CompareOp, Condition, SimilarityCondition, VectorExpr};

use crate::database::DatabaseInner;
use crate::velesql_logic::push_not_inward;
use crate::velesql_value::{resolve_vector, Params};

/// Walks the condition tree and returns the first `similarity()` predicate.
///
/// The input is first passed through [`push_not_inward`] so every `NOT`
/// is pushed all the way down to leaves via De Morgan distribution. As
/// a result this walker only ever encounters bare `Similarity(_)`
/// leaves (with the operator already flipped if the original query
/// wrapped them in one or more `NOT`s) — no special-case handling for
/// `NOT Similarity(...)` is needed here. This correctly covers all
/// compound forms such as `NOT (sim > 0.5 AND x = 1)`, which
/// De-Morgan-rewrites to `sim <= 0.5 OR x != 1` before the walk.
///
/// Returns an owned `SimilarityCondition` because [`push_not_inward`]
/// may have synthesized a flipped-op form that has no home in the
/// caller's borrowed AST.
pub(crate) fn find_similarity(cond: Option<&Condition>) -> Option<SimilarityCondition> {
    cond.cloned()
        .map(push_not_inward)
        .as_ref()
        .and_then(find_similarity_normalized)
}

/// Recursive walk over a condition that is already De-Morgan-normalized:
/// every `NOT` wraps a negation-agnostic leaf (`LIKE`, `BETWEEN`, ...),
/// and no `NOT` wraps a compound or a `Similarity`. See
/// [`find_similarity`] for the normalization contract.
fn find_similarity_normalized(cond: &Condition) -> Option<SimilarityCondition> {
    match cond {
        Condition::Similarity(s) => Some(s.clone()),
        Condition::And(l, r) | Condition::Or(l, r) => {
            find_similarity_normalized(l).or_else(|| find_similarity_normalized(r))
        }
        Condition::Not(inner) | Condition::Group(inner) => find_similarity_normalized(inner),
        _ => None,
    }
}

/// Rejects WHERE clauses that contain two `similarity()` predicates
/// referencing different query vectors (finding H).
///
/// [`SimilarityEvaluator`] pre-computes scores against ONE query vector
/// (the first similarity predicate found). If the WHERE tree contains a
/// second `similarity()` with a DIFFERENT vector, the evaluator would
/// silently reuse the first vector's scores for the second threshold —
/// returning wrong rows.
///
/// Rather than extending the evaluator to key scores by vector (more
/// code for a pre-seed WASM demo surface, plus N× more scorer passes),
/// we fail loud: the caller gets an explicit error pointing them at the
/// core (persistence-enabled) backend for multi-vector queries.
///
/// Identity is by `VectorExpr` equality, which treats two distinct
/// parameter names as distinct and two distinct literal vectors as
/// distinct — the same binding / literal used twice is accepted.
///
/// The input is expected to already be De-Morgan-normalized via
/// [`push_not_inward`] so no `NOT` wraps a `Similarity(_)`.
pub(crate) fn assert_single_similarity_vector(cond: Option<&Condition>) -> Result<(), String> {
    let mut first: Option<VectorExpr> = None;
    if let Some(c) = cond {
        walk_similarity_vectors(c, &mut |v| match &first {
            None => {
                first = Some(v.clone());
                Ok(())
            }
            Some(existing) if existing == v => Ok(()),
            Some(_) => Err(
                "Multiple similarity() conditions with different query vectors are not yet \
                 supported in WASM. Use a single similarity() predicate or use core \
                 (persistence-enabled) for multi-vector queries."
                    .to_string(),
            ),
        })?;
    }
    Ok(())
}

/// Visits every `SimilarityCondition` in `cond` and calls `visit` with
/// its `vector` expression. Short-circuits on the first `Err` returned
/// by the visitor — used by [`assert_single_similarity_vector`] to stop
/// as soon as a second distinct vector is seen.
fn walk_similarity_vectors<F>(cond: &Condition, visit: &mut F) -> Result<(), String>
where
    F: FnMut(&VectorExpr) -> Result<(), String>,
{
    match cond {
        Condition::Similarity(s) => visit(&s.vector),
        Condition::And(l, r) | Condition::Or(l, r) => {
            walk_similarity_vectors(l, visit)?;
            walk_similarity_vectors(r, visit)
        }
        Condition::Not(inner) | Condition::Group(inner) => walk_similarity_vectors(inner, visit),
        _ => Ok(()),
    }
}

/// Returns the condition with every `similarity()` predicate removed.
///
/// The input is first passed through [`push_not_inward`] so NOT is
/// De-Morgan-distributed before stripping. Without this, a compound
/// NOT like `NOT (sim > 0.5 AND x = 1)` would recurse into the NOT
/// wrapper, collapse the inner similarity to `None`, and leave
/// `NOT(x = 1)` as residual — semantically equivalent to
/// `sim > 0.5 AND NOT(x = 1)` instead of the correct
/// `sim <= 0.5 OR x != 1`.
///
/// Once normalized, the strip pass only sees bare `Similarity(_)`
/// leaves, which [`strip_condition_if`] removes while preserving the
/// surrounding And/Or/Not/Group structure.
pub(crate) fn strip_similarity(cond: Option<&Condition>) -> Option<Condition> {
    let normalized = cond.cloned().map(push_not_inward);
    strip_condition_if(normalized.as_ref(), &|c| {
        matches!(c, Condition::Similarity(_))
    })
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

/// Holds pre-computed similarity scores by row index for fast filtering.
///
/// Scores are computed once at construction time against the full
/// collection so [`Self::passes_with_cond`] is an O(1) lookup instead
/// of re-running the metric kernel for every row in the scan loop.
#[derive(Debug)]
pub(crate) struct SimilarityEvaluator {
    /// Score at row `i` for the pre-resolved query vector.
    scores: Vec<f32>,
}

impl SimilarityEvaluator {
    /// Builds an evaluator for a vector collection, resolving the query
    /// vector against `$param` bindings and validating dimension.
    pub(crate) fn new(
        db: &DatabaseInner,
        collection: &str,
        cond: &SimilarityCondition,
        params: &Params,
    ) -> Result<Self, String> {
        let store = db.get_shared_store(collection)?;
        let borrowed = store.borrow();
        if borrowed.dimension() == 0 {
            return Err(format!(
                "Collection '{collection}' is metadata-only; similarity() requires a vector collection"
            ));
        }
        let query = resolve_vector(&cond.vector, params)?;
        if query.len() != borrowed.dimension() {
            return Err(format!(
                "similarity() query dimension mismatch: expected {}, got {}",
                borrowed.dimension(),
                query.len()
            ));
        }
        let scored = crate::vector_ops::compute_scores(
            &query,
            &borrowed.ids,
            &borrowed.data,
            &borrowed.data_sq8,
            &borrowed.data_binary,
            &borrowed.sq8_mins,
            &borrowed.sq8_scales,
            borrowed.dimension,
            borrowed.metric,
            borrowed.storage_mode,
        );
        let scores = scored.into_iter().map(|(_, s)| s).collect();
        Ok(Self { scores })
    }

    /// Returns `true` if the given `sim` predicate passes at row `idx`.
    ///
    /// Used by [`evaluate_where_with_similarity`] when it encounters a
    /// `Similarity` leaf. The score cache is reused so we don't recompute
    /// vectors per row; the operator and threshold come from the AST
    /// node so rewrites by [`push_not_inward`] (e.g. `>` flipped to `<=`
    /// under a NOT) surface with the right polarity without mutating
    /// the evaluator.
    pub(crate) fn passes_with_cond(&self, cond: &SimilarityCondition, idx: usize) -> bool {
        let score = self.scores.get(idx).copied().unwrap_or(0.0);
        #[allow(clippy::cast_possible_truncation)]
        let threshold = cond.threshold as f32;
        match cond.operator {
            CompareOp::Gt => score > threshold,
            CompareOp::Gte => score >= threshold,
            CompareOp::Lt => score < threshold,
            CompareOp::Lte => score <= threshold,
            CompareOp::Eq => (score - threshold).abs() < f32::EPSILON,
            CompareOp::NotEq => (score - threshold).abs() >= f32::EPSILON,
            _ => false,
        }
    }
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
/// Non-similarity leaves delegate to [`crate::velesql_where::matches`]
/// for identical semantics. The input `cond` is expected to already
/// be De-Morgan-normalized via [`push_not_inward`]; callers that build
/// the condition through [`find_similarity`] / [`strip_similarity`]
/// do this automatically.
pub(crate) fn evaluate_where_with_similarity(
    cond: &Condition,
    id: u64,
    payload: Option<&serde_json::Value>,
    idx: usize,
    eval: Option<&SimilarityEvaluator>,
    params: &Params,
) -> Result<bool, String> {
    match cond {
        Condition::And(l, r) => Ok(evaluate_where_with_similarity(
            l, id, payload, idx, eval, params,
        )? && evaluate_where_with_similarity(
            r, id, payload, idx, eval, params,
        )?),
        Condition::Or(l, r) => Ok(evaluate_where_with_similarity(
            l, id, payload, idx, eval, params,
        )? || evaluate_where_with_similarity(
            r, id, payload, idx, eval, params,
        )?),
        Condition::Not(inner) => Ok(!evaluate_where_with_similarity(
            inner, id, payload, idx, eval, params,
        )?),
        Condition::Group(inner) => {
            evaluate_where_with_similarity(inner, id, payload, idx, eval, params)
        }
        Condition::Similarity(sim) => {
            // No evaluator => no vector collection context; this should
            // never happen when the query was validated upstream, but we
            // surface a clear error rather than silently passing or
            // failing the row.
            let evaluator = eval.ok_or_else(|| {
                "similarity() predicate found without a vector-collection evaluator".to_string()
            })?;
            Ok(evaluator.passes_with_cond(sim, idx))
        }
        other => crate::velesql_where::matches(other, id, payload, params),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::velesql::Parser;

    fn parse_cond(sql: &str) -> Condition {
        let q = Parser::parse(sql).expect("test: parse");
        q.select.where_clause.expect("test: where")
    }

    #[test]
    fn test_find_similarity_returns_predicate() {
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.8");
        assert!(find_similarity(Some(&c)).is_some());
    }

    #[test]
    fn test_find_similarity_returns_none_when_absent() {
        let c = parse_cond("SELECT * FROM t WHERE x = 1");
        assert!(find_similarity(Some(&c)).is_none());
    }

    #[test]
    fn test_strip_similarity_keeps_other_predicates() {
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.8 AND x = 1");
        let stripped = strip_similarity(Some(&c)).expect("test: stripped");
        assert!(find_similarity(Some(&stripped)).is_none());
    }

    #[test]
    fn test_strip_similarity_returns_none_when_only_pred() {
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.8");
        assert!(strip_similarity(Some(&c)).is_none());
    }

    #[test]
    fn test_evaluator_rejects_metadata_only_collection() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("t").expect("test: create");
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5");
        let sim = find_similarity(Some(&c)).expect("test: has sim");
        let params =
            crate::velesql_value::parse_params(Some(r#"{"q": [1.0, 0.0]}"#)).expect("test: params");
        let err = SimilarityEvaluator::new(&db, "t", &sim, &params);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("metadata-only"));
    }

    #[test]
    fn test_evaluator_rejects_dim_mismatch() {
        let mut db = DatabaseInner::new();
        db.create_collection("v", 4, "cosine")
            .expect("test: create");
        let c = parse_cond("SELECT * FROM v WHERE similarity(vector, $q) > 0.5");
        let sim = find_similarity(Some(&c)).expect("test: has sim");
        let params =
            crate::velesql_value::parse_params(Some(r#"{"q": [1.0, 0.0]}"#)).expect("test: params");
        let err = SimilarityEvaluator::new(&db, "v", &sim, &params);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("dimension mismatch"));
    }

    // --- NOT similarity rewriting (finding E) ---------------------------------

    #[test]
    fn test_flip_similarity_op_is_logical_complement() {
        assert_eq!(flip_similarity_op(CompareOp::Gt), CompareOp::Lte);
        assert_eq!(flip_similarity_op(CompareOp::Gte), CompareOp::Lt);
        assert_eq!(flip_similarity_op(CompareOp::Lt), CompareOp::Gte);
        assert_eq!(flip_similarity_op(CompareOp::Lte), CompareOp::Gt);
        assert_eq!(flip_similarity_op(CompareOp::Eq), CompareOp::NotEq);
        assert_eq!(flip_similarity_op(CompareOp::NotEq), CompareOp::Eq);
    }

    #[test]
    fn test_find_similarity_flips_op_under_not() {
        let c = parse_cond("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5");
        let sim = find_similarity(Some(&c)).expect("test: flipped similarity");
        // `NOT sim > 0.5` surfaces as `sim <= 0.5`.
        assert_eq!(sim.operator, CompareOp::Lte);
        assert!((sim.threshold - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_strip_similarity_removes_not_wrapped_predicate() {
        // After strip, the similarity subtree (including its NOT wrapper)
        // must be gone from the residual. Without the normalization, the
        // pre-fix implementation left the raw (un-flipped) similarity
        // behind, which velesql_where::matches would reject.
        let c = parse_cond("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5");
        let stripped = strip_similarity(Some(&c));
        assert!(
            stripped.is_none(),
            "NOT similarity predicate must be fully removed from residual, got {stripped:?}"
        );
    }

    #[test]
    fn test_strip_similarity_under_not_with_conjunction_keeps_residual() {
        // `NOT sim > 0.5 AND x = 1` → after strip, only `x = 1` remains,
        // since the NOT-similarity is surfaced via find_similarity with a
        // flipped op.
        let c = parse_cond("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5 AND x = 1");
        let stripped = strip_similarity(Some(&c)).expect("test: residual kept");
        assert!(find_similarity(Some(&stripped)).is_none());
    }

    #[test]
    fn test_find_similarity_preserves_op_without_not() {
        // Non-regression: no NOT wrapper means the operator is returned
        // as-is (no accidental flip).
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) >= 0.8");
        let sim = find_similarity(Some(&c)).expect("test: plain similarity");
        assert_eq!(sim.operator, CompareOp::Gte);
    }

    // --- combine_after_strip: finding G ---------------------------------------

    fn leaf(op: CompareOp, v: i64) -> Condition {
        use velesdb_core::velesql::{Comparison, Value};
        Condition::Comparison(Comparison {
            column: "x".into(),
            operator: op,
            value: Value::Integer(v),
        })
    }

    #[test]
    fn test_combine_and_both_some_rebuilds_and_node() {
        let a = leaf(CompareOp::Eq, 1);
        let b = leaf(CompareOp::Eq, 2);
        let out = combine_after_strip(Some(a), Some(b), LogicalOp::And);
        assert!(matches!(out, Some(Condition::And(_, _))));
    }

    #[test]
    fn test_combine_and_one_none_returns_surviving_side() {
        let a = leaf(CompareOp::Eq, 1);
        let out = combine_after_strip(Some(a.clone()), None, LogicalOp::And);
        assert_eq!(out, Some(a.clone()));
        let out = combine_after_strip(None, Some(a.clone()), LogicalOp::And);
        assert_eq!(out, Some(a));
    }

    #[test]
    fn test_combine_and_both_none_is_none() {
        assert!(combine_after_strip(None, None, LogicalOp::And).is_none());
    }

    #[test]
    fn test_combine_or_both_some_rebuilds_or_node() {
        let a = leaf(CompareOp::Eq, 1);
        let b = leaf(CompareOp::Eq, 2);
        let out = combine_after_strip(Some(a), Some(b), LogicalOp::Or);
        assert!(matches!(out, Some(Condition::Or(_, _))));
    }

    #[test]
    fn test_combine_or_one_none_collapses_to_none() {
        // The key fix for finding G: `true OR x = true`, so the residual
        // post-filter is None — NOT the surviving side.
        let a = leaf(CompareOp::Eq, 1);
        assert!(combine_after_strip(Some(a.clone()), None, LogicalOp::Or).is_none());
        assert!(combine_after_strip(None, Some(a), LogicalOp::Or).is_none());
    }

    #[test]
    fn test_combine_or_both_none_is_none() {
        assert!(combine_after_strip(None, None, LogicalOp::Or).is_none());
    }

    #[test]
    fn test_strip_similarity_or_predicate_collapses_to_none() {
        // BDD-style regression of finding G at the strip layer: a
        // `similarity() OR x = 1` query has `None` residual after
        // stripping the similarity leaf — the OR is trivially satisfied.
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5 OR x = 1");
        assert!(
            strip_similarity(Some(&c)).is_none(),
            "OR(stripped, x) must collapse to None (true OR x = true)"
        );
    }

    #[test]
    fn test_strip_similarity_and_predicate_keeps_predicate() {
        // Non-regression: `similarity() AND x = 1` still strips to `x = 1`.
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5 AND x = 1");
        let residual = strip_similarity(Some(&c)).expect("test: residual");
        assert!(find_similarity(Some(&residual)).is_none());
    }

    // --- assert_single_similarity_vector: finding H ---------------------------

    #[test]
    fn test_assert_single_sim_vec_accepts_none() {
        assert!(assert_single_similarity_vector(None).is_ok());
    }

    #[test]
    fn test_assert_single_sim_vec_accepts_no_similarity() {
        let c = parse_cond("SELECT * FROM t WHERE x = 1");
        assert!(assert_single_similarity_vector(Some(&c)).is_ok());
    }

    #[test]
    fn test_assert_single_sim_vec_accepts_single_predicate() {
        let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5");
        assert!(assert_single_similarity_vector(Some(&c)).is_ok());
    }

    #[test]
    fn test_assert_single_sim_vec_accepts_same_param_twice() {
        let c = parse_cond(
            "SELECT * FROM t WHERE similarity(vector, $q) > 0.5 AND similarity(vector, $q) < 0.9",
        );
        assert!(assert_single_similarity_vector(Some(&c)).is_ok());
    }

    #[test]
    fn test_assert_single_sim_vec_rejects_different_params_and() {
        let c = parse_cond(
            "SELECT * FROM t WHERE similarity(vector, $a) > 0.5 AND similarity(vector, $b) > 0.3",
        );
        let err = assert_single_similarity_vector(Some(&c));
        assert!(err.is_err());
        assert!(
            err.expect_err("test: err")
                .contains("Multiple similarity()"),
            "error must name the feature"
        );
    }

    #[test]
    fn test_assert_single_sim_vec_rejects_different_params_or() {
        let c = parse_cond(
            "SELECT * FROM t WHERE similarity(vector, $a) > 0.5 OR similarity(vector, $b) > 0.3",
        );
        assert!(assert_single_similarity_vector(Some(&c)).is_err());
    }

    #[test]
    fn test_assert_single_sim_vec_rejects_param_vs_literal() {
        // A param vector and a literal vector are distinct VectorExprs
        // even if the runtime-bound value would match — identity is at
        // the AST level.
        let c = parse_cond(
            "SELECT * FROM t WHERE similarity(vector, $q) > 0.5 AND similarity(vector, [1.0, 0.0]) > 0.3",
        );
        assert!(assert_single_similarity_vector(Some(&c)).is_err());
    }

    #[test]
    fn test_assert_single_sim_vec_accepts_same_literal_twice() {
        let c = parse_cond(
            "SELECT * FROM t WHERE similarity(vector, [1.0, 0.0]) > 0.5 AND similarity(vector, [1.0, 0.0]) < 0.9",
        );
        assert!(assert_single_similarity_vector(Some(&c)).is_ok());
    }

    #[test]
    fn test_assert_single_sim_vec_walks_under_not_compound() {
        // A compound `NOT (sim_a AND sim_b)` must still be caught.
        // After De-Morgan normalization it becomes `sim_a' OR sim_b'` with
        // flipped ops — the two distinct vectors are still present.
        let raw = parse_cond(
            "SELECT * FROM t WHERE NOT (similarity(vector, $a) > 0.5 AND similarity(vector, $b) > 0.3)",
        );
        let normalized = push_not_inward(raw);
        assert!(assert_single_similarity_vector(Some(&normalized)).is_err());
    }
}
