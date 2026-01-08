//! Tests for `backend_adapter` module

use super::backend_adapter::*;
use super::distance::SimdDistance;
use super::graph::NativeHnsw;
use crate::distance::DistanceMetric;
use tempfile::tempdir;

// =========================================================================
// TDD Tests: NativeNeighbour
// =========================================================================

#[test]
fn test_native_neighbour_creation() {
    let n = NativeNeighbour::new(42, 0.5);
    assert_eq!(n.d_id, 42);
    assert!((n.distance - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_native_neighbour_equality() {
    let n1 = NativeNeighbour::new(1, 0.1);
    let n2 = NativeNeighbour::new(1, 0.1);
    let n3 = NativeNeighbour::new(2, 0.1);

    assert_eq!(n1, n2);
    assert_ne!(n1, n3);
}

// =========================================================================
// TDD Tests: parallel_insert
// =========================================================================

#[test]
fn test_parallel_insert_small_batch() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    let vectors: Vec<Vec<f32>> = (0..10).map(|i| vec![i as f32; 32]).collect();
    let data: Vec<(&Vec<f32>, usize)> = vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();

    hnsw.parallel_insert(&data);

    assert_eq!(hnsw.len(), 10);
}

#[test]
fn test_parallel_insert_large_batch() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 200);

    let vectors: Vec<Vec<f32>> = (0..150).map(|i| vec![i as f32 * 0.01; 32]).collect();
    let data: Vec<(&Vec<f32>, usize)> = vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();

    hnsw.parallel_insert(&data);

    assert_eq!(hnsw.len(), 150);
}

// =========================================================================
// TDD Tests: search_neighbours
// =========================================================================

#[test]
fn test_search_neighbours_format() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    for i in 0..50 {
        hnsw.insert(vec![i as f32 * 0.1; 32]);
    }

    let query = vec![0.0; 32];
    let results = hnsw.search_neighbours(&query, 5, 50);

    assert!(results.len() <= 5);
    for result in &results {
        assert!(result.d_id < 50);
        assert!(result.distance >= 0.0);
    }
}

// =========================================================================
// TDD Tests: transform_score
// =========================================================================

#[test]
fn test_transform_score_euclidean() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    assert!((hnsw.transform_score(0.5) - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_transform_score_cosine() {
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Cosine: similarity = 1 - distance
    assert!((hnsw.transform_score(0.3) - 0.7).abs() < f32::EPSILON);
    assert!((hnsw.transform_score(1.5) - 0.0).abs() < f32::EPSILON); // clamped
}

#[test]
fn test_transform_score_dot_product() {
    let engine = SimdDistance::new(DistanceMetric::DotProduct);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // DotProduct: score = -distance
    assert!((hnsw.transform_score(0.5) - (-0.5)).abs() < f32::EPSILON);
}

// =========================================================================
// TDD Tests: file_dump and file_load
// =========================================================================

#[test]
fn test_file_dump_creates_files() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    for i in 0..20 {
        hnsw.insert(vec![i as f32; 32]);
    }

    let dir = tempdir().unwrap();
    let result = hnsw.file_dump(dir.path(), "test_index");

    assert!(result.is_ok());
    assert!(dir.path().join("test_index.vectors").exists());
    assert!(dir.path().join("test_index.graph").exists());
}

#[test]
fn test_file_dump_and_load_roundtrip() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert some vectors
    let vectors: Vec<Vec<f32>> = (0..30)
        .map(|i| (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect())
        .collect();

    for v in &vectors {
        hnsw.insert(v.clone());
    }

    // Dump to files
    let dir = tempdir().unwrap();
    hnsw.file_dump(dir.path(), "roundtrip").unwrap();

    // Load from files
    let engine2 = SimdDistance::new(DistanceMetric::Euclidean);
    let loaded = NativeHnsw::file_load(dir.path(), "roundtrip", engine2).unwrap();

    // Verify loaded index
    assert_eq!(loaded.len(), 30);

    // Search should return same results
    let query = vectors[0].clone();
    let results_orig = hnsw.search(&query, 5, 50);
    let results_loaded = loaded.search(&query, 5, 50);

    assert_eq!(results_orig.len(), results_loaded.len());
    // First result should be the same (exact match)
    if !results_orig.is_empty() && !results_loaded.is_empty() {
        assert_eq!(results_orig[0].0, results_loaded[0].0);
    }
}

// =========================================================================
// TDD Tests: set_searching_mode (no-op but should not panic)
// =========================================================================

#[test]
fn test_set_searching_mode_no_panic() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = NativeHnsw::new(engine, 16, 100, 100);

    hnsw.set_searching_mode(true);
    hnsw.set_searching_mode(false);
    // Should not panic
}

// =========================================================================
// TDD Tests: NativeHnswBackend trait
// =========================================================================

#[test]
fn test_native_backend_trait_is_object_safe() {
    // Verify trait can be used as dyn object
    fn accepts_dyn_backend(_backend: &dyn NativeHnswBackend) {}

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);
    accepts_dyn_backend(&hnsw);
}

#[test]
fn test_native_backend_trait_search() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert via trait
    for i in 0..20 {
        let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
        <NativeHnsw<SimdDistance> as NativeHnswBackend>::insert(&hnsw, (&vec, i));
    }

    // Search via trait
    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
    let results = <NativeHnsw<SimdDistance> as NativeHnswBackend>::search(&hnsw, &query, 5, 50);

    assert!(!results.is_empty());
    assert!(results.len() <= 5);
}

#[test]
fn test_native_backend_generic_function() {
    // Test that trait can be used in generic context
    fn search_with_backend<B: NativeHnswBackend>(
        backend: &B,
        query: &[f32],
        k: usize,
    ) -> Vec<NativeNeighbour> {
        backend.search(query, k, 100)
    }

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    for i in 0..10 {
        hnsw.insert(vec![i as f32; 32]);
    }

    let query = vec![0.0; 32];
    let results = search_with_backend(&hnsw, &query, 5);

    assert!(!results.is_empty());
}

#[test]
fn test_native_backend_len_and_is_empty() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    assert!(<NativeHnsw<SimdDistance> as NativeHnswBackend>::is_empty(
        &hnsw
    ));
    assert_eq!(
        <NativeHnsw<SimdDistance> as NativeHnswBackend>::len(&hnsw),
        0
    );

    hnsw.insert(vec![1.0; 32]);

    assert!(!<NativeHnsw<SimdDistance> as NativeHnswBackend>::is_empty(
        &hnsw
    ));
    assert_eq!(
        <NativeHnsw<SimdDistance> as NativeHnswBackend>::len(&hnsw),
        1
    );
}
