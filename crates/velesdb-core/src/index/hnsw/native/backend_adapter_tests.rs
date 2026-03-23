//! Tests for `backend_adapter` module

#![allow(deprecated)] // SimdDistance deprecated in favor of CachedSimdDistance

use super::backend_adapter::*;
use super::distance::{DistanceEngine, SimdDistance};
use super::graph::NativeHnsw;
use crate::distance::DistanceMetric;
use crate::metrics::recall_at_k;
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
    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data).expect("test");

    assert_eq!(hnsw.len(), 10);
}

#[test]
fn test_parallel_insert_large_batch() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Use 50 vectors to stay under Rayon parallelization threshold (100)
    // This avoids deadlocks when tests run in parallel
    let vectors: Vec<Vec<f32>> = (0..50).map(|i| vec![i as f32 * 0.01; 32]).collect();
    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data).expect("test");

    assert_eq!(hnsw.len(), 50);
}

// =========================================================================
// TDD Tests: search_neighbours
// =========================================================================

#[test]
fn test_search_neighbours_format() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    for i in 0..50 {
        hnsw.insert(&[i as f32 * 0.1; 32]).expect("test");
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
        hnsw.insert(&[i as f32; 32]).expect("test");
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
        hnsw.insert(v).expect("test");
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
        <NativeHnsw<SimdDistance> as NativeHnswBackend>::insert(&hnsw, (&vec, i)).expect("test");
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
        hnsw.insert(&[i as f32; 32]).expect("test");
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

    hnsw.insert(&[1.0; 32]).expect("test");

    assert!(!<NativeHnsw<SimdDistance> as NativeHnswBackend>::is_empty(
        &hnsw
    ));
    assert_eq!(
        <NativeHnsw<SimdDistance> as NativeHnswBackend>::len(&hnsw),
        1
    );
}

// =========================================================================
// TDD Tests: chunked Phase B for large batch insert (#364 — RED)
// =========================================================================

#[test]
fn test_compute_chunk_size_boundaries() {
    // Formula: (batch_len / 50).max(1000).min(5000)
    assert_eq!(NativeHnsw::<SimdDistance>::compute_chunk_size(100), 1000);
    assert_eq!(NativeHnsw::<SimdDistance>::compute_chunk_size(1_000), 1000);
    assert_eq!(NativeHnsw::<SimdDistance>::compute_chunk_size(10_000), 1000);
    assert_eq!(
        NativeHnsw::<SimdDistance>::compute_chunk_size(100_000),
        2000
    );
    assert_eq!(
        NativeHnsw::<SimdDistance>::compute_chunk_size(500_000),
        5000
    );
}

#[test]
fn test_parallel_insert_chunked_count() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Generate 2000 deterministic 32-D vectors using index-based values
    let vectors: Vec<Vec<f32>> = (0..2000)
        .map(|i| (0..32).map(|j| ((i * 32 + j) as f32) * 0.001).collect())
        .collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data)
        .expect("parallel_insert of 2000 vectors should succeed");

    assert_eq!(hnsw.len(), 2000);
}

#[test]
fn test_parallel_insert_chunked_ep_update() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Generate 2000 deterministic 32-D vectors
    let vectors: Vec<Vec<f32>> = (0..2000)
        .map(|i| (0..32).map(|j| ((i * 32 + j) as f32) * 0.001).collect())
        .collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data)
        .expect("parallel_insert of 2000 vectors should succeed");

    // With 2000 nodes and deterministic PRNG (fixed seed 0x5DEE_CE66_D1A4_B5B5),
    // node 0 is never assigned the highest layer. The entry point must have been
    // promoted to a higher-layer node during chunked insertion.
    let ep = *hnsw.entry_point.read();
    let ep_id = ep.expect("entry_point should be Some after inserting 2000 vectors");
    assert_ne!(
        ep_id, 0,
        "entry point should have been promoted beyond node 0 with 2000 inserts"
    );
}

#[test]
fn test_parallel_insert_chunked_recall() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Generate 2000 deterministic 32-D vectors with enough spread for recall testing
    let vectors: Vec<Vec<f32>> = (0..2000)
        .map(|i| (0..32).map(|j| ((i * 32 + j) as f32) * 0.001).collect())
        .collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data)
        .expect("parallel_insert of 2000 vectors should succeed");

    // Brute-force distance engine (same metric as the index)
    let bf_engine = SimdDistance::new(DistanceMetric::Euclidean);
    let k = 10;
    let ef_search = 128;
    let num_queries = 50;

    let mut total_recall = 0.0;

    for q_idx in 0..num_queries {
        // Deterministic query vector derived from query index
        let query: Vec<f32> = (0..32)
            .map(|j| ((q_idx * 7 + j * 13) as f32) * 0.002)
            .collect();

        // HNSW search
        let hnsw_results = hnsw.search(&query, k, ef_search);
        let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|&(id, _)| id).collect();

        // Brute-force ground truth: compute distance to every vector, sort, take top-k
        let mut distances: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(id, v)| (id, bf_engine.distance(&query, v)))
            .collect();
        distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let ground_truth: Vec<usize> = distances.iter().take(k).map(|&(id, _)| id).collect();

        total_recall += recall_at_k(&ground_truth, &hnsw_ids);
    }

    #[allow(clippy::cast_precision_loss)]
    // Reason: num_queries is a small constant (50); f64 is exact for integers up to 2^53.
    let avg_recall = total_recall / num_queries as f64;

    assert!(
        avg_recall >= 0.90,
        "average recall@{k} should be >= 0.90, got {avg_recall:.4}"
    );
}
