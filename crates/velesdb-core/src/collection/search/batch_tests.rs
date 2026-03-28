//! Tests for batch and multi-query search methods.

#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::Collection;
use crate::filter::{Condition, Filter};
use crate::test_fixtures::fixtures::{make_point_with_payload, setup_collection};

/// Helper: create a collection with 6 test points spanning the unit hypercube.
fn setup_batch_collection() -> (tempfile::TempDir, Collection) {
    let points = vec![
        make_point_with_payload(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            serde_json::json!({ "category": "tech" }),
        ),
        make_point_with_payload(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            serde_json::json!({ "category": "sports" }),
        ),
        make_point_with_payload(
            3,
            vec![0.7, 0.7, 0.0, 0.0],
            serde_json::json!({ "category": "tech" }),
        ),
        make_point_with_payload(
            4,
            vec![0.0, 0.0, 1.0, 0.0],
            serde_json::json!({ "category": "music" }),
        ),
        make_point_with_payload(
            5,
            vec![0.5, 0.5, 0.5, 0.5],
            serde_json::json!({ "category": "tech" }),
        ),
        make_point_with_payload(
            6,
            vec![0.9, 0.1, 0.0, 0.0],
            serde_json::json!({ "category": "sports" }),
        ),
    ];
    let (dir, col) = setup_collection(4);
    col.upsert(points).expect("test: upsert");
    (dir, col)
}

// -----------------------------------------------------------------------
// search_batch_parallel
// -----------------------------------------------------------------------

#[test]
fn test_batch_parallel_two_queries_returns_per_query_results() {
    let (_dir, col) = setup_batch_collection();

    let q1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
    let q2: Vec<f32> = vec![0.0, 1.0, 0.0, 0.0];
    let queries: Vec<&[f32]> = vec![&q1, &q2];

    let results = col
        .search_batch_parallel(&queries, 3)
        .expect("batch search");

    assert_eq!(results.len(), 2, "one result set per query");
    assert!(!results[0].is_empty(), "first query should have results");
    assert!(!results[1].is_empty(), "second query should have results");

    // First query is close to point 1 ([1,0,0,0]).
    assert_eq!(results[0][0].point.id, 1, "q1 best match should be id=1");
    // Second query is close to point 2 ([0,1,0,0]).
    assert_eq!(results[1][0].point.id, 2, "q2 best match should be id=2");
}

// -----------------------------------------------------------------------
// search_batch_with_filters — per-query filters
// -----------------------------------------------------------------------

#[test]
fn test_batch_with_filters_applies_distinct_filter_per_query() {
    let (_dir, col) = setup_batch_collection();

    let q1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
    let q2: Vec<f32> = vec![0.0, 1.0, 0.0, 0.0];
    let queries: Vec<&[f32]> = vec![&q1, &q2];

    let filter_tech = Some(Filter::new(Condition::eq("category", "tech")));
    let filter_sports = Some(Filter::new(Condition::eq("category", "sports")));
    let filters = vec![filter_tech, filter_sports];

    let results = col
        .search_batch_with_filters(&queries, 5, &filters)
        .expect("batch with filters");

    assert_eq!(results.len(), 2);

    // All q1 results should be category=tech.
    for r in &results[0] {
        let cat = r.point.payload.as_ref().and_then(|p| p.get("category"));
        assert_eq!(cat.and_then(|v| v.as_str()), Some("tech"));
    }
    // All q2 results should be category=sports.
    for r in &results[1] {
        let cat = r.point.payload.as_ref().and_then(|p| p.get("category"));
        assert_eq!(cat.and_then(|v| v.as_str()), Some("sports"));
    }
}

// -----------------------------------------------------------------------
// search_batch_with_filters — mismatched lengths
// -----------------------------------------------------------------------

#[test]
fn test_batch_with_filters_rejects_mismatched_lengths() {
    let (_dir, col) = setup_batch_collection();

    let q1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
    let queries: Vec<&[f32]> = vec![&q1];
    let filters = vec![None, None]; // 2 filters for 1 query

    let result = col.search_batch_with_filters(&queries, 5, &filters);
    assert!(result.is_err(), "should reject mismatched lengths");
}

// -----------------------------------------------------------------------
// Batch search on empty collection
// -----------------------------------------------------------------------

#[test]
fn test_batch_parallel_empty_collection_returns_empty_vecs() {
    let (_dir, col) = setup_collection(4);

    let q: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
    let queries: Vec<&[f32]> = vec![&q];

    let results = col
        .search_batch_parallel(&queries, 5)
        .expect("batch search on empty");

    assert_eq!(results.len(), 1);
    assert!(
        results[0].is_empty(),
        "empty collection should return no hits"
    );
}

// -----------------------------------------------------------------------
// Dimension mismatch
// -----------------------------------------------------------------------

#[test]
fn test_batch_parallel_rejects_wrong_dimension() {
    let (_dir, col) = setup_batch_collection();

    let bad_q: Vec<f32> = vec![1.0, 0.0]; // dim=2, expected=4
    let queries: Vec<&[f32]> = vec![&bad_q];

    let result = col.search_batch_parallel(&queries, 3);
    assert!(result.is_err(), "should reject wrong dimension");
}

// -----------------------------------------------------------------------
// Per-query top_k: separate calls with different k values (#419)
// -----------------------------------------------------------------------

#[test]
fn test_batch_with_filters_respects_per_call_k() {
    let (_dir, col) = setup_batch_collection();

    let q1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
    let q2: Vec<f32> = vec![0.0, 1.0, 0.0, 0.0];

    // Call with k=2: both queries truncated to 2 results.
    let results_k2 = col
        .search_batch_with_filters(&[&q1, &q2], 2, &[None, None])
        .expect("batch k=2");
    assert!(results_k2[0].len() <= 2, "q1 should have at most 2 results");
    assert!(results_k2[1].len() <= 2, "q2 should have at most 2 results");

    // Call with k=5: both queries can return up to 5 (collection has 6 points).
    let results_k5 = col
        .search_batch_with_filters(&[&q1, &q2], 5, &[None, None])
        .expect("batch k=5");
    assert!(
        results_k5[0].len() > results_k2[0].len(),
        "k=5 should return more results than k=2"
    );
}
