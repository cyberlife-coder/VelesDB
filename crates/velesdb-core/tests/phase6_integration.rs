//! E2E integration tests for Phase 6: Unified Query & Full-Text Search.
//!
//! Exercises all new query paths introduced in Phase 6:
//! - VP-012: `NEAR_FUSED` multi-vector fused search
//! - VP-011: BM25 + NEAR hybrid search via `VelesQL`
//! - VP-011: MATCH + metadata filter via `VelesQL`
//! - VP-011: Three-way (NEAR + MATCH + filter) via `VelesQL`

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::distance::DistanceMetric;
use velesdb_core::fusion::FusionStrategy;
use velesdb_core::point::SearchResult;
use velesdb_core::{Database, Point};

/// Setup a collection with diverse data for comprehensive testing.
fn setup_phase6_collection() -> (tempfile::TempDir, velesdb_core::Collection) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .expect("create");
    let col = db.get_collection("docs").expect("get");

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "title": "Rust systems programming guide",
                "category": "tech", "score": 95
            })),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "title": "Python machine learning tutorial",
                "category": "tech", "score": 85
            })),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "title": "Rust async programming patterns",
                "category": "tech", "score": 90
            })),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({
                "title": "Cooking seasonal vegetables",
                "category": "food", "score": 70
            })),
        ),
        Point::new(
            5,
            vec![0.7, 0.7, 0.0, 0.0],
            Some(json!({
                "title": "Advanced Rust memory management",
                "category": "tech", "score": 92
            })),
        ),
        Point::new(
            6,
            vec![0.0, 0.7, 0.7, 0.0],
            Some(json!({
                "title": "Data science with Python notebooks",
                "category": "science", "score": 78
            })),
        ),
    ];

    col.upsert(points).expect("upsert");
    (dir, col)
}

// ============================================================================
// VP-012: NEAR_FUSED multi-vector fused search
// ============================================================================

#[test]
fn e2e_near_fused_rrf_returns_results() {
    let (_dir, col) = setup_phase6_collection();

    let q1 = vec![1.0, 0.0, 0.0, 0.0];
    let q2 = vec![0.0, 1.0, 0.0, 0.0];
    let vecs: Vec<&[f32]> = vec![&q1, &q2];

    let results = col
        .multi_query_search(&vecs, 5, FusionStrategy::RRF { k: 60 }, None)
        .expect("NEAR_FUSED RRF should work");

    assert!(!results.is_empty(), "Should return results");
    assert!(results.len() <= 5, "Should respect limit");
}

#[test]
fn e2e_near_fused_average_returns_results() {
    let (_dir, col) = setup_phase6_collection();

    let q1 = vec![1.0, 0.0, 0.0, 0.0];
    let q2 = vec![0.0, 0.0, 1.0, 0.0];
    let vecs: Vec<&[f32]> = vec![&q1, &q2];

    let results = col
        .multi_query_search(&vecs, 5, FusionStrategy::Average, None)
        .expect("NEAR_FUSED Average should work");

    assert!(!results.is_empty());
}

#[test]
fn e2e_near_fused_with_metadata_filter() {
    let (_dir, col) = setup_phase6_collection();

    let q1 = vec![1.0, 0.0, 0.0, 0.0];
    let q2 = vec![0.0, 1.0, 0.0, 0.0];
    let vecs: Vec<&[f32]> = vec![&q1, &q2];

    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
        field: "category".to_string(),
        value: json!("tech"),
    });

    let results = col
        .multi_query_search(&vecs, 5, FusionStrategy::RRF { k: 60 }, Some(&filter))
        .expect("NEAR_FUSED + filter should work");

    for r in &results {
        if let Some(ref payload) = r.point.payload {
            assert_eq!(
                payload.get("category").and_then(|v| v.as_str()),
                Some("tech"),
                "All results should be tech"
            );
        }
    }
}

// ============================================================================
// VP-011: BM25 text search via VelesQL execute_query()
// ============================================================================

fn make_velesql_query(
    where_clause: Option<velesdb_core::velesql::Condition>,
    limit: Option<u64>,
) -> velesdb_core::velesql::Query {
    velesdb_core::velesql::Query {
        select: velesdb_core::velesql::SelectStatement {
            distinct: velesdb_core::velesql::DistinctMode::None,
            columns: velesdb_core::velesql::SelectColumns::All,
            from: "docs".to_string(),
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

#[test]
fn e2e_velesql_text_search_only() {
    let (_dir, col) = setup_phase6_collection();

    let cond = velesdb_core::velesql::Condition::Match(velesdb_core::velesql::MatchCondition {
        column: "title".to_string(),
        query: "rust".to_string(),
    });
    let query = make_velesql_query(Some(cond), Some(10));
    let params = HashMap::new();

    let results = col.execute_query(&query, &params).expect("text search");
    assert!(!results.is_empty(), "MATCH 'rust' should find results");
}

#[test]
fn e2e_velesql_hybrid_near_and_match() {
    let (_dir, col) = setup_phase6_collection();

    let near =
        velesdb_core::velesql::Condition::VectorSearch(velesdb_core::velesql::VectorSearch {
            vector: velesdb_core::velesql::VectorExpr::Parameter("v".to_string()),
        });
    let text = velesdb_core::velesql::Condition::Match(velesdb_core::velesql::MatchCondition {
        column: "title".to_string(),
        query: "rust".to_string(),
    });
    let cond = velesdb_core::velesql::Condition::And(Box::new(near), Box::new(text));

    let query = make_velesql_query(Some(cond), Some(10));
    let mut params = HashMap::new();
    params.insert("v".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = col.execute_query(&query, &params).expect("hybrid search");
    assert!(!results.is_empty(), "NEAR + MATCH should return results");
}

#[test]
fn e2e_velesql_match_with_metadata_filter() {
    let (_dir, col) = setup_phase6_collection();

    let text = velesdb_core::velesql::Condition::Match(velesdb_core::velesql::MatchCondition {
        column: "title".to_string(),
        query: "programming".to_string(),
    });
    let filter = velesdb_core::velesql::Condition::Comparison(velesdb_core::velesql::Comparison {
        column: "category".to_string(),
        operator: velesdb_core::velesql::CompareOp::Eq,
        value: velesdb_core::velesql::Value::String("tech".to_string()),
    });
    let cond = velesdb_core::velesql::Condition::And(Box::new(text), Box::new(filter));

    let query = make_velesql_query(Some(cond), Some(10));
    let params = HashMap::new();

    let results = col.execute_query(&query, &params).expect("MATCH + filter");

    for r in &results {
        if let Some(ref payload) = r.point.payload {
            assert_eq!(
                payload.get("category").and_then(|v| v.as_str()),
                Some("tech"),
            );
        }
    }
}

#[test]
fn e2e_velesql_three_way_near_match_filter() {
    let (_dir, col) = setup_phase6_collection();

    let near =
        velesdb_core::velesql::Condition::VectorSearch(velesdb_core::velesql::VectorSearch {
            vector: velesdb_core::velesql::VectorExpr::Parameter("v".to_string()),
        });
    let text = velesdb_core::velesql::Condition::Match(velesdb_core::velesql::MatchCondition {
        column: "title".to_string(),
        query: "rust".to_string(),
    });
    let filter = velesdb_core::velesql::Condition::Comparison(velesdb_core::velesql::Comparison {
        column: "category".to_string(),
        operator: velesdb_core::velesql::CompareOp::Eq,
        value: velesdb_core::velesql::Value::String("tech".to_string()),
    });

    let near_and_text = velesdb_core::velesql::Condition::And(Box::new(near), Box::new(text));
    let cond = velesdb_core::velesql::Condition::And(Box::new(near_and_text), Box::new(filter));

    let query = make_velesql_query(Some(cond), Some(10));
    let mut params = HashMap::new();
    params.insert("v".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = col
        .execute_query(&query, &params)
        .expect("three-way dispatch");

    for r in &results {
        if let Some(ref payload) = r.point.payload {
            assert_eq!(
                payload.get("category").and_then(|v| v.as_str()),
                Some("tech"),
                "Three-way results must be category=tech"
            );
        }
    }
}

// ============================================================================
// Limit and no-panic validation
// ============================================================================

#[test]
fn e2e_all_paths_respect_limit() {
    let (_dir, col) = setup_phase6_collection();

    // Pure text
    let cond = velesdb_core::velesql::Condition::Match(velesdb_core::velesql::MatchCondition {
        column: "title".to_string(),
        query: "programming".to_string(),
    });
    let query = make_velesql_query(Some(cond), Some(2));
    let results = col
        .execute_query(&query, &HashMap::new())
        .expect("text limit");
    assert!(results.len() <= 2, "Limit 2 respected for text search");

    // NEAR_FUSED
    let q1 = vec![1.0, 0.0, 0.0, 0.0];
    let q2 = vec![0.0, 1.0, 0.0, 0.0];
    let vecs: Vec<&[f32]> = vec![&q1, &q2];
    let results = col
        .multi_query_search(&vecs, 2, FusionStrategy::RRF { k: 60 }, None)
        .expect("fused limit");
    assert!(results.len() <= 2, "Limit 2 respected for NEAR_FUSED");
}

#[test]
fn e2e_empty_text_query_no_panic() {
    let (_dir, col) = setup_phase6_collection();

    let cond = velesdb_core::velesql::Condition::Match(velesdb_core::velesql::MatchCondition {
        column: "title".to_string(),
        query: String::new(),
    });
    let query = make_velesql_query(Some(cond), Some(10));
    // Should not panic
    let _ = col.execute_query(&query, &HashMap::new());
}

/// Helper to verify results are non-empty and have valid scores.
fn _assert_valid_results(results: &[SearchResult], ctx: &str) {
    assert!(!results.is_empty(), "{ctx}: should return results");
    for r in results {
        assert!(r.score.is_finite(), "{ctx}: score should be finite");
    }
}
