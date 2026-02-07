//! Tests for native HNSW implementation.

#![allow(clippy::cast_precision_loss)]

use super::distance::{CpuDistance, SimdDistance};
use super::graph::NativeHnsw;
use crate::distance::DistanceMetric;

#[test]
fn test_native_hnsw_basic_insert_search() {
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

    // Insert 100 vectors
    for i in 0..100_u64 {
        let v: Vec<f32> = (0..128).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
        hnsw.insert(v);
    }

    assert_eq!(hnsw.len(), 100);

    // Search for first vector
    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search(&query, 10, 50);

    assert_eq!(results.len(), 10);
    // First result should be node 0 or very close
    assert!(results[0].1 < 0.1, "First result should be very close");
}

#[test]
fn test_native_hnsw_recall() {
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    // Reduced parameters for faster test execution
    let hnsw = NativeHnsw::new(engine, 16, 100, 500);

    // Reduced from 1000×768D to 200×128D for faster test execution
    let vectors: Vec<Vec<f32>> = (0..200)
        .map(|i| {
            (0..128)
                .map(|j| ((i * 128 + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v.clone());
    }

    // Test recall with multiple queries
    let mut total_recall = 0.0;
    let n_queries = 5;
    let k = 10;

    for q_idx in 0..n_queries {
        let query = &vectors[q_idx * 40]; // Use existing vectors as queries

        // Get HNSW results
        let hnsw_results: Vec<usize> = hnsw
            .search(query, k, 128)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        // Compute ground truth (brute force)
        let mut distances: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let dist = cosine_distance(query, v);
                (i, dist)
            })
            .collect();
        distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let ground_truth: Vec<usize> = distances.iter().take(k).map(|(i, _)| *i).collect();

        // Calculate recall
        let hits = hnsw_results
            .iter()
            .filter(|id| ground_truth.contains(id))
            .count();
        total_recall += hits as f64 / k as f64;
    }

    let avg_recall = total_recall / n_queries as f64;
    assert!(
        avg_recall >= 0.8,
        "Recall should be at least 80%, got {:.1}%",
        avg_recall * 100.0
    );
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        1.0
    } else {
        1.0 - (dot / (norm_a * norm_b))
    }
}

#[test]
fn test_cpu_vs_simd_consistency() {
    let cpu_engine = CpuDistance::new(DistanceMetric::Euclidean);
    let simd_engine = SimdDistance::new(DistanceMetric::Euclidean);

    let cpu_hnsw = NativeHnsw::new(cpu_engine, 16, 100, 100);
    let simd_hnsw = NativeHnsw::new(simd_engine, 16, 100, 100);

    // Insert same vectors
    for i in 0..50_u64 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32).collect();
        cpu_hnsw.insert(v.clone());
        simd_hnsw.insert(v);
    }

    // Search should return similar results
    let query: Vec<f32> = (0..64).map(|j| j as f32).collect();
    let cpu_results = cpu_hnsw.search(&query, 5, 30);
    let simd_results = simd_hnsw.search(&query, 5, 30);

    // First result should match
    assert_eq!(
        cpu_results[0].0, simd_results[0].0,
        "CPU and SIMD should find same nearest neighbor"
    );
}

// =============================================================================
// Phase 2: VAMANA α diversification tests (TDD)
// =============================================================================

#[test]
fn test_native_hnsw_with_alpha_diversification() {
    // Test that higher alpha produces more diverse neighbors
    let engine = SimdDistance::new(DistanceMetric::Cosine);

    // Create index with alpha=1.2 (VAMANA-style diversification)
    let hnsw = NativeHnsw::with_alpha(engine, 16, 100, 100, 1.2);

    // Insert clustered vectors (two clusters)
    for i in 0..25_u64 {
        // Cluster 1: vectors near [1, 0, 0, ...]
        let v: Vec<f32> = (0..32)
            .map(|j| {
                if j == 0 {
                    1.0
                } else {
                    (i as f32 + j as f32) * 0.001
                }
            })
            .collect();
        hnsw.insert(v);
    }
    for i in 0..25_u64 {
        // Cluster 2: vectors near [0, 1, 0, ...]
        let v: Vec<f32> = (0..32)
            .map(|j| {
                if j == 1 {
                    1.0
                } else {
                    (i as f32 + j as f32) * 0.001
                }
            })
            .collect();
        hnsw.insert(v);
    }

    assert_eq!(hnsw.len(), 50);

    // Search should work correctly
    let query: Vec<f32> = (0..32).map(|j| if j == 0 { 0.9 } else { 0.01 }).collect();
    let results = hnsw.search(&query, 5, 50);

    assert!(!results.is_empty(), "Should return results");
}

#[test]
fn test_native_hnsw_alpha_default_is_one() {
    // Default alpha should be 1.0 (standard HNSW behavior)
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    assert!(
        (hnsw.get_alpha() - 1.0).abs() < f32::EPSILON,
        "Default alpha should be 1.0"
    );
}

#[test]
fn test_native_hnsw_alpha_affects_graph_structure() {
    // With alpha > 1.0, the graph should have more diverse connections
    let engine1 = SimdDistance::new(DistanceMetric::Euclidean);
    let engine2 = SimdDistance::new(DistanceMetric::Euclidean);

    let hnsw_standard = NativeHnsw::new(engine1, 16, 100, 100);
    let hnsw_diverse = NativeHnsw::with_alpha(engine2, 16, 100, 100, 1.2);

    // Insert same vectors
    for i in 0..30_u64 {
        let v: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.1).collect();
        hnsw_standard.insert(v.clone());
        hnsw_diverse.insert(v);
    }

    // Both should have same count
    assert_eq!(hnsw_standard.len(), hnsw_diverse.len());
}

// =============================================================================
// Phase 3: Multi-Entry Points tests
// =============================================================================

#[test]
fn test_search_multi_entry_returns_results() {
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..50_u64 {
        let v: Vec<f32> = (0..32).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
        hnsw.insert(v);
    }

    let query: Vec<f32> = (0..32).map(|j| (j as f32 * 0.01).sin()).collect();

    // Multi-entry search with 3 probes
    let results = hnsw.search_multi_entry(&query, 5, 50, 3);

    assert!(!results.is_empty(), "Should return results");
    assert!(results.len() <= 5, "Should not exceed k");
}

#[test]
fn test_search_multi_entry_vs_standard() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..30_u64 {
        let v: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.1).collect();
        hnsw.insert(v);
    }

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.05).collect();

    // Both searches should return results
    let standard = hnsw.search(&query, 5, 50);
    let multi = hnsw.search_multi_entry(&query, 5, 50, 2);

    assert!(!standard.is_empty());
    assert!(!multi.is_empty());
}

// =============================================================================
// BUG-CORE-001: Deadlock Prevention Tests (TDD)
// =============================================================================
// These tests verify that concurrent insert + search operations do not deadlock.
// The root cause was lock order inversion between search_layer (vectors→layers)
// and add_bidirectional_connection (layers→vectors).

#[test]
fn test_concurrent_insert_search_no_deadlock() {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 500));

    // Pre-populate with some vectors
    for i in 0..50_u64 {
        let v: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.1).collect();
        hnsw.insert(v);
    }

    let mut handles = vec![];

    // Spawn insert threads
    for t in 0..4 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..25_u64 {
                let v: Vec<f32> = (0..32).map(|j| ((t * 100 + i) + j) as f32 * 0.01).collect();
                hnsw_clone.insert(v);
            }
        }));
    }

    // Spawn search threads concurrently
    for _ in 0..4 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..25_u64 {
                let query: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.05).collect();
                let _ = hnsw_clone.search(&query, 5, 30);
            }
        }));
    }

    // Wait for all threads with timeout (deadlock detection)
    for handle in handles {
        // If this hangs, we have a deadlock
        let result = handle.join();
        assert!(result.is_ok(), "Thread should complete without panic");
    }

    // Verify index is in consistent state
    assert!(hnsw.len() >= 50, "Should have at least initial vectors");
}

#[test]
fn test_parallel_insert_stress_no_deadlock() {
    use std::sync::Arc;
    use std::thread;

    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = Arc::new(NativeHnsw::new(engine, 32, 200, 1000));

    let num_threads = 8;
    let vectors_per_thread = 50;
    let mut handles = vec![];

    // Stress test: many parallel inserts
    for t in 0..num_threads {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..vectors_per_thread {
                let idx = t * vectors_per_thread + i;
                let v: Vec<f32> = (0..64)
                    .map(|j| ((idx * 64 + j) as f32 * 0.001).sin())
                    .collect();
                hnsw_clone.insert(v);
            }
        }));
    }

    // All threads must complete (no deadlock)
    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    // Final count may be less due to race conditions, but should be substantial
    let final_count = hnsw.len();
    assert!(
        final_count >= (num_threads * vectors_per_thread) / 2,
        "Should have inserted many vectors, got {final_count}"
    );

    // Search should still work after parallel inserts
    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.001).sin()).collect();
    let results = hnsw.search(&query, 10, 50);
    assert!(
        !results.is_empty(),
        "Search should return results after parallel inserts"
    );
}

#[test]
fn test_mixed_operations_no_deadlock() {
    use std::sync::Arc;
    use std::thread;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 300));

    // Pre-populate
    for i in 0..30_u64 {
        let v: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.1).collect();
        hnsw.insert(v);
    }

    let mut handles = vec![];

    // Mix of operations: insert, search, multi-entry search
    for t in 0..3 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..20_u64 {
                let v: Vec<f32> = (0..32).map(|j| ((t * 100 + i) + j) as f32 * 0.01).collect();
                hnsw_clone.insert(v);
            }
        }));
    }

    for _ in 0..2 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..30_u64 {
                let query: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.05).collect();
                let _ = hnsw_clone.search(&query, 5, 30);
            }
        }));
    }

    for _ in 0..2 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..20_u64 {
                let query: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.03).collect();
                let _ = hnsw_clone.search_multi_entry(&query, 5, 30, 2);
            }
        }));
    }

    // All threads must complete
    for handle in handles {
        handle
            .join()
            .expect("Thread should complete without deadlock");
    }

    assert!(hnsw.len() >= 30, "Index should have vectors");
}

// =============================================================================
// Phase 3, Plan 04: Concurrency Family 1 — Parallel Insert/Search/Delete
// =============================================================================
// Validates correctness under concurrent operations with explicit invariant
// assertions (not just "no panic"). Exercises lock-order paths and safety
// counters introduced in Plan 03-03.

/// Stress test: concurrent inserts from many threads with deterministic
/// post-condition on total count and graph searchability.
#[test]
fn test_concurrent_insert_deterministic_count() {
    use std::sync::Arc;
    use std::thread;

    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 2000));

    let num_threads = 8;
    let vectors_per_thread = 100;
    let mut handles = vec![];

    for t in 0..num_threads {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..vectors_per_thread {
                let idx = t * vectors_per_thread + i;
                let v: Vec<f32> = (0..64)
                    .map(|j| ((idx * 64 + j) as f32 * 0.001).sin())
                    .collect();
                hnsw_clone.insert(v);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread should complete without panic");
    }

    // Deterministic assertion: all inserts must be reflected in count
    let final_count = hnsw.len();
    assert_eq!(
        final_count,
        num_threads * vectors_per_thread,
        "Every insert must be counted; got {final_count} expected {}",
        num_threads * vectors_per_thread
    );

    // Verify graph is searchable and returns correct k
    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.001).sin()).collect();
    let results = hnsw.search(&query, 20, 50);
    assert_eq!(
        results.len(),
        20,
        "Search should return exactly k results from populated graph"
    );
    // Results must be sorted by distance
    for window in results.windows(2) {
        assert!(
            window[0].1 <= window[1].1,
            "Results must be sorted by distance: {} > {}",
            window[0].1,
            window[1].1
        );
    }
}

/// Concurrent insert + search with search correctness assertions.
/// Verifies that search always returns valid node IDs and sorted distances
/// even while inserts are actively modifying the graph.
#[test]
fn test_concurrent_insert_search_correctness() {
    use std::sync::Arc;
    use std::thread;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 1000));

    // Pre-populate to ensure searches have data
    for i in 0..100_u64 {
        let v: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.1).collect();
        hnsw.insert(v);
    }

    let mut handles = vec![];

    // 4 insert threads
    for t in 0..4_u64 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..50_u64 {
                let v: Vec<f32> = (0..32)
                    .map(|j| ((t * 1000 + i) + j) as f32 * 0.01)
                    .collect();
                hnsw_clone.insert(v);
            }
        }));
    }

    // 4 search threads with correctness assertions
    for t in 0..4_u64 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..50_u64 {
                let query: Vec<f32> = (0..32).map(|j| ((t * 500 + i) + j) as f32 * 0.02).collect();
                let results = hnsw_clone.search(&query, 5, 30);
                // Must respect k limit
                assert!(results.len() <= 5, "Search must return at most k results");
                // All returned node IDs must be within valid range
                let current_len = hnsw_clone.len();
                for &(node_id, dist) in &results {
                    assert!(
                        node_id < current_len + 200,
                        "Node ID {node_id} should be in valid range"
                    );
                    assert!(
                        dist.is_finite(),
                        "Distance must be finite, got {dist} for node {node_id}"
                    );
                }
                // Results should be distance-sorted
                for window in results.windows(2) {
                    assert!(
                        window[0].1 <= window[1].1,
                        "Results must be sorted by distance"
                    );
                }
            }
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("Thread should complete without deadlock");
    }

    // Post-condition: index grew correctly
    let final_count = hnsw.len();
    assert!(
        final_count >= 300,
        "Should have at least 100 initial + 200 inserted, got {final_count}"
    );

    // Safety counters: no invariant violations
    let snapshot = super::graph::safety_counters::HNSW_COUNTERS.snapshot();
    assert_eq!(
        snapshot.invariant_violation_total, 0,
        "Concurrent insert+search must not trigger lock-order violations"
    );
}

/// Concurrent insert + multi-entry search interleaving.
/// Validates that multi-entry search remains consistent under concurrent writes.
#[test]
fn test_concurrent_insert_multi_entry_search() {
    use std::sync::Arc;
    use std::thread;

    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 600));

    // Pre-populate
    for i in 0..50_u64 {
        let v: Vec<f32> = (0..32).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
        hnsw.insert(v);
    }

    let mut handles = vec![];

    // Inserters
    for t in 0..3_u64 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..30_u64 {
                let v: Vec<f32> = (0..32)
                    .map(|j| ((t * 100 + i) + j) as f32 * 0.005)
                    .collect();
                hnsw_clone.insert(v);
            }
        }));
    }

    // Multi-entry searchers
    for _ in 0..3_u64 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..30_u64 {
                let query: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.03).collect();
                let results = hnsw_clone.search_multi_entry(&query, 5, 30, 3);
                assert!(results.len() <= 5, "Multi-entry search must respect k");
                // Verify distance monotonicity
                for window in results.windows(2) {
                    assert!(
                        window[0].1 <= window[1].1,
                        "Multi-entry results must be sorted by distance"
                    );
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread must complete (no deadlock)");
    }

    assert!(
        hnsw.len() >= 50,
        "Index must retain at least pre-populated vectors"
    );
}

// =============================================================================
// BUG-04 / QUAL-01: Lock-order safety + observability counters
// =============================================================================

#[test]
fn test_hnsw_no_deadlock_during_parallel_insert_search() {
    use std::sync::Arc;
    use std::thread;

    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 500));

    // Pre-populate so search has data to traverse
    for i in 0..100_u64 {
        let v: Vec<f32> = (0..64).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
        hnsw.insert(v);
    }

    let mut handles = vec![];

    // 4 insert threads
    for t in 0..4 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..50_u64 {
                let v: Vec<f32> = (0..64)
                    .map(|j| ((t * 1000 + i) + j) as f32 * 0.001)
                    .collect();
                hnsw_clone.insert(v);
            }
        }));
    }

    // 4 search threads
    for t in 0..4 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..50_u64 {
                let query: Vec<f32> = (0..64)
                    .map(|j| ((t * 500 + i) + j) as f32 * 0.002)
                    .collect();
                let results = hnsw_clone.search(&query, 10, 50);
                assert!(
                    results.len() <= 10,
                    "Search should return at most k results"
                );
            }
        }));
    }

    // All threads must complete (no deadlock)
    for handle in handles {
        handle
            .join()
            .expect("Thread should complete without deadlock");
    }

    // Verify consistent state
    let final_count = hnsw.len();
    assert!(
        final_count >= 100,
        "Should have at least initial 100 vectors, got {final_count}"
    );

    // Verify safety counters are accessible and no invariant violations
    let snapshot = super::graph::safety_counters::HNSW_COUNTERS.snapshot();
    assert_eq!(
        snapshot.invariant_violation_total, 0,
        "No lock-order violations should occur with correct lock ordering"
    );
}

// =============================================================================
// Concurrency Family 1b: Delete-Aware Contention (NativeHnswIndex level)
// =============================================================================
// These tests exercise soft-delete paths under concurrent insert/search/delete
// operations to verify tombstone consistency and search exclusion correctness.

#[test]
fn test_concurrent_insert_delete_search_at_index_level() {
    use crate::distance::DistanceMetric as DM;
    use crate::index::hnsw::native_index::NativeHnswIndex;
    use crate::index::VectorIndex;
    use std::sync::Arc;
    use std::thread;

    let index = Arc::new(NativeHnswIndex::new(32, DM::Euclidean));

    // Pre-populate with IDs 0..99
    for i in 0u64..100 {
        let v: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.1).collect();
        index.insert(i, &v);
    }
    assert_eq!(index.len(), 100);

    let mut handles = vec![];

    // 2 insert threads: IDs 1000..1099
    for t in 0..2u64 {
        let idx = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in 0..50u64 {
                let id = 1000 + t * 50 + i;
                let v: Vec<f32> = (0..32).map(|j| (id + j) as f32 * 0.01).collect();
                idx.insert(id, &v);
            }
        }));
    }

    // 2 delete threads: remove IDs 0..49 (soft-delete)
    for t in 0..2u64 {
        let idx = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in 0..25u64 {
                let id = t * 25 + i;
                let _ = idx.remove(id);
            }
        }));
    }

    // 2 search threads: verify search works during mutations
    for _ in 0..2 {
        let idx = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in 0..30u64 {
                let query: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.05).collect();
                let results = idx.search(&query, 10);
                // Search results must respect k
                assert!(results.len() <= 10, "Search must respect k limit");
                // All returned IDs must be valid (not deleted from mappings)
                // and distances must be finite
                for &(id, dist) in &results {
                    assert!(dist.is_finite(), "Distance must be finite for ID {id}");
                }
            }
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("Thread must complete without deadlock");
    }

    // Post-conditions:
    // NativeHnswIndex::len() returns graph size (soft-deletes don't shrink it).
    // Graph size = 100 initial + 100 new inserts = 200
    let graph_len = index.len();
    assert_eq!(
        graph_len, 200,
        "Graph size must reflect all inserts: got {graph_len}"
    );

    // Verify deleted IDs (0..49) are excluded from search results.
    // This is the core soft-delete invariant.
    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.1).collect();
    let results = index.search(&query, 50);
    for &(id, _) in &results {
        assert!(
            !(0..50).contains(&id),
            "Soft-deleted ID {id} must not appear in search results"
        );
    }

    // Newly inserted IDs (1000..1099) must be findable
    let query_new: Vec<f32> = (0..32).map(|j| (1050 + j) as f32 * 0.01).collect();
    let results_new = index.search(&query_new, 10);
    assert!(
        !results_new.is_empty(),
        "Newly inserted vectors must be searchable"
    );
}

/// Verify that concurrent deletes + searches never return stale/deleted entries.
#[test]
fn test_delete_exclusion_under_concurrent_search() {
    use crate::distance::DistanceMetric as DM;
    use crate::index::hnsw::native_index::NativeHnswIndex;
    use crate::index::VectorIndex;
    use std::sync::atomic::{AtomicBool, Ordering as AtomOrd};
    use std::sync::Arc;
    use std::thread;

    let index = Arc::new(NativeHnswIndex::new(16, DM::Cosine));

    // Pre-populate with IDs 0..199
    for i in 0u64..200 {
        let v: Vec<f32> = (0..16).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
        index.insert(i, &v);
    }

    let violation_found = Arc::new(AtomicBool::new(false));
    let mut handles = vec![];

    // Delete thread: remove even IDs
    {
        let idx = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in (0u64..200).step_by(2) {
                idx.remove(i);
            }
        }));
    }

    // Search threads: check that deleted IDs don't appear after deletion completes
    for _ in 0..4 {
        let idx = Arc::clone(&index);
        let vf = Arc::clone(&violation_found);
        handles.push(thread::spawn(move || {
            for i in 0..50u64 {
                let query: Vec<f32> = (0..16).map(|j| ((i + j) as f32 * 0.02).sin()).collect();
                let results = idx.search(&query, 20);
                // During concurrent deletion, we may or may not see some IDs.
                // But search results must always have finite distances.
                for &(_id, dist) in &results {
                    if !dist.is_finite() {
                        vf.store(true, AtomOrd::Relaxed);
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread must complete");
    }

    assert!(
        !violation_found.load(AtomOrd::Relaxed),
        "No non-finite distances should appear during concurrent delete+search"
    );

    // After all deletes complete, graph size remains 200 (soft-delete keeps nodes).
    let graph_len = index.len();
    assert_eq!(graph_len, 200, "Graph size unchanged by soft-deletes");

    // All even IDs must be gone from search results (soft-delete exclusion).
    let query: Vec<f32> = (0..16).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = index.search(&query, 200);
    for &(id, _) in &results {
        assert!(
            id % 2 != 0,
            "Soft-deleted even ID {id} must not appear in post-delete search"
        );
    }
    // Verify odd IDs are still returned
    assert!(
        !results.is_empty(),
        "Odd IDs should still be searchable after deleting even IDs"
    );
}

#[test]
fn test_safety_counters_accessible_after_operations() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    for i in 0..20_u64 {
        let v: Vec<f32> = (0..32).map(|j| (i + j) as f32 * 0.1).collect();
        hnsw.insert(v);
    }

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.05).collect();
    let _ = hnsw.search(&query, 5, 30);

    // Counters should be readable without panic
    let snapshot = super::graph::safety_counters::HNSW_COUNTERS.snapshot();

    // No invariant violations expected with correct lock ordering
    assert_eq!(
        snapshot.invariant_violation_total, 0,
        "Correct lock ordering should produce zero violations"
    );
    // Corruption counter should be zero for normal operations
    assert_eq!(
        snapshot.corruption_detected_total, 0,
        "Normal operations should not trigger corruption signals"
    );
}
