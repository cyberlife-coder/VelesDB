//! Tests for `extraction` module - Query condition extraction utilities.

use crate::collection::types::Collection;
use crate::velesql::{
    CompareOp, Comparison, Condition, MatchCondition, Parser, SimilarityCondition, Value,
    VectorExpr, VectorSearch,
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

fn make_graph_match() -> Condition {
    let query = Parser::parse("SELECT * FROM docs WHERE MATCH (d:Doc)-[:REFERENCES]->(x)").unwrap();
    match query.select.where_clause {
        Some(condition) => condition,
        None => panic!("expected where clause"),
    }
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

#[test]
fn test_extract_metadata_filter_removes_graph_match() {
    let cond = make_graph_match();
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_and_with_graph_match() {
    let cond = Condition::And(
        Box::new(make_comparison("category", 1)),
        Box::new(make_graph_match()),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Comparison(_))));
}

#[test]
fn test_collect_graph_match_predicates_nested() {
    let cond = Condition::And(
        Box::new(make_comparison("a", 1)),
        Box::new(Condition::Or(
            Box::new(make_graph_match()),
            Box::new(Condition::Not(Box::new(make_graph_match()))),
        )),
    );
    let mut predicates = Vec::new();
    Collection::collect_graph_match_predicates(&cond, &mut predicates);
    assert_eq!(predicates.len(), 2);
}

// =============================================================================
// B-01 Regression Tests: NaN/Infinity vector rejection
// =============================================================================

#[test]
fn test_resolve_vector_rejects_nan_json_null() {
    // JSON spec doesn't support NaN — serde_json serializes NaN as null.
    // The important thing is the value IS rejected, not what error message says.
    let vector = VectorExpr::Parameter("v".to_string());
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, null, 3.0]));
    let result = Collection::resolve_vector(&vector, &params);
    assert!(result.is_err(), "NaN (as JSON null) must be rejected");
}

#[test]
fn test_resolve_vector_rejects_infinity_json_null() {
    // JSON spec doesn't support Infinity — serde_json serializes it as null.
    let vector = VectorExpr::Parameter("v".to_string());
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, 2.0, null]));
    let result = Collection::resolve_vector(&vector, &params);
    assert!(result.is_err(), "Infinity (as JSON null) must be rejected");
}

#[test]
fn test_resolve_vector_rejects_string_in_array() {
    // Non-numeric values in the vector array must be rejected.
    let vector = VectorExpr::Parameter("v".to_string());
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, "NaN", 3.0]));
    let result = Collection::resolve_vector(&vector, &params);
    assert!(result.is_err(), "String 'NaN' in vector must be rejected");
}

#[test]
fn test_resolve_vector_accepts_valid_values() {
    let vector = VectorExpr::Parameter("v".to_string());
    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.1, -0.5, 0.99]));
    let result = Collection::resolve_vector(&vector, &params);
    assert!(result.is_ok());
    let vec = result.unwrap();
    assert_eq!(vec.len(), 3);
    assert!((vec[0] - 0.1).abs() < 0.001);
}

#[test]
fn test_resolve_vector_rejects_non_number() {
    let vector = VectorExpr::Parameter("v".to_string());
    let mut params = std::collections::HashMap::new();
    params.insert(
        "v".to_string(),
        serde_json::json!([1.0, "not_a_number", 3.0]),
    );
    let result = Collection::resolve_vector(&vector, &params);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not a number"),
        "Error should mention 'not a number': {err_msg}"
    );
}
