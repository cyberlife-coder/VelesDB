//! Tests for public sparse and hybrid dense+sparse search API methods.

#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::fusion::FusionStrategy;
use crate::index::sparse::SparseVector;
use crate::point::Point;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Helper: create a collection with both dense and sparse vectors.
fn setup_sparse_collection() -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
        .expect("create collection");

    let mut points = Vec::new();
    for i in 0u64..8 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        let dense = vec![fi / 8.0, 0.5, 0.3, 0.1];
        let sparse = {
            let mut map = BTreeMap::new();
            #[allow(clippy::cast_precision_loss)]
            let w = 1.0 + i as f32;
            map.insert(
                String::new(), // default sparse index
                SparseVector::new(vec![(10, w), (20, 0.5)]),
            );
            Some(map)
        };
        points.push(Point {
            id: i,
            vector: dense,
            payload: Some(serde_json::json!({ "idx": i })),
            sparse_vectors: sparse,
        });
    }
    col.upsert(points).expect("upsert");
    (dir, col)
}

// -----------------------------------------------------------------------
// Sparse-only search via public API
// -----------------------------------------------------------------------

#[test]
fn test_sparse_search_default_returns_scored_results() {
    let (_dir, col) = setup_sparse_collection();

    let query = SparseVector::new(vec![(10, 1.0), (20, 1.0)]);
    let results = col.sparse_search_default(&query, 5).expect("sparse search");

    assert!(!results.is_empty(), "should find sparse results");
    assert!(results.len() <= 5, "should respect k limit");

    // Results should be sorted descending by score.
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "scores must be descending: {} < {}",
            window[0].score,
            window[1].score
        );
    }
}

#[test]
fn test_sparse_search_named_missing_index_errors() {
    let (_dir, col) = setup_sparse_collection();

    let query = SparseVector::new(vec![(10, 1.0)]);
    let result = col.sparse_search_named(&query, 5, "nonexistent_index");
    assert!(result.is_err(), "missing sparse index should error");
}

// -----------------------------------------------------------------------
// Hybrid dense + sparse via public API
// -----------------------------------------------------------------------

#[test]
fn test_hybrid_sparse_search_fuses_both_branches() {
    let (_dir, col) = setup_sparse_collection();

    // Dense query close to point 7 (fi=7, dense=[0.875,0.5,0.3,0.1]).
    let dense_query = vec![0.9, 0.5, 0.3, 0.1];
    let sparse_query = SparseVector::new(vec![(10, 1.0), (20, 1.0)]);
    let strategy = FusionStrategy::rrf_default();

    let results = col
        .hybrid_sparse_search(&dense_query, &sparse_query, 5, &strategy)
        .expect("hybrid sparse search");

    assert!(!results.is_empty(), "hybrid should return results");

    // Point 7 has both high dense proximity and highest sparse weight.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        ids.contains(&7),
        "point 7 should appear in hybrid results (dense + sparse hit)"
    );
}

#[test]
fn test_hybrid_sparse_search_empty_sparse_falls_back_to_dense() {
    let (_dir, col) = setup_sparse_collection();

    let dense_query = vec![0.9, 0.5, 0.3, 0.1];
    // Term that no document has -> empty sparse branch.
    let sparse_query = SparseVector::new(vec![(99999, 1.0)]);
    let strategy = FusionStrategy::rrf_default();

    let results = col
        .hybrid_sparse_search(&dense_query, &sparse_query, 5, &strategy)
        .expect("hybrid with empty sparse");

    assert!(
        !results.is_empty(),
        "should fall back to dense results when sparse is empty"
    );
}
