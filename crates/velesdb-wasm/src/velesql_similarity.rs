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
pub(crate) fn find_similarity(cond: Option<&Condition>) -> Option<&SimilarityCondition> {
    let cond = cond?;
    match cond {
        Condition::Similarity(s) => Some(s),
        Condition::And(l, r) | Condition::Or(l, r) => {
            find_similarity(Some(l)).or_else(|| find_similarity(Some(r)))
        }
        Condition::Not(inner) | Condition::Group(inner) => find_similarity(Some(inner)),
        _ => None,
    }
}

/// Returns the condition with every `similarity()` predicate removed.
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
        let err = SimilarityEvaluator::new(&db, "t", sim, &params);
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
        let err = SimilarityEvaluator::new(&db, "v", sim, &params);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("dimension mismatch"));
    }
}
