//! Tests for `VectorCollection` search methods.

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;

use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::point::Point;
use crate::quantization::StorageMode;
use crate::VectorCollection;

/// Creates a 4-dim `VectorCollection` backed by a temporary directory.
fn create_test_vc() -> (VectorCollection, TempDir) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_coll");
    let coll = VectorCollection::create(path, "test", 4, DistanceMetric::Cosine, StorageMode::Full)
        .unwrap();
    (coll, dir)
}

/// Inserts five orthogonal-ish points with text payloads.
fn seed_points(coll: &VectorCollection) {
    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"text": "hello world", "cat": "a"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"text": "foo bar", "cat": "b"})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"text": "hello foo", "cat": "a"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"text": "baz qux", "cat": "b"})),
        ),
        Point::new(
            5,
            vec![1.0, 1.0, 0.0, 0.0],
            Some(json!({"text": "hello bar", "cat": "a"})),
        ),
    ];
    coll.upsert(points).unwrap();
}

// ─── search() ────────────────────────────────────────────────────────────────

#[test]
fn test_search_returns_k_nearest() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll.search(&[1.0, 0.0, 0.0, 0.0], 3).unwrap();

    assert_eq!(results.len(), 3);
    // The exact match (id=1) should be the closest result.
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_search_empty_collection() {
    let (coll, _dir) = create_test_vc();

    let results = coll.search(&[1.0, 0.0, 0.0, 0.0], 5).unwrap();

    assert!(results.is_empty());
}

#[test]
fn test_search_k_exceeds_collection_size() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll.search(&[1.0, 0.0, 0.0, 0.0], 100).unwrap();

    // Cannot return more points than exist in the collection.
    assert!(results.len() <= 5);
}

#[test]
fn test_search_results_ordered_by_similarity() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll.search(&[1.0, 0.0, 0.0, 0.0], 5).unwrap();

    // Cosine similarity: higher score = more similar, sorted descending.
    for pair in results.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "Results should be sorted by descending similarity"
        );
    }
}

// ─── text_search() ──────────────────────────────────────────────────────────

#[test]
fn test_text_search_returns_matching_docs() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll.text_search("hello", 10).unwrap();

    // Points 1, 3, and 5 contain "hello" in their text payload.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1), "id=1 should match 'hello'");
    assert!(ids.contains(&3), "id=3 should match 'hello'");
    assert!(ids.contains(&5), "id=5 should match 'hello'");
}

#[test]
fn test_text_search_no_match() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll.text_search("nonexistent_term_xyz", 10).unwrap();

    assert!(results.is_empty());
}

// ─── search_with_ef() ───────────────────────────────────────────────────────

#[test]
fn test_search_with_ef_returns_results() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll.search_with_ef(&[1.0, 0.0, 0.0, 0.0], 3, 200).unwrap();

    assert!(!results.is_empty());
    assert!(results.len() <= 3);
    // Exact match should still be top-1 with higher ef.
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_search_with_ef_empty_collection() {
    let (coll, _dir) = create_test_vc();

    let results = coll.search_with_ef(&[0.5, 0.5, 0.0, 0.0], 5, 50).unwrap();

    assert!(results.is_empty());
}

// ─── search_with_filter() ───────────────────────────────────────────────────

#[test]
fn test_search_with_filter_equality() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let filter = Filter::new(Condition::eq("cat", "a"));
    let results = coll
        .search_with_filter(&[1.0, 0.0, 0.0, 0.0], 10, &filter)
        .unwrap();

    // Only points with cat="a" (ids 1, 3, 5) should be returned.
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("cat"))
            .and_then(|v| v.as_str());
        assert_eq!(cat, Some("a"), "filter should only admit cat=a");
    }
}

#[test]
fn test_search_with_filter_no_match() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let filter = Filter::new(Condition::eq("cat", "nonexistent"));
    let results = coll
        .search_with_filter(&[1.0, 0.0, 0.0, 0.0], 10, &filter)
        .unwrap();

    assert!(results.is_empty());
}

// ─── search_ids() ───────────────────────────────────────────────────────────

#[test]
fn test_search_ids_returns_id_score_pairs() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll.search_ids(&[1.0, 0.0, 0.0, 0.0], 3).unwrap();

    assert_eq!(results.len(), 3);
    // Exact match (id=1) should be closest.
    assert_eq!(results[0].id, 1);
    // Scores should be non-negative for cosine distance.
    for r in &results {
        assert!(r.score >= 0.0, "cosine scores should be non-negative");
    }
}

#[test]
fn test_search_ids_empty_collection() {
    let (coll, _dir) = create_test_vc();

    let results = coll.search_ids(&[1.0, 0.0, 0.0, 0.0], 5).unwrap();

    assert!(results.is_empty());
}

// ─── hybrid_search() ────────────────────────────────────────────────────────

#[test]
fn test_hybrid_search_combines_vector_and_text() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let results = coll
        .hybrid_search(&[1.0, 0.0, 0.0, 0.0], "hello", 5, None)
        .unwrap();

    // Point id=1 is both the vector nearest neighbor AND contains "hello",
    // so it should rank at or near the top.
    assert!(!results.is_empty());
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1), "id=1 should appear in hybrid results");
}

#[test]
fn test_hybrid_search_with_alpha() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    // alpha=1.0 should heavily weight vector similarity.
    let results = coll
        .hybrid_search(&[1.0, 0.0, 0.0, 0.0], "hello", 3, Some(1.0))
        .unwrap();

    assert!(!results.is_empty());
}

#[test]
fn test_hybrid_search_empty_collection() {
    let (coll, _dir) = create_test_vc();

    let results = coll
        .hybrid_search(&[1.0, 0.0, 0.0, 0.0], "hello", 5, None)
        .unwrap();

    assert!(results.is_empty());
}

// ─── execute_query() ────────────────────────────────────────────────────────

#[test]
fn test_execute_query_select_all_with_limit() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let query = crate::velesql::Parser::parse("SELECT * FROM coll LIMIT 3;").unwrap();
    let params = HashMap::new();
    let results = coll.execute_query(&query, &params).unwrap();

    assert!(results.len() <= 3);
}

#[test]
fn test_execute_query_with_where_clause() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let query =
        crate::velesql::Parser::parse("SELECT * FROM coll WHERE cat = 'a' LIMIT 10;").unwrap();
    let params = HashMap::new();
    let results = coll.execute_query(&query, &params).unwrap();

    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("cat"))
            .and_then(|v| v.as_str());
        assert_eq!(cat, Some("a"));
    }
}

// ─── execute_query_str() ────────────────────────────────────────────────────

#[test]
fn test_execute_query_str_basic() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let params = HashMap::new();
    let results = coll
        .execute_query_str("SELECT * FROM coll LIMIT 5;", &params)
        .unwrap();

    assert!(results.len() <= 5);
}

#[test]
fn test_execute_query_str_invalid_sql_returns_error() {
    let (coll, _dir) = create_test_vc();
    let params = HashMap::new();

    let result = coll.execute_query_str("THIS IS NOT SQL", &params);

    assert!(result.is_err(), "invalid SQL should produce an error");
}

#[test]
fn test_execute_query_str_with_filter() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let params = HashMap::new();
    let results = coll
        .execute_query_str("SELECT * FROM coll WHERE cat = 'b' LIMIT 10;", &params)
        .unwrap();

    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("cat"))
            .and_then(|v| v.as_str());
        assert_eq!(cat, Some("b"));
    }
}

#[test]
fn test_execute_query_str_returns_consistent_results() {
    let (coll, _dir) = create_test_vc();
    seed_points(&coll);

    let params = HashMap::new();
    let sql = "SELECT * FROM coll LIMIT 3;";

    let r1 = coll.execute_query_str(sql, &params).unwrap();
    let r2 = coll.execute_query_str(sql, &params).unwrap();

    assert_eq!(
        r1.len(),
        r2.len(),
        "repeated queries should return the same count"
    );
}
