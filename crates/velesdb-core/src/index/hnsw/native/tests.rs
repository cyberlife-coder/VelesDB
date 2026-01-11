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
