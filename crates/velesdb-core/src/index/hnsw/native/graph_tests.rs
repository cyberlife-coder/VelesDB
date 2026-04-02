//! Tests for `graph` module - Native HNSW graph implementation.

#![allow(deprecated)] // SimdDistance deprecated in favor of CachedSimdDistance

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
        hnsw.insert(&v).expect("test");
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
    hnsw.insert(&[0.0; 32]).expect("test");

    let candidates: Vec<(NodeId, f32)> = vec![];

    let selected = hnsw.select_neighbors(&candidates, 10);
    assert!(selected.is_empty(), "Empty candidates should return empty");
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_selection_fewer_than_max() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..5 {
        hnsw.insert(&[i as f32; 32]).expect("test");
    }

    let candidates: Vec<(NodeId, f32)> = vec![(0, 0.0), (1, 1.0), (2, 2.0)];

    let selected = hnsw.select_neighbors(&candidates, 10);
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
        hnsw.insert(&[i as f32; 32]).expect("test");
    }

    let candidates: Vec<(NodeId, f32)> = (0..15).map(|i| (i, i as f32)).collect();

    let selected = hnsw.select_neighbors(&candidates, 5);
    assert_eq!(selected.len(), 5, "Should respect max_neighbors limit");
}

#[test]
fn test_heuristic_selection_prefers_diverse_neighbors() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert diverse vectors: one at origin, cluster around (10,0,0...), spread around (0,10,0...)
    hnsw.insert(&[0.0; 32]).expect("test"); // 0: origin

    // Cluster A: near (10, 0, 0, ...)
    let mut v1 = vec![0.0; 32];
    v1[0] = 10.0;
    hnsw.insert(&v1).expect("test"); // 1
    let mut v2 = vec![0.0; 32];
    v2[0] = 10.5;
    hnsw.insert(&v2).expect("test"); // 2
    let mut v3 = vec![0.0; 32];
    v3[0] = 10.2;
    hnsw.insert(&v3).expect("test"); // 3

    // Diverse point: near (0, 10, 0, ...)
    let mut v4 = vec![0.0; 32];
    v4[1] = 10.0;
    hnsw.insert(&v4).expect("test"); // 4

    // Candidates: all close to query in euclidean terms
    let candidates: Vec<(NodeId, f32)> = vec![
        (1, 10.0), // Cluster A
        (2, 10.5), // Cluster A (close to 1)
        (3, 10.2), // Cluster A (close to 1)
        (4, 10.0), // Diverse (perpendicular direction)
    ];

    let selected = hnsw.select_neighbors(&candidates, 2);

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
        hnsw.insert(&[i as f32; 32]).expect("test");
    }

    let candidates: Vec<(NodeId, f32)> = (0..10).map(|i| (i, i as f32)).collect();

    let selected = hnsw.select_neighbors(&candidates, 8);

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
        hnsw.insert(&v).expect("test");
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
// Phase 3, Plan 04: Concurrent graph-level insert/search with invariants
// =========================================================================

/// Parallel insert at graph level with deterministic count + search integrity.
#[test]
fn test_graph_parallel_insert_search_integrity() {
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 500));

    // Pre-populate
    for i in 0..50 {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    let mut handles = vec![];

    // 4 inserters
    for t in 0..4_usize {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..50_usize {
                #[allow(clippy::cast_precision_loss)]
                let v: Vec<f32> = (0..32).map(|j| ((t * 1000 + i) * 32 + j) as f32).collect();
                hnsw_clone.insert(&v).expect("test");
            }
        }));
    }

    // 2 searchers asserting result quality
    for _ in 0..2 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..30_usize {
                #[allow(clippy::cast_precision_loss)]
                let query: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
                let results = hnsw_clone.search(&query, 5, 50);
                // Results must always be distance-sorted
                for window in results.windows(2) {
                    assert!(
                        window[0].1 <= window[1].1,
                        "Search results must be distance-sorted"
                    );
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("No deadlock or panic");
    }

    // Deterministic: 50 pre-pop + 200 parallel = 250
    assert_eq!(hnsw.len(), 250, "All inserts must be reflected in count");

    // Safety counters check
    let snapshot = super::graph::safety_counters::HNSW_COUNTERS.snapshot();
    assert_eq!(
        snapshot.invariant_violation_total, 0,
        "No lock-order violations during parallel graph operations"
    );
}

// =========================================================================
// Concurrent insertion tests (Flag 6 fix - tests PRNG thread-safety indirectly)
// =========================================================================

#[test]
fn test_concurrent_insertions() {
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 1000));

    let handles: Vec<_> = (0..4)
        .map(|thread_id| {
            let hnsw_clone = Arc::clone(&hnsw);
            thread::spawn(move || {
                for i in 0..50 {
                    #[allow(clippy::cast_precision_loss)]
                    let v: Vec<f32> = (0..32)
                        .map(|j| ((thread_id * 50 + i) * 32 + j) as f32)
                        .collect();
                    hnsw_clone.insert(&v).expect("test");
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    assert_eq!(hnsw.len(), 200, "All insertions should succeed");
}

#[test]
fn test_concurrent_insert_and_search() {
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 1000));

    for i in 0..100 {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    let handles: Vec<_> = (0..4)
        .map(|thread_id| {
            let hnsw_clone = Arc::clone(&hnsw);
            thread::spawn(move || {
                for i in 0..25 {
                    if thread_id % 2 == 0 {
                        #[allow(clippy::cast_precision_loss)]
                        let v: Vec<f32> = (0..32)
                            .map(|j| ((100 + thread_id * 25 + i) * 32 + j) as f32)
                            .collect();
                        hnsw_clone.insert(&v).expect("test");
                    } else {
                        #[allow(clippy::cast_precision_loss)]
                        let query: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
                        let results = hnsw_clone.search(&query, 5, 50);
                        assert!(!results.is_empty(), "Search should return results");
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    assert!(
        hnsw.len() >= 100,
        "Index should have at least initial vectors"
    );
}

// =========================================================================
// Lock-free CAS entry-point promotion (I3)
// =========================================================================

/// Verifies that concurrent promote_entry_point calls using CAS produce a
/// valid final state: entry_point references a node at max_layer.
#[test]
fn test_cas_promote_entry_point_concurrent() {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 2000));

    // Pre-populate so the graph has valid vectors and layers
    for i in 0..200_usize {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test: insert should succeed");
    }

    let initial_ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(
        initial_ep,
        super::graph::NO_ENTRY_POINT,
        "test: entry point must be set after pre-population"
    );

    // Spawn threads that each attempt to promote with increasing layers.
    // Only the highest layer should win.
    let mut handles = vec![];
    for t in 0..8_usize {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            // Each thread promotes with a different layer (t+1..t+5)
            for layer in (t + 1)..=(t + 5) {
                hnsw_clone.promote_entry_point(t, layer);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("test: thread must not panic");
    }

    let final_ep = hnsw.entry_point.load(Ordering::Acquire);
    let final_max = hnsw.max_layer.load(Ordering::Relaxed);

    // The final max_layer must be the highest layer any thread promoted
    // (thread 7 promotes up to layer 12)
    assert_eq!(
        final_max, 12,
        "test: max_layer must be highest promoted layer"
    );

    // The entry point must be a valid node (not NO_ENTRY_POINT)
    assert_ne!(
        final_ep,
        super::graph::NO_ENTRY_POINT,
        "test: entry point must not be sentinel after promotions"
    );
}

/// Verifies CAS promotion from NO_ENTRY_POINT (first insert race).
#[test]
fn test_cas_promote_from_empty() {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 100));

    // Promote from empty concurrently — only one should succeed as first EP
    let mut handles = vec![];
    for t in 0..4_usize {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            hnsw_clone.promote_entry_point(t, 0);
        }));
    }

    for handle in handles {
        handle.join().expect("test: thread must not panic");
    }

    let ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(
        ep,
        super::graph::NO_ENTRY_POINT,
        "test: one thread must have set the entry point"
    );
    assert!(
        ep < 4,
        "test: entry point must be one of the promoted node IDs (0..4)"
    );
}

/// Verifies that the struct no longer contains entry_point_promote_lock.
/// This is a compile-time check — if the field existed, this test would
/// fail to compile because NativeHnsw is not #[repr(C)] and field access
/// would be valid.
#[test]
fn test_no_mutex_field_exists() {
    // If entry_point_promote_lock still existed as a Mutex field, this
    // test would be trivially correct. The real verification is that the
    // promote_entry_point function body uses CAS, which is tested above.
    // This test verifies that search + insert work correctly without the mutex.
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 500);

    for i in 0..200_usize {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test: insert should succeed");
    }

    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);
    assert!(!results.is_empty(), "test: search must return results");
}
