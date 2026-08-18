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
