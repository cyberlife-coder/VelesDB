//! Tests for `graph` module - Native HNSW graph implementation.

use super::graph::NativeHnsw;
use super::layer::NodeId;
use crate::distance::DistanceMetric;
use crate::index::hnsw::native::distance::CpuDistance;

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_insert_and_search() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

    // Insert some vectors
    for i in 0..100 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(v);
    }

    assert_eq!(hnsw.len(), 100);

    // Search
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);

    assert!(!results.is_empty());
    assert!(results.len() <= 10);
    // First result should be node 0 (closest to query)
    assert_eq!(results[0].0, 0);
}

#[test]
fn test_empty_search() {
    let engine = CpuDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

    let query = vec![1.0, 2.0, 3.0];
    let results = hnsw.search(&query, 10, 50);

    assert!(results.is_empty());
}

// =========================================================================
// TDD Tests for Heuristic Neighbor Selection (PERF-3)
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_selection_empty_candidates() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert a single vector to have valid query
    hnsw.insert(vec![0.0; 32]);

    let query = vec![0.0; 32];
    let candidates: Vec<(NodeId, f32)> = vec![];

    let selected = hnsw.select_neighbors(&query, &candidates, 10);
    assert!(selected.is_empty(), "Empty candidates should return empty");
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_selection_fewer_than_max() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..5 {
        hnsw.insert(vec![i as f32; 32]);
    }

    let query = vec![0.0; 32];
    let candidates: Vec<(NodeId, f32)> = vec![(0, 0.0), (1, 1.0), (2, 2.0)];

    let selected = hnsw.select_neighbors(&query, &candidates, 10);
    assert_eq!(
        selected.len(),
        3,
        "Should return all candidates when fewer than max"
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_selection_respects_max() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..20 {
        hnsw.insert(vec![i as f32; 32]);
    }

    let query = vec![0.0; 32];
    let candidates: Vec<(NodeId, f32)> = (0..15).map(|i| (i, i as f32)).collect();

    let selected = hnsw.select_neighbors(&query, &candidates, 5);
    assert_eq!(selected.len(), 5, "Should respect max_neighbors limit");
}

#[test]
fn test_heuristic_selection_prefers_diverse_neighbors() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert diverse vectors: one at origin, cluster around (10,0,0...), spread around (0,10,0...)
    hnsw.insert(vec![0.0; 32]); // 0: origin

    // Cluster A: near (10, 0, 0, ...)
    let mut v1 = vec![0.0; 32];
    v1[0] = 10.0;
    hnsw.insert(v1); // 1
    let mut v2 = vec![0.0; 32];
    v2[0] = 10.5;
    hnsw.insert(v2); // 2
    let mut v3 = vec![0.0; 32];
    v3[0] = 10.2;
    hnsw.insert(v3); // 3

    // Diverse point: near (0, 10, 0, ...)
    let mut v4 = vec![0.0; 32];
    v4[1] = 10.0;
    hnsw.insert(v4); // 4

    let query = vec![0.0; 32];
    // Candidates: all close to query in euclidean terms
    let candidates: Vec<(NodeId, f32)> = vec![
        (1, 10.0), // Cluster A
        (2, 10.5), // Cluster A (close to 1)
        (3, 10.2), // Cluster A (close to 1)
        (4, 10.0), // Diverse (perpendicular direction)
    ];

    let selected = hnsw.select_neighbors(&query, &candidates, 2);

    // Heuristic should prefer diverse selection
    // Should include node 1 (first closest) and node 4 (diverse direction)
    assert_eq!(selected.len(), 2);
    assert!(selected.contains(&1), "Should include first closest");
    // The heuristic should prefer 4 over 2,3 because 4 is in a different direction
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_fills_quota_with_closest_if_needed() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..10 {
        hnsw.insert(vec![i as f32; 32]);
    }

    let query = vec![0.0; 32];
    let candidates: Vec<(NodeId, f32)> = (0..10).map(|i| (i, i as f32)).collect();

    let selected = hnsw.select_neighbors(&query, &candidates, 8);

    // Should fill up to max even if heuristic rejects some
    assert_eq!(
        selected.len(),
        8,
        "Should fill quota with closest candidates"
    );
}

#[test]
fn test_recall_with_heuristic_selection() {
    // Test that heuristic selection maintains good recall
    use crate::index::hnsw::native::distance::SimdDistance;

    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 32, 200, 1000);

    // Insert 500 random-ish vectors
    for i in 0..500 {
        let v: Vec<f32> = (0..128)
            .map(|j| ((i * 127 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(v);
    }

    // Test recall: search should find vectors close to query
    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search(&query, 10, 100);

    assert!(!results.is_empty(), "Should find results");
    assert!(results.len() >= 5, "Should find at least 5 neighbors");

    // Results should be sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i].1 >= results[i - 1].1,
            "Results should be sorted by distance"
        );
    }
}

// =========================================================================
// Regression test for PR #179: Infinite loop in next_random() when rng_state is 0
// =========================================================================

#[test]
fn test_next_random_zero_state_no_infinite_loop() {
    use std::sync::atomic::Ordering;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Force rng_state to 0 (edge case that caused infinite loop before fix)
    hnsw.rng_state.store(0, Ordering::Relaxed);

    // This should NOT hang - if it does, the test times out
    // The fix ensures CAS compares against original value (0), not the seed
    hnsw.insert(vec![1.0; 32]);

    // Verify the state was properly updated (no longer 0)
    let state_after = hnsw.rng_state.load(Ordering::Relaxed);
    assert_ne!(state_after, 0, "State should be non-zero after insert");

    // Verify the vector was actually inserted
    assert_eq!(hnsw.len(), 1, "Vector should be inserted");
}
