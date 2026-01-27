//! Tests for `native_inner` module - Native HNSW inner implementation.

use super::native_inner::*;
use crate::distance::DistanceMetric;
use tempfile::tempdir;

#[test]
fn test_native_inner_new_all_metrics() {
    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ] {
        let inner = NativeHnswInner::new(metric, 16, 100, 100);
        assert_eq!(inner.metric(), metric);
        assert!(inner.is_empty());
    }
}

#[test]
fn test_native_inner_insert_and_search() {
    let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);

    for i in 0..20 {
        let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
        inner.insert((&vec, i));
    }

    assert_eq!(inner.len(), 20);

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
    let results = inner.search(&query, 5, 50);

    assert!(!results.is_empty());
    assert!(results.len() <= 5);
    assert_eq!(results[0].d_id, 0);
}

#[test]
fn test_native_inner_transform_score_cosine() {
    let inner = NativeHnswInner::new(DistanceMetric::Cosine, 16, 100, 100);
    assert!((inner.transform_score(0.3) - 0.7).abs() < f32::EPSILON);
}

#[test]
fn test_native_inner_transform_score_euclidean() {
    let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
    assert!((inner.transform_score(0.5) - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_native_inner_transform_score_dot_product() {
    let inner = NativeHnswInner::new(DistanceMetric::DotProduct, 16, 100, 100);
    assert!((inner.transform_score(0.5) - (-0.5)).abs() < f32::EPSILON);
}

#[test]
fn test_native_inner_persistence_roundtrip() {
    let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);

    let vectors: Vec<Vec<f32>> = (0..30)
        .map(|i| (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect())
        .collect();

    for (i, v) in vectors.iter().enumerate() {
        inner.insert((v, i));
    }

    let dir = tempdir().unwrap();
    inner.file_dump(dir.path(), "native_test").unwrap();

    let loaded =
        NativeHnswInner::file_load(dir.path(), "native_test", DistanceMetric::Euclidean).unwrap();

    assert_eq!(loaded.len(), 30);
    assert_eq!(loaded.metric(), DistanceMetric::Euclidean);

    let query = vectors[0].clone();
    let results_orig = inner.search(&query, 5, 50);
    let results_loaded = loaded.search(&query, 5, 50);

    assert_eq!(results_orig.len(), results_loaded.len());
    if !results_orig.is_empty() {
        assert_eq!(results_orig[0].d_id, results_loaded[0].d_id);
    }
}

#[test]
fn test_native_inner_parallel_insert() {
    let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 200);

    let vectors: Vec<Vec<f32>> = (0..150).map(|i| vec![i as f32 * 0.01; 32]).collect();
    let data: Vec<(&Vec<f32>, usize)> = vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();

    inner.parallel_insert(&data);

    assert_eq!(inner.len(), 150);
}

#[test]
fn test_native_inner_set_searching_mode_no_panic() {
    let mut inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
    inner.set_searching_mode(true);
    inner.set_searching_mode(false);
}
