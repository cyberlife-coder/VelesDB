//! Generic recursive helpers for walking VelesQL condition trees.
//!
//! Eliminates per-module duplication of the `And/Or -> recurse, Group/Not -> recurse, _ -> base`
//! pattern that appears across validation, extraction, where_eval, and hybrid_sparse modules.

use crate::velesql::Condition;

/// Returns `true` if any subtree of `condition` satisfies `predicate`.
///
/// Walks `And`, `Or`, `Group`, and `Not` combinators recursively.
pub(crate) fn any_subtree(condition: &Condition, predicate: &dyn Fn(&Condition) -> bool) -> bool {
    if predicate(condition) {
        return true;
    }
    match condition {
        Condition::And(left, right) | Condition::Or(left, right) => {
            any_subtree(left, predicate) || any_subtree(right, predicate)
        }
        Condition::Group(inner) | Condition::Not(inner) => any_subtree(inner, predicate),
        _ => false,
    }
}

/// Recursively counts leaves matching `predicate`.
pub(crate) fn count_matching_leaves(
    condition: &Condition,
    predicate: fn(&Condition) -> bool,
) -> usize {
    if predicate(condition) {
        return 1;
    }
    match condition {
        Condition::And(left, right) | Condition::Or(left, right) => {
            count_matching_leaves(left, predicate) + count_matching_leaves(right, predicate)
        }
        Condition::Group(inner) | Condition::Not(inner) => count_matching_leaves(inner, predicate),
        _ => 0,
    }
}

/// Returns `true` if the condition is a vector-type leaf
/// (`Similarity`, `VectorSearch`, `VectorFusedSearch`).
pub(crate) fn is_vector_leaf(condition: &Condition) -> bool {
    matches!(
        condition,
        Condition::Similarity(_) | Condition::VectorSearch(_) | Condition::VectorFusedSearch(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{
        CompareOp, Comparison, FusionConfig, SimilarityCondition, Value, VectorExpr,
        VectorFusedSearch, VectorSearch,
    };

    /// Helper: builds a `Condition::Similarity` leaf.
    fn similarity_leaf() -> Condition {
        Condition::Similarity(SimilarityCondition {
            field: "embedding".to_string(),
            vector: VectorExpr::Parameter("v".to_string()),
            operator: CompareOp::Gt,
            threshold: 0.8,
        })
    }

    /// Helper: builds a `Condition::Comparison` leaf (non-vector).
    fn comparison_leaf(column: &str) -> Condition {
        Condition::Comparison(Comparison {
            column: column.to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        })
    }

    /// Helper: builds a `Condition::VectorSearch` leaf.
    fn vector_search_leaf() -> Condition {
        Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1, 0.2, 0.3]),
        })
    }

    /// Helper: builds a `Condition::VectorFusedSearch` leaf.
    fn vector_fused_leaf() -> Condition {
        Condition::VectorFusedSearch(VectorFusedSearch {
            vectors: vec![
                VectorExpr::Literal(vec![0.1, 0.2]),
                VectorExpr::Literal(vec![0.3, 0.4]),
            ],
            fusion: FusionConfig::rrf(),
        })
    }

    // ── any_subtree ──────────────────────────────────────────────────

    #[test]
    fn test_any_subtree_leaf_match() {
        let cond = similarity_leaf();
        assert!(any_subtree(&cond, &is_vector_leaf));
    }

    #[test]
    fn test_any_subtree_leaf_no_match() {
        let cond = comparison_leaf("age");
        assert!(!any_subtree(&cond, &is_vector_leaf));
    }

    #[test]
    fn test_any_subtree_nested_and() {
        // And(comparison, And(comparison, similarity))
        let inner_and = Condition::And(
            Box::new(comparison_leaf("status")),
            Box::new(similarity_leaf()),
        );
        let root = Condition::And(Box::new(comparison_leaf("age")), Box::new(inner_and));
        assert!(any_subtree(&root, &is_vector_leaf));
    }

    #[test]
    fn test_any_subtree_nested_not_group() {
        // Not(Group(VectorSearch))
        let cond = Condition::Not(Box::new(Condition::Group(Box::new(vector_search_leaf()))));
        assert!(any_subtree(&cond, &is_vector_leaf));
    }

    // ── count_matching_leaves ────────────────────────────────────────

    #[test]
    fn test_count_matching_leaves_zero() {
        // Tree with no vector leaves at all.
        let cond = Condition::And(
            Box::new(comparison_leaf("a")),
            Box::new(Condition::Or(
                Box::new(comparison_leaf("b")),
                Box::new(comparison_leaf("c")),
            )),
        );
        assert_eq!(count_matching_leaves(&cond, is_vector_leaf), 0);
    }

    #[test]
    fn test_count_matching_leaves_nested() {
        // And(Similarity, Or(VectorSearch, Comparison)) -> 2 vector leaves.
        let cond = Condition::And(
            Box::new(similarity_leaf()),
            Box::new(Condition::Or(
                Box::new(vector_search_leaf()),
                Box::new(comparison_leaf("x")),
            )),
        );
        assert_eq!(count_matching_leaves(&cond, is_vector_leaf), 2);
    }

    // ── is_vector_leaf ───────────────────────────────────────────────

    #[test]
    fn test_is_vector_leaf_variants() {
        // Positive cases: all three vector leaf types.
        assert!(is_vector_leaf(&similarity_leaf()));
        assert!(is_vector_leaf(&vector_search_leaf()));
        assert!(is_vector_leaf(&vector_fused_leaf()));

        // Negative cases: non-vector leaves and combinators.
        assert!(!is_vector_leaf(&comparison_leaf("col")));
        assert!(!is_vector_leaf(&Condition::And(
            Box::new(similarity_leaf()),
            Box::new(comparison_leaf("col")),
        )));
        assert!(!is_vector_leaf(&Condition::Or(
            Box::new(comparison_leaf("a")),
            Box::new(comparison_leaf("b")),
        )));
        assert!(!is_vector_leaf(&Condition::Not(
            Box::new(similarity_leaf())
        )));
        assert!(!is_vector_leaf(&Condition::Group(Box::new(
            similarity_leaf()
        ))));
    }
}
