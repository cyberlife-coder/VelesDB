use super::*;
use crate::velesql::{CompareOp, Comparison, Value};

/// #904: the metadata `Filter` for a leaf condition is built **once** and
/// reused across repeated evaluations of the same AST node (one per result
/// row), instead of rebuilding + cloning per row. Results are unchanged.
#[test]
fn test_metadata_filter_built_once_per_leaf() {
    let cond = Condition::Comparison(Comparison {
        column: "status".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("active".to_string()),
    });

    let mut cache = GraphMatchEvalCache::default();
    let active = serde_json::json!({"status": "active"});
    let inactive = serde_json::json!({"status": "inactive"});

    // Evaluate the SAME borrowed node many times (simulates N result rows).
    for _ in 0..100 {
        let f = cache.metadata_filter(&cond);
        assert!(f.matches(&active));
        assert!(!f.matches(&inactive));
    }

    assert_eq!(
        cache.filters_built(),
        1,
        "metadata Filter must be built exactly once, not per row"
    );
}

/// #904: distinct leaf nodes each get their own cached `Filter`.
#[test]
fn test_metadata_filter_distinct_leaves_cached_separately() {
    let cond_a = Condition::Comparison(Comparison {
        column: "a".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(1),
    });
    let cond_b = Condition::Comparison(Comparison {
        column: "b".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(2),
    });

    let mut cache = GraphMatchEvalCache::default();
    let _ = cache.metadata_filter(&cond_a);
    let _ = cache.metadata_filter(&cond_b);
    let _ = cache.metadata_filter(&cond_a);

    assert_eq!(cache.filters_built(), 2);
}

#[test]
fn test_condition_contains_or_detects_nested_or() {
    let cond = Condition::And(
        Box::new(Condition::Comparison(Comparison {
            column: "status".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("active".to_string()),
        })),
        Box::new(Condition::Group(Box::new(Condition::Or(
            Box::new(Condition::Comparison(Comparison {
                column: "tier".to_string(),
                operator: CompareOp::Eq,
                value: Value::String("pro".to_string()),
            })),
            Box::new(Condition::Comparison(Comparison {
                column: "tier".to_string(),
                operator: CompareOp::Eq,
                value: Value::String("enterprise".to_string()),
            })),
        )))),
    );

    assert!(Collection::condition_contains_or(&cond));
}

#[test]
fn test_condition_contains_or_false_without_or() {
    let cond = Condition::And(
        Box::new(Condition::Comparison(Comparison {
            column: "status".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("active".to_string()),
        })),
        Box::new(Condition::Not(Box::new(Condition::Comparison(
            Comparison {
                column: "deleted".to_string(),
                operator: CompareOp::Eq,
                value: Value::Boolean(true),
            },
        )))),
    );

    assert!(!Collection::condition_contains_or(&cond));
}

// ============================================================================
// Regression (#2106 item 2): the graph-predicate WHERE evaluator scored an
// unscorable pair (length mismatch) as 0.0. On a distance metric that reads
// as a perfect match, so `similarity() > t` admitted the malformed record.
// ============================================================================

#[cfg(feature = "persistence")]
mod similarity_length_mismatch {
    use crate::collection::Collection;
    use crate::distance::DistanceMetric;
    use crate::point::{Point, SearchResult};
    use crate::velesql::{CompareOp, Condition, SimilarityCondition, VectorExpr};
    use std::path::PathBuf;

    /// `similarity(v, $q) > 0.9` against a query vector of the wrong length
    /// must admit nothing — pre-fix the fabricated 0.0 distance inverted into
    /// "closer than 0.9" and the record passed.
    #[test]
    fn test_where_similarity_rejects_length_mismatched_query_vector() {
        let dir = tempfile::tempdir().expect("test: tempdir");
        let col = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Euclidean)
            .expect("test: collection");

        let results = vec![SearchResult::new(
            Point {
                id: 1,
                vector: vec![3.0, 0.0],
                payload: None,
                sparse_vectors: None,
            },
            0.0,
        )];
        let condition = Condition::Similarity(SimilarityCondition {
            field: "vector".to_string(),
            vector: VectorExpr::Literal(vec![0.0, 0.0, 0.0]),
            operator: CompareOp::Gt,
            threshold: 0.9,
        });

        let filtered = col
            .apply_where_condition_to_results(
                results,
                &condition,
                &std::collections::HashMap::new(),
                &[],
            )
            .expect("test: where evaluation");

        assert!(
            filtered.is_empty(),
            "an unscorable vector pair must fail the predicate, got {filtered:?}"
        );
    }
}
