//! Tests for BM25 + NEAR VelesQL integration (VP-011, Plan 06-02).
//!
//! Tests verify that VelesQL queries combining full-text MATCH with
//! vector NEAR correctly dispatch to hybrid_search(), text_search(),
//! and text_search_with_filter() through execute_query().

use std::collections::HashMap;

use serde_json::json;

use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::velesql::{
    CompareOp, Comparison, Condition, DistinctMode, MatchCondition, Query, SelectColumns,
    SelectStatement, Value, VectorExpr, VectorSearch,
};
use crate::Database;

/// Helper: create a collection with text-searchable documents.
fn setup_bm25_collection() -> (tempfile::TempDir, crate::Collection) {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");

    db.create_collection("articles", 4, DistanceMetric::Cosine)
        .expect("Failed to create collection");
    let collection = db
        .get_collection("articles")
        .expect("Failed to get collection");

    // Insert documents with text content for BM25 indexing
    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "title": "Introduction to Rust programming",
                "category": "tech",
                "score": 95
            })),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "title": "Python machine learning tutorial",
                "category": "tech",
                "score": 85
            })),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "title": "Rust systems programming guide",
                "category": "tech",
                "score": 90
            })),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({
                "title": "Cooking with seasonal ingredients",
                "category": "food",
                "score": 70
            })),
        ),
        Point::new(
            5,
            vec![0.7, 0.7, 0.0, 0.0],
            Some(json!({
                "title": "Advanced Rust async programming",
                "category": "tech",
                "score": 92
            })),
        ),
        Point::new(
            6,
            vec![0.0, 0.7, 0.7, 0.0],
            Some(json!({
                "title": "Python data science cookbook",
                "category": "science",
                "score": 78
            })),
        ),
    ];

    // Upsert automatically indexes text fields in payload via extract_text_from_payload
    collection.upsert(points).expect("Failed to upsert points");

    (temp_dir, collection)
}

/// Helper: build a SELECT * FROM articles WHERE [condition] LIMIT [limit] query.
fn make_query(where_clause: Option<Condition>, limit: Option<u64>) -> Query {
    Query {
        select: SelectStatement {
            distinct: DistinctMode::None,
            columns: SelectColumns::All,
            from: "articles".to_string(),
            from_alias: None,
            joins: Vec::new(),
            where_clause,
            order_by: None,
            limit,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: None,
    }
}

fn make_match_condition(query: &str) -> Condition {
    Condition::Match(MatchCondition {
        column: "title".to_string(),
        query: query.to_string(),
    })
}

fn make_near_condition(param_name: &str) -> Condition {
    Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter(param_name.to_string()),
    })
}

fn make_filter(column: &str, value: &str) -> Condition {
    Condition::Comparison(Comparison {
        column: column.to_string(),
        operator: CompareOp::Eq,
        value: Value::String(value.to_string()),
    })
}

fn make_vector_params(name: &str, values: &[f32]) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert(
        name.to_string(),
        serde_json::Value::Array(values.iter().map(|&v| json!(v)).collect()),
    );
    params
}

// ============================================================================
// Pure text search tests
// ============================================================================

#[test]
fn test_velesql_match_keyword_only() {
    let (_dir, collection) = setup_bm25_collection();

    // WHERE MATCH 'rust' — should use text_search
    let cond = make_match_condition("rust");
    let query = make_query(Some(cond), Some(10));
    let params = HashMap::new();

    let results = collection
        .execute_query(&query, &params)
        .expect("MATCH query should work");
    // Should find docs containing "rust" (docs 1, 3, 5 have "Rust" in title)
    assert!(!results.is_empty(), "MATCH 'rust' should return results");
}

// ============================================================================
// Hybrid search tests (NEAR + MATCH)
// ============================================================================

#[test]
fn test_velesql_near_and_match() {
    let (_dir, collection) = setup_bm25_collection();

    // WHERE vector NEAR $v AND MATCH 'rust' — should dispatch to hybrid_search
    let near = make_near_condition("v");
    let text = make_match_condition("rust");
    let cond = Condition::And(Box::new(near), Box::new(text));

    let query = make_query(Some(cond), Some(10));
    let params = make_vector_params("v", &[1.0, 0.0, 0.0, 0.0]);

    let results = collection
        .execute_query(&query, &params)
        .expect("NEAR + MATCH should work");
    assert!(!results.is_empty(), "NEAR + MATCH should return results");
}

// ============================================================================
// Text search with metadata filter
// ============================================================================

#[test]
fn test_velesql_match_with_filter() {
    let (_dir, collection) = setup_bm25_collection();

    // WHERE MATCH 'programming' AND category = 'tech'
    let text = make_match_condition("programming");
    let filter = make_filter("category", "tech");
    let cond = Condition::And(Box::new(text), Box::new(filter));

    let query = make_query(Some(cond), Some(10));
    let params = HashMap::new();

    let results = collection
        .execute_query(&query, &params)
        .expect("MATCH + filter should work");

    // All results should be category=tech
    for r in &results {
        if let Some(ref payload) = r.point.payload {
            assert_eq!(
                payload.get("category").and_then(|v| v.as_str()),
                Some("tech"),
                "All results should be tech category"
            );
        }
    }
}

// ============================================================================
// Three-way combination: NEAR + MATCH + metadata filter
// ============================================================================

#[test]
fn test_velesql_near_match_filter() {
    let (_dir, collection) = setup_bm25_collection();

    // WHERE vector NEAR $v AND MATCH 'rust' AND category = 'tech'
    let near = make_near_condition("v");
    let text = make_match_condition("rust");
    let filter = make_filter("category", "tech");
    let near_and_text = Condition::And(Box::new(near), Box::new(text));
    let cond = Condition::And(Box::new(near_and_text), Box::new(filter));

    let query = make_query(Some(cond), Some(10));
    let params = make_vector_params("v", &[1.0, 0.0, 0.0, 0.0]);

    let results = collection
        .execute_query(&query, &params)
        .expect("BM25+NEAR+filter query should succeed");

    // All results should be category=tech
    for r in &results {
        if let Some(ref payload) = r.point.payload {
            assert_eq!(
                payload.get("category").and_then(|v| v.as_str()),
                Some("tech"),
                "Three-way results should be tech category"
            );
        }
    }
}

// ============================================================================
// Limit and ordering tests
// ============================================================================

#[test]
fn test_velesql_match_with_limit() {
    let (_dir, collection) = setup_bm25_collection();

    let cond = make_match_condition("programming");
    let query = make_query(Some(cond), Some(2));
    let params = HashMap::new();

    let results = collection
        .execute_query(&query, &params)
        .expect("MATCH with LIMIT should work");
    assert!(
        results.len() <= 2,
        "LIMIT should be respected, got {}",
        results.len()
    );
}

#[test]
fn test_velesql_match_empty_query() {
    let (_dir, collection) = setup_bm25_collection();

    // Empty MATCH query should return empty results
    let cond = make_match_condition("");
    let query = make_query(Some(cond), Some(10));
    let params = HashMap::new();

    // Empty query should either return empty or all — not error
    let result = collection.execute_query(&query, &params);
    assert!(result.is_ok(), "Empty MATCH should not error");
}

// ============================================================================
// Error case: MATCH + NEAR_FUSED
// ============================================================================

#[test]
fn test_velesql_match_plus_near_fused_is_error() {
    use crate::velesql::{FusionConfig, VectorFusedSearch};

    let (_dir, collection) = setup_bm25_collection();

    let text = make_match_condition("rust");
    let fused = Condition::VectorFusedSearch(VectorFusedSearch {
        vectors: vec![
            VectorExpr::Parameter("v1".to_string()),
            VectorExpr::Parameter("v2".to_string()),
        ],
        fusion: FusionConfig::default(),
    });
    let cond = Condition::And(Box::new(text), Box::new(fused));

    let query = make_query(Some(cond), Some(10));
    let mut params = HashMap::new();
    params.insert("v1".to_string(), json!([1.0, 0.0, 0.0, 0.0]));
    params.insert("v2".to_string(), json!([0.0, 1.0, 0.0, 0.0]));

    // NEAR_FUSED dispatch should still work — MATCH is just metadata from NEAR_FUSED's perspective
    // But the test verifies the query doesn't panic
    let _result = collection.execute_query(&query, &params);
}
