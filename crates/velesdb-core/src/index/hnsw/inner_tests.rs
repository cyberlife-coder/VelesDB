//! Tests for `inner` module

use super::inner::*;
use hnsw_rs::prelude::*;

/// Test search works for all distance metrics
#[test]
fn test_hnsw_inner_search_all_metrics() {
    let indices = [
        HnswInner::Cosine(Hnsw::new(16, 100, 16, 4, DistCosine)),
        HnswInner::Euclidean(Hnsw::new(16, 100, 16, 4, DistL2)),
        HnswInner::DotProduct(Hnsw::new(16, 100, 16, 4, DistDot)),
    ];

    for index in &indices {
        let query = vec![0.5_f32; 4];
        let results = index.search(&query, 3, 32);
        assert!(results.is_empty());
    }
}

/// Test insert works for `HnswInner`
#[test]
fn test_hnsw_inner_insert() {
    let index = HnswInner::Cosine(Hnsw::new(16, 100, 16, 4, DistCosine));
    let vector = vec![0.1_f32; 4];
    index.insert((&vector, 0));
    let results = index.search(&vector, 1, 32);
    assert_eq!(results.len(), 1);
}

/// Test `transform_score` for different metrics
#[test]
fn test_hnsw_inner_transform_score() {
    let cosine = HnswInner::Cosine(Hnsw::new(16, 100, 16, 4, DistCosine));
    let euclidean = HnswInner::Euclidean(Hnsw::new(16, 100, 16, 4, DistL2));
    let dot = HnswInner::DotProduct(Hnsw::new(16, 100, 16, 4, DistDot));

    assert!((cosine.transform_score(0.5) - 0.5).abs() < 0.001);
    assert!((euclidean.transform_score(0.5) - 0.5).abs() < 0.001);
    assert!((dot.transform_score(0.5) - (-0.5)).abs() < 0.001);
}
