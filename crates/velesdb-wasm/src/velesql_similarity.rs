//! `similarity()` threshold evaluation for WASM WHERE clauses (S4-13).
//!
//! Supports `WHERE similarity(vector, $q) <op> <threshold>` inside a SELECT
//! when the target is a vector collection. The similarity is computed on
//! the fly against the row's current vector; non-matching rows are skipped.
//!
//! This module only decides "is this row's similarity <op> threshold?" —
//! the actual vector NEAR ranking stays in `velesql_select::vector_path`.

use velesdb_core::velesql::{CompareOp, Condition, SimilarityCondition};

use crate::database::DatabaseInner;
use crate::vector_store::VectorStore;
use crate::velesql_value::{resolve_vector, Params};

// VectorStore is used in the `passes()` signature for future-proofing (e.g.
// lazy per-row evaluation); keep the parameter to avoid breaking callers.
const _: fn(&VectorStore) = |_s| {};

/// Walks the condition tree and returns the first `similarity()` predicate.
///
/// A `NOT similarity(v, $q) <op> t` subtree surfaces as a `Similarity`
/// with the operator flipped by [`flip_similarity_op`] so the caller
/// (typically [`SimilarityEvaluator`]) applies the correct polarity.
/// Without this rewrite, the naive walker would descend into the `NOT`
/// wrapper and return the un-flipped inner similarity, silently
/// inverting the user's intent.
///
/// Returns an owned `SimilarityCondition` because the flipped form is
/// synthesized on the fly and has no home in the original AST.
pub(crate) fn find_similarity(cond: Option<&Condition>) -> Option<SimilarityCondition> {
    let cond = cond?;
    match cond {
        Condition::Similarity(s) => Some(s.clone()),
        Condition::And(l, r) | Condition::Or(l, r) => {
            find_similarity(Some(l)).or_else(|| find_similarity(Some(r)))
        }
        Condition::Not(inner) => {
            if let Condition::Similarity(s) = inner.as_ref() {
                let mut flipped = s.clone();
                flipped.operator = flip_similarity_op(s.operator);
                return Some(flipped);
            }
            find_similarity(Some(inner))
        }
        Condition::Group(inner) => find_similarity(Some(inner)),
        _ => None,
    }
}

/// Returns the condition with every `similarity()` predicate removed.
///
/// The input is first passed through [`normalize_not_similarity`] so
/// any `NOT similarity(...)` subtree is rewritten into a regular
/// `similarity(flipped-op, ...)` predicate. Without this normalization,
/// the strip would return `None.map(Not)` = `None`, dropping the whole
/// subtree and silently flipping the polarity (see [`flip_similarity_op`]
/// for the rationale).
pub(crate) fn strip_similarity(cond: Option<&Condition>) -> Option<Condition> {
    let normalized = cond.map(normalize_not_similarity);
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
        Condition::And(l, r) => combine_binary(
            strip_condition_if(Some(l), should_remove),
            strip_condition_if(Some(r), should_remove),
            Condition::And,
        ),
        Condition::Or(l, r) => combine_binary(
            strip_condition_if(Some(l), should_remove),
            strip_condition_if(Some(r), should_remove),
            Condition::Or,
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

/// Rewrites `NOT similarity(v, $q) <op> t` into `similarity(v, $q) <flipped-op> t`.
///
/// Without this rewrite, `strip_condition_if` over `NOT similarity(...)`
/// would recurse into the `NOT` wrapper, strip the inner similarity to
/// `None`, and `None.map(Not)` collapses back to `None` — dropping the
/// whole NOT subtree silently. The companion [`find_similarity`] would
/// then return the un-flipped inner similarity and the executor would
/// apply the threshold with the wrong polarity (e.g. `NOT sim > 0.5`
/// would behave like `sim > 0.5` instead of `sim <= 0.5`).
///
/// The normalization is local: only `NOT` directly wrapping a
/// `Similarity` is rewritten. `NOT (similarity(...) AND x = 1)` is NOT
/// distributed (that would require full De-Morgan rewriting), and the
/// residual error surface is unchanged for non-similarity NOT subtrees.
pub(crate) fn normalize_not_similarity(cond: &Condition) -> Condition {
    match cond {
        Condition::Not(inner) => {
            if let Condition::Similarity(sim) = inner.as_ref() {
                let mut flipped = sim.clone();
                flipped.operator = flip_similarity_op(sim.operator);
                return Condition::Similarity(flipped);
            }
            Condition::Not(Box::new(normalize_not_similarity(inner)))
        }
        Condition::And(l, r) => Condition::And(
            Box::new(normalize_not_similarity(l)),
            Box::new(normalize_not_similarity(r)),
        ),
        Condition::Or(l, r) => Condition::Or(
            Box::new(normalize_not_similarity(l)),
            Box::new(normalize_not_similarity(r)),
        ),
        Condition::Group(inner) => Condition::Group(Box::new(normalize_not_similarity(inner))),
        other => other.clone(),
    }
}

/// Flips a comparison operator to its logical complement.
///
/// Used by [`normalize_not_similarity`] to rewrite a `NOT` around a
/// similarity predicate as a flipped-operator similarity. `CompareOp`
/// is `#[non_exhaustive]`; unknown variants fall back to the input
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

fn combine_binary<F>(l: Option<Condition>, r: Option<Condition>, ctor: F) -> Option<Condition>
where
    F: FnOnce(Box<Condition>, Box<Condition>) -> Condition,
{
    match (l, r) {
        (Some(a), Some(b)) => Some(ctor(Box::new(a), Box::new(b))),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

/// Holds pre-computed similarity scores by row index for fast filtering.
///
/// Scores are computed once at construction time against the full collection
/// so `passes(idx)` is an O(1) lookup instead of re-running the metric
/// kernel for every row in the scan loop.
#[derive(Debug)]
pub(crate) struct SimilarityEvaluator {
    cond: SimilarityCondition,
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
        Ok(Self {
            cond: cond.clone(),
            scores,
        })
    }

    /// Returns `true` if the vector at row `idx` passes the similarity test.
    pub(crate) fn passes(&self, _store: &VectorStore, idx: usize) -> bool {
        let score = self.scores.get(idx).copied().unwrap_or(0.0);
        let op = self.cond.operator;
        #[allow(clippy::cast_possible_truncation)]
        let threshold = self.cond.threshold as f32;
        match op {
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
}
