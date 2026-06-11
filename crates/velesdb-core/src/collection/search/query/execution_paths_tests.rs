//! Tests for the SELECT `ExecutionStrategy::Parallel` path:
//! `dispatch_vector_with_strategy` union semantics and the
//! `merge_select_parallel_results` best-score merge.

#![cfg(all(test, feature = "persistence"))]

use super::merge_select_parallel_results;
use crate::collection::search::query::QuerySearchOptions;
use crate::point::{Point, SearchResult};
use crate::test_fixtures::fixtures::{make_point_with_payload, setup_collection};
use crate::velesql::ExecutionStrategy;

fn make_result(id: u64, score: f32) -> SearchResult {
    SearchResult::new(Point::without_payload(id, vec![0.0; 4]), score)
}

fn ids_of(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

// ============================================================================
// A. merge_select_parallel_results unit semantics
// ============================================================================

/// Union: ids present in only one branch survive; duplicated ids keep the
/// BETTER score (higher wins when `higher_is_better`), sorted descending.
#[test]
fn test_merge_parallel_union_keeps_best_score_higher_is_better() {
    let graph = vec![make_result(1, 0.4), make_result(2, 0.9)];
    let vector = vec![make_result(2, 0.5), make_result(3, 0.7)];

    let merged = merge_select_parallel_results(graph, vector, true, 10);

    assert_eq!(ids_of(&merged), vec![2, 3, 1], "descending best-score order");
    let id2 = merged.iter().find(|r| r.point.id == 2).expect("id 2");
    assert!((id2.score - 0.9).abs() < f32::EPSILON, "best score must win");
}

/// Lower-is-better polarity: the LOWER score wins per id and the merged
/// list sorts ascending.
#[test]
fn test_merge_parallel_union_keeps_best_score_lower_is_better() {
    let graph = vec![make_result(1, 0.8), make_result(2, 0.2)];
    let vector = vec![make_result(1, 0.3), make_result(3, 0.5)];

    let merged = merge_select_parallel_results(graph, vector, false, 10);

    assert_eq!(ids_of(&merged), vec![2, 1, 3], "ascending best-score order");
    let id1 = merged.iter().find(|r| r.point.id == 1).expect("id 1");
    assert!((id1.score - 0.3).abs() < f32::EPSILON, "lower score must win");
}

/// The merged union is truncated to `limit` after sorting.
#[test]
fn test_merge_parallel_truncates_to_limit() {
    let graph = vec![make_result(1, 0.9), make_result(2, 0.8)];
    let vector = vec![make_result(3, 0.7), make_result(4, 0.6)];

    let merged = merge_select_parallel_results(graph, vector, true, 2);

    assert_eq!(ids_of(&merged), vec![1, 2], "top-2 of the union only");
}

// ============================================================================
// B. Forcing ExecutionStrategy::Parallel through dispatch
// ============================================================================

/// GIVEN a filtered collection
/// WHEN dispatching with a FORCED `ExecutionStrategy::Parallel`
/// THEN the result is exactly the best-score union of the GraphFirst scan
///      and the VectorFirst HNSW branch (same merge the planner promises).
#[test]
fn test_parallel_strategy_returns_best_score_union_of_both_branches() {
    let (_dir, col) = setup_collection(4);
    let points: Vec<Point> = (1..=20u64)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = vec![1.0, i as f32 * 0.05, 0.0, 0.0];
            make_point_with_payload(i, v, serde_json::json!({"category": "tech"}))
        })
        .collect();
    col.upsert(points).expect("test: upsert");

    let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "category".into(),
        value: serde_json::json!("tech"),
    });
    let query = [1.0, 0.0, 0.0, 0.0];
    let opts = QuerySearchOptions::default();
    let (k, limit) = (10, 5);

    let parallel = col
        .dispatch_vector_with_strategy(&query, &filter, ExecutionStrategy::Parallel, k, limit, &opts)
        .expect("test: parallel dispatch");

    // Reference union: run both branches exactly as the Parallel arm does.
    let graph = col.scan_and_score_by_vector(&filter, &query, limit);
    let vector = col
        .search_with_filter_and_opts(&query, k, &filter, &opts)
        .expect("test: vector branch");
    let expected = merge_select_parallel_results(graph, vector, true, limit);

    assert_eq!(parallel.len(), limit, "LIMIT must be respected");
    assert_eq!(
        ids_of(&parallel),
        ids_of(&expected),
        "Parallel must equal the best-score union of GraphFirst + VectorFirst"
    );
    // Cosine: most similar to [1,0,0,0] is the smallest second component.
    assert_eq!(parallel[0].point.id, 1, "best-scored point first");
    for w in parallel.windows(2) {
        assert!(w[0].score >= w[1].score, "descending similarity order");
    }
}
