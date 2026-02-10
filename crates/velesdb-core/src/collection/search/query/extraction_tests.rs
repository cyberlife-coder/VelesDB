//! Tests for `extraction` module - Query condition extraction utilities.

use crate::collection::types::Collection;
use crate::velesql::{
    CompareOp, Comparison, Condition, MatchCondition, SimilarityCondition, Value, VectorExpr,
    VectorSearch,
};

fn make_comparison(column: &str, val: i64) -> Condition {
    Condition::Comparison(Comparison {
        column: column.to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(val),
    })
}

fn make_match(column: &str, query: &str) -> Condition {
    Condition::Match(MatchCondition {
        column: column.to_string(),
        query: query.to_string(),
    })
}

fn make_similarity(field: &str, threshold: f64) -> Condition {
    Condition::Similarity(SimilarityCondition {
        field: field.to_string(),
        vector: VectorExpr::Parameter("v".to_string()),
        operator: CompareOp::Gt,
        threshold,
    })
}

fn make_vector_search() -> Condition {
    Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    })
}

#[test]
fn test_extract_match_query_direct() {
    let cond = make_match("text", "hello world");
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("hello world".to_string()));
}

#[test]
fn test_extract_match_query_in_and() {
    let cond = Condition::And(
        Box::new(make_comparison("a", 1)),
        Box::new(make_match("text", "search term")),
    );
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("search term".to_string()));
}

#[test]
fn test_extract_match_query_in_group() {
    let cond = Condition::Group(Box::new(make_match("text", "query")));
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("query".to_string()));
}

#[test]
fn test_extract_match_query_none() {
    let cond = make_comparison("a", 1);
    let result = Collection::extract_match_query(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_match_query_nested_and() {
    let inner = Condition::And(
        Box::new(make_match("text", "inner query")),
        Box::new(make_comparison("b", 2)),
    );
    let cond = Condition::And(Box::new(make_comparison("a", 1)), Box::new(inner));
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("inner query".to_string()));
}

#[test]
fn test_extract_metadata_filter_comparison() {
    let cond = make_comparison("category", 1);
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_some());
}

#[test]
fn test_extract_metadata_filter_removes_similarity() {
    let cond = make_similarity("embedding", 0.8);
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_removes_vector_search() {
    let cond = make_vector_search();
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_and_with_similarity() {
    let cond = Condition::And(
        Box::new(make_similarity("embedding", 0.8)),
        Box::new(make_comparison("category", 1)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_some());
    assert!(matches!(result, Some(Condition::Comparison(_))));
}

#[test]
fn test_extract_metadata_filter_and_both_metadata() {
    let cond = Condition::And(
        Box::new(make_comparison("a", 1)),
        Box::new(make_comparison("b", 2)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::And(_, _))));
}

#[test]
fn test_extract_metadata_filter_and_both_similarity() {
    let cond = Condition::And(
        Box::new(make_similarity("e1", 0.8)),
        Box::new(make_similarity("e2", 0.9)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_or_both_metadata() {
    let cond = Condition::Or(
        Box::new(make_comparison("a", 1)),
        Box::new(make_comparison("b", 2)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Or(_, _))));
}

#[test]
fn test_extract_metadata_filter_or_with_similarity_returns_none() {
    let cond = Condition::Or(
        Box::new(make_similarity("embedding", 0.8)),
        Box::new(make_comparison("category", 1)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_group() {
    let cond = Condition::Group(Box::new(make_comparison("a", 1)));
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Group(_))));
}

#[test]
fn test_extract_metadata_filter_not() {
    let cond = Condition::Not(Box::new(make_comparison("deleted", 1)));
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Not(_))));
}

#[test]
fn test_extract_metadata_filter_not_similarity_returns_none() {
    let cond = Condition::Not(Box::new(make_similarity("embedding", 0.8)));
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

// --- B-01 Regression Tests: NaN/Infinity vector rejection ---

/// Helper to create a Collection for vector extraction tests.
fn make_test_collection() -> Collection {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    Collection::create(
        dir.path().to_path_buf(),
        3,
        crate::distance::DistanceMetric::Cosine,
    )
    .expect("create collection")
}

#[test]
fn test_nan_vector_components_rejected_in_vector_search() {
    let collection = make_test_collection();
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, f64::NAN, 3.0]));

    let mut cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    });

    let result = collection.extract_vector_search(&mut cond, &params);
    assert!(result.is_err(), "NaN component should be rejected");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("out of f32 range or not a number"),
        "Error should mention invalid value: {err_msg}"
    );
}

#[test]
fn test_infinity_vector_components_rejected_in_vector_search() {
    let collection = make_test_collection();
    let mut params = std::collections::HashMap::new();
    params.insert(
        "v".to_string(),
        serde_json::json!([1.0, f64::INFINITY, 3.0]),
    );

    let mut cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    });

    let result = collection.extract_vector_search(&mut cond, &params);
    assert!(result.is_err(), "Infinity component should be rejected");
}

#[test]
fn test_neg_infinity_vector_components_rejected() {
    let collection = make_test_collection();
    let mut params = std::collections::HashMap::new();
    params.insert(
        "v".to_string(),
        serde_json::json!([f64::NEG_INFINITY, 2.0, 3.0]),
    );

    let mut cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    });

    let result = collection.extract_vector_search(&mut cond, &params);
    assert!(result.is_err(), "NEG_INFINITY component should be rejected");
}

#[test]
fn test_valid_vector_passes_extraction() {
    let collection = make_test_collection();
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, 2.0, 3.0]));

    let mut cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    });

    let result = collection.extract_vector_search(&mut cond, &params);
    assert!(result.is_ok(), "Valid vector should pass");
    let vec = result.unwrap();
    assert!(vec.is_some());
    assert_eq!(vec.unwrap(), vec![1.0_f32, 2.0, 3.0]);
}

#[test]
fn test_nan_rejected_in_similarity_condition() {
    let collection = make_test_collection();
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, f64::NAN, 3.0]));

    let cond = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("v".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });

    let result = collection.extract_all_similarity_conditions(&cond, &params);
    assert!(
        result.is_err(),
        "NaN should be rejected in similarity extraction"
    );
}

#[test]
fn test_nan_rejected_in_resolve_vector() {
    let collection = make_test_collection();
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([f64::NAN, 2.0, 3.0]));

    let vector_expr = VectorExpr::Parameter("v".to_string());
    let result = collection.resolve_vector(&vector_expr, &params);
    assert!(result.is_err(), "NaN should be rejected in resolve_vector");
}

// --- B-02 Regression Test: ORDER BY property path now works (VP-006) ---

#[test]
fn test_order_by_property_path_succeeds() {
    use crate::collection::search::query::match_exec::MatchResult;

    let mut result1 = MatchResult::new(1, 0, vec![]);
    result1
        .projected
        .insert("n.name".to_string(), serde_json::json!("Zebra"));

    let mut result2 = MatchResult::new(2, 1, vec![1]);
    result2
        .projected
        .insert("n.name".to_string(), serde_json::json!("Apple"));

    let mut items = vec![result1, result2];

    // VP-006: ORDER BY property path should now succeed
    let result = Collection::order_match_results(&mut items, "n.name", false);
    assert!(
        result.is_ok(),
        "ORDER BY property path should succeed (VP-006)"
    );
    // ASC: "Apple" < "Zebra"
    assert_eq!(items[0].node_id, 2, "Apple should come first in ASC order");
    assert_eq!(items[1].node_id, 1, "Zebra should come second in ASC order");
}

#[test]
fn test_order_by_similarity_succeeds() {
    use crate::collection::search::query::match_exec::MatchResult;

    let mut results = vec![
        {
            let mut r = MatchResult::new(1, 0, vec![]);
            r.score = Some(0.5);
            r
        },
        {
            let mut r = MatchResult::new(2, 1, vec![1]);
            r.score = Some(0.9);
            r
        },
    ];

    let result = Collection::order_match_results(&mut results, "similarity()", true);
    assert!(result.is_ok(), "ORDER BY similarity() should succeed");
    // DESC: highest score first
    assert_eq!(results[0].node_id, 2);
    assert_eq!(results[1].node_id, 1);
}

#[test]
fn test_order_by_depth_succeeds() {
    use crate::collection::search::query::match_exec::MatchResult;

    let mut results = vec![
        MatchResult::new(1, 3, vec![]),
        MatchResult::new(2, 1, vec![1]),
    ];

    let result = Collection::order_match_results(&mut results, "depth", false);
    assert!(result.is_ok(), "ORDER BY depth should succeed");
    // ASC: lowest depth first
    assert_eq!(results[0].node_id, 2);
    assert_eq!(results[1].node_id, 1);
}
