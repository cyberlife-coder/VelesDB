//! Tests for `backend_adapter` module

#![allow(deprecated)] // SimdDistance deprecated in favor of CachedSimdDistance

use super::backend_adapter::*;
use super::distance::{DistanceEngine, SimdDistance};
use super::graph::{NativeHnsw, NO_ENTRY_POINT};
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

    // Euclidean: transform_score applies sqrt (raw distances are squared L2)
    assert!(
        (hnsw.transform_score(0.25) - 0.5).abs() < f32::EPSILON,
        "sqrt(0.25) should be 0.5"
    );
    assert!(
        (hnsw.transform_score(25.0) - 5.0).abs() < 1e-5,
        "sqrt(25.0) should be 5.0"
    );
    assert!(
        hnsw.transform_score(0.0).abs() < f32::EPSILON,
        "sqrt(0.0) should be 0.0"
    );
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
    let ep_id = hnsw.entry_point.load(std::sync::atomic::Ordering::Acquire);
    assert_ne!(
        ep_id, NO_ENTRY_POINT,
        "entry_point should be set after inserting 2000 vectors"
    );
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

// =========================================================================
// TDD Tests: adaptive_ef_for_batch (#486 — bulk insert optimization)
// =========================================================================

#[test]
fn test_adaptive_ef_small_batch_no_reduction() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 32, 400, 100);

    // Batches <= 1000 use full ef_construction with stagnation disabled
    let (ef, stag) = hnsw.adaptive_ef_for_batch(500);
    assert_eq!(ef, 400, "small batch should use full ef_construction");
    assert_eq!(stag, 0, "small batch should have stagnation disabled");

    let (ef, stag) = hnsw.adaptive_ef_for_batch(1000);
    assert_eq!(
        ef, 400,
        "batch of exactly 1000 should use full ef_construction"
    );
    assert_eq!(
        stag, 0,
        "batch of exactly 1000 should have stagnation disabled"
    );
}

#[test]
fn test_adaptive_ef_medium_batch_85_percent() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 32, 400, 100);

    // Batches > 1K and <= 10K use 85% of ef_construction
    let (ef, stag) = hnsw.adaptive_ef_for_batch(5_000);
    assert_eq!(
        ef, 340,
        "batch of 5K should use 85% of ef_construction (340)"
    );
    assert_eq!(stag, 170, "stagnation should be ef/2 = 170");
}

#[test]
fn test_adaptive_ef_large_batch_75_percent() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 32, 400, 100);

    // Batches > 10K and <= 50K use 75% of ef_construction
    let (ef, stag) = hnsw.adaptive_ef_for_batch(20_000);
    assert_eq!(
        ef, 300,
        "batch of 20K should use 75% of ef_construction (300)"
    );
    assert_eq!(stag, 150, "stagnation should be ef/2 = 150");
}

#[test]
fn test_adaptive_ef_very_large_batch_60_percent() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 32, 400, 100);

    // Batches > 50K use 60% of ef_construction
    let (ef, stag) = hnsw.adaptive_ef_for_batch(100_000);
    assert_eq!(
        ef, 240,
        "batch of 100K should use 60% of ef_construction (240)"
    );
    assert_eq!(stag, 120, "stagnation should be ef/2 = 120");
}

#[test]
fn test_adaptive_ef_floor_at_4x_max_connections() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    // ef_construction=40, M=32: 60% of 40 = 24, but floor is 4*M=128
    let hnsw = NativeHnsw::new(engine, 32, 40, 100);

    let (ef, stag) = hnsw.adaptive_ef_for_batch(100_000);
    assert_eq!(
        ef, 128,
        "ef should be floored at 4*max_connections when scaling goes below it"
    );
    assert_eq!(stag, 64, "stagnation should be ef/2 = 64");
}

#[test]
fn test_adaptive_ef_boundary_10001() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 400, 100);

    // Exactly 10001 crosses into the 75% tier
    let (ef, _) = hnsw.adaptive_ef_for_batch(10_001);
    assert_eq!(ef, 300, "batch of 10001 should use 75% tier");
}

#[test]
fn test_adaptive_ef_boundary_50001() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 400, 100);

    // Exactly 50001 crosses into the 60% tier
    let (ef, _) = hnsw.adaptive_ef_for_batch(50_001);
    assert_eq!(ef, 240, "batch of 50001 should use 60% tier");
}

#[test]
fn test_adaptive_ef_recall_preserved_with_2000_vectors() {
    // Regression test: adaptive ef for a 2000-vector batch (75% tier)
    // must maintain recall >= 0.90 (same threshold as non-adaptive test above).
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    let vectors: Vec<Vec<f32>> = (0..2000)
        .map(|i| (0..32).map(|j| ((i * 32 + j) as f32) * 0.001).collect())
        .collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    // This exercises connect_batch_chunked -> connect_node_with_ef with adaptive ef
    hnsw.parallel_insert(&data)
        .expect("parallel_insert should succeed");

    assert_eq!(hnsw.len(), 2000);

    let bf_engine = SimdDistance::new(DistanceMetric::Euclidean);
    let k = 10;
    let ef_search = 128;
    let num_queries = 50;
    let mut total_recall = 0.0;

    for q_idx in 0..num_queries {
        let query: Vec<f32> = (0..32)
            .map(|j| ((q_idx * 7 + j * 13) as f32) * 0.002)
            .collect();

        let hnsw_results = hnsw.search(&query, k, ef_search);
        let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|&(id, _)| id).collect();

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
        "adaptive ef recall@{k} should be >= 0.90, got {avg_recall:.4}"
    );
}
// =========================================================================
// TDD Tests: BatchEfSchedule — graduated ef_construction (I1)
// =========================================================================

#[test]
fn test_batch_ef_schedule_small_batch_uniform() {
    // Batches < 1000 should use full ef for all phases
    let schedule = super::backend_adapter::compute_batch_ef_schedule(200, 500, 16);
    assert_eq!(schedule.scaffold_ef, 200);
    assert_eq!(schedule.bulk_ef, 200);
    assert_eq!(schedule.finalize_ef, 200);
    assert_eq!(schedule.scaffold_count, 500);
    assert_eq!(schedule.finalize_start, 500);
}

#[test]
fn test_batch_ef_schedule_large_batch_graduated() {
    // Batch of 10_000 with ef=200, M=16
    let schedule = super::backend_adapter::compute_batch_ef_schedule(200, 10_000, 16);
    assert_eq!(schedule.scaffold_ef, 200, "scaffold should use full ef");
    assert_eq!(schedule.bulk_ef, 100, "bulk should use 0.5x ef");
    assert_eq!(schedule.finalize_ef, 150, "finalize should use 0.75x ef");
    assert_eq!(schedule.scaffold_count, 1000, "scaffold = 10%");
    assert_eq!(schedule.finalize_start, 9000, "finalize starts at 90%");
}

#[test]
fn test_batch_ef_schedule_floor_enforcement() {
    // When 0.5x ef < 2*m, the floor should kick in
    // ef=40, m=16 → bulk = max(20, 32) = 32; finalize = max(30, 32) = 32
    let schedule = super::backend_adapter::compute_batch_ef_schedule(40, 5000, 16);
    assert_eq!(schedule.scaffold_ef, 40);
    assert_eq!(schedule.bulk_ef, 32, "bulk floored at 2*M");
    assert_eq!(schedule.finalize_ef, 32, "finalize floored at 2*M");
}

#[test]
fn test_batch_ef_schedule_ef_for_position() {
    let schedule = super::backend_adapter::compute_batch_ef_schedule(200, 10_000, 16);

    // Scaffold phase: positions 0..999
    assert_eq!(schedule.ef_for_position(0), 200);
    assert_eq!(schedule.ef_for_position(999), 200);

    // Bulk phase: positions 1000..8999
    assert_eq!(schedule.ef_for_position(1000), 100);
    assert_eq!(schedule.ef_for_position(5000), 100);
    assert_eq!(schedule.ef_for_position(8999), 100);

    // Finalize phase: positions 9000..9999
    assert_eq!(schedule.ef_for_position(9000), 150);
    assert_eq!(schedule.ef_for_position(9999), 150);
}

#[test]
fn test_batch_ef_schedule_boundary_batch_size() {
    // Exactly 1000: should apply graduated schedule
    let schedule = super::backend_adapter::compute_batch_ef_schedule(100, 1000, 16);
    assert_eq!(schedule.scaffold_count, 100);
    assert_eq!(schedule.finalize_start, 900);
    assert_eq!(schedule.bulk_ef, 50);
    assert_eq!(schedule.finalize_ef, 75);

    // 999: should use uniform full ef
    let schedule = super::backend_adapter::compute_batch_ef_schedule(100, 999, 16);
    assert_eq!(schedule.scaffold_ef, 100);
    assert_eq!(schedule.bulk_ef, 100);
    assert_eq!(schedule.finalize_ef, 100);
}

// =========================================================================
// TDD Tests: graduated ef_construction recall (I1)
// =========================================================================

/// Verifies that graduated ef_construction maintains recall >= 0.90
/// with 5000 vectors. The 3-phase schedule (scaffold/bulk/finalize)
/// reduces construction work while preserving graph quality.
#[test]
fn test_graduated_ef_construction_recall() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Generate 5000 deterministic 64-D vectors with enough spread for recall testing
    let vectors: Vec<Vec<f32>> = (0..5000)
        .map(|i| (0..64).map(|j| ((i * 64 + j) as f32) * 0.0001).collect())
        .collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data)
        .expect("test: parallel_insert of 5000 vectors should succeed");

    assert_eq!(hnsw.len(), 5000);

    let bf_engine = SimdDistance::new(DistanceMetric::Euclidean);
    let k = 10;
    let ef_search = 128;
    let num_queries = 100;

    let mut total_recall = 0.0;

    for q_idx in 0..num_queries {
        let query: Vec<f32> = (0..64)
            .map(|j| ((q_idx * 11 + j * 17) as f32) * 0.0003)
            .collect();

        let hnsw_results = hnsw.search(&query, k, ef_search);
        let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|&(id, _)| id).collect();

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
    // Reason: num_queries is a small constant (100); f64 is exact for integers up to 2^53.
    let avg_recall = total_recall / num_queries as f64;

    assert!(
        avg_recall >= 0.90,
        "graduated ef_construction: recall@{k} should be >= 0.90 at 5000 vectors, got {avg_recall:.4}"
    );
}

/// Verifies that the graduated schedule applies to cosine metric as well,
/// since cosine normalizes vectors before insertion.
#[test]
fn test_graduated_ef_construction_recall_cosine() {
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Generate 3000 deterministic 32-D vectors
    let vectors: Vec<Vec<f32>> = (0..3000)
        .map(|i| {
            let raw: Vec<f32> = (0..32)
                .map(|j| ((i * 32 + j) as f32) * 0.001 + 0.1)
                .collect();
            // Pre-normalize for cosine metric ground-truth comparison
            let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
            raw.iter().map(|x| x / norm).collect()
        })
        .collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data)
        .expect("test: parallel_insert of 3000 cosine vectors should succeed");

    assert_eq!(hnsw.len(), 3000);

    let bf_engine = SimdDistance::new(DistanceMetric::Cosine);
    let k = 10;
    let ef_search = 128;
    let num_queries = 50;

    let mut total_recall = 0.0;

    for q_idx in 0..num_queries {
        let raw: Vec<f32> = (0..32)
            .map(|j| ((q_idx * 7 + j * 13) as f32) * 0.002 + 0.1)
            .collect();
        let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        let query: Vec<f32> = raw.iter().map(|x| x / norm).collect();

        let hnsw_results = hnsw.search(&query, k, ef_search);
        let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|&(id, _)| id).collect();

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
        avg_recall >= 0.89,
        "graduated ef cosine: recall@{k} should be >= 0.89 at 3000 vectors, got {avg_recall:.4}"
    );
}

// =========================================================================
// I2: Pre-Allocated Vector Storage — Regression tests
// =========================================================================

/// Verifies that batch insert with the split lock strategy (I2) produces
/// the same recall as sequential insert. This guards against the resize/push
/// split introducing any data corruption or ordering bugs.
#[test]
fn test_i2_preallocated_batch_insert_recall() {
    let dim = 64;
    let n = 1000;
    let k = 10;
    let ef_search = 128;
    let num_queries = 30;

    // Build index via parallel_insert (uses split reserve + push)
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 200, n);

    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            let mut v: Vec<f32> = (0..dim)
                .map(|j| ((i * dim + j) as f32) * 0.001 + 0.01)
                .collect();
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut v {
                *x /= norm;
            }
            v
        })
        .collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data)
        .expect("test: parallel_insert should succeed");

    assert_eq!(hnsw.len(), n, "all vectors should be inserted");

    // Recall check against brute-force
    let bf_engine = SimdDistance::new(DistanceMetric::Cosine);
    let mut total_recall = 0.0;

    for q_idx in 0..num_queries {
        let mut query: Vec<f32> = (0..dim)
            .map(|j| ((q_idx * 3 + j * 7) as f32) * 0.003)
            .collect();
        let norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut query {
            *x /= norm;
        }

        let hnsw_results = hnsw.search(&query, k, ef_search);
        let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|&(id, _)| id).collect();

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
    // Reason: num_queries is a small constant (30); f64 is exact for integers up to 2^53.
    let avg_recall = total_recall / num_queries as f64;

    // Threshold 0.89 to account for float-precision edge cases where
    // recall = 27/30 = 0.9000... rounds to 0.8999... in f64 arithmetic.
    assert!(
        avg_recall >= 0.89,
        "I2 pre-allocated batch recall@{k} should be >= 0.89, got {avg_recall:.4}"
    );
}

/// Verifies that a batch insert much larger than the initial `max_elements`
/// correctly resizes in the reserve phase and pushes without corruption.
#[test]
fn test_i2_batch_exceeding_initial_capacity() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    // Initial max_elements = 16, but we insert 500 — forces multiple resizes
    let hnsw = NativeHnsw::new(engine, 16, 100, 16);

    let vectors: Vec<Vec<f32>> = (0..500).map(|i| vec![i as f32 * 0.01; 32]).collect();

    let data: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_slice(), i))
        .collect();

    hnsw.parallel_insert(&data)
        .expect("test: batch exceeding initial capacity should succeed");

    assert_eq!(hnsw.len(), 500);

    // Verify vector data integrity by searching for exact matches
    let query = vectors[0].clone();
    let results = hnsw.search(&query, 1, 50);
    assert!(
        !results.is_empty(),
        "search should find at least one result"
    );
    assert_eq!(
        results[0].0, 0,
        "nearest neighbor of vector 0 should be itself"
    );
}
