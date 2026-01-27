//! Tests for `extraction` module - Query condition extraction utilities.

use super::extraction::*;
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
