// =============================================================================
// Contract tests for SearchState, gather_unvisited_neighbors, and
// process_batch_results helpers extracted from search_layer (Issue #366).
// =============================================================================

use super::super::distance::SimdDistance;
use super::super::layer::NodeId;
use super::super::ordered_float::OrderedFloat;
use super::search_pools::{BitVecVisited, CANDIDATE_HEAP_POOL, POOL_MAX, RESULT_HEAP_POOL};
use super::search_state::{gather_unvisited_neighbors, process_batch_results, SearchState};
use super::{NativeHnsw, NO_ENTRY_POINT};
use crate::distance::DistanceMetric;
use rustc_hash::FxHashSet;
use smallvec::SmallVec;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

// =========================================================================
// Helpers
// =========================================================================

/// Brute-force cosine distance for ground-truth computation.
#[allow(dead_code)]
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

// =========================================================================
// 1. SearchState::new + push_candidate
// =========================================================================

#[test]
fn test_search_state_new_and_push() {
    let mut state = SearchState::new(0);

    // Push three candidates with known distances
    state.push_candidate(10, 0.5);
    state.push_candidate(20, 0.1);
    state.push_candidate(30, 0.9);

    // Candidates min-heap: closest first (0.1 at top)
    let (OrderedFloat(top_dist), top_node) = state
        .candidates
        .peek()
        .map(|Reverse(item)| item)
        .copied()
        .expect("candidates should not be empty");
    assert_eq!(top_node, 20, "min-heap should surface closest candidate");
    assert!((top_dist - 0.1).abs() < f32::EPSILON);

    // Results max-heap: furthest first (0.9 at top)
    let &(OrderedFloat(furthest_dist), furthest_node) =
        state.results.peek().expect("results should not be empty");
    assert_eq!(furthest_node, 30, "max-heap should surface furthest result");
    assert!((furthest_dist - 0.9).abs() < f32::EPSILON);

    // Visited set should contain all three
    assert!(state.visited.contains(10));
    assert!(state.visited.contains(20));
    assert!(state.visited.contains(30));
}

// =========================================================================
// 2. SearchState::should_terminate
// =========================================================================

#[test]
fn test_search_state_should_terminate() {
    let mut state = SearchState::new(0);

    // Fill ef=3 results with distances 0.1, 0.3, 0.5
    state.push_candidate(1, 0.1);
    state.push_candidate(2, 0.3);
    state.push_candidate(3, 0.5);

    let ef = 3;
    let stagnation_limit = 10;

    // c_dist=0.6 > furthest(0.5) AND results.len()>=ef => should terminate
    assert!(
        state.should_terminate(0.6, ef, stagnation_limit),
        "should terminate: c_dist > furthest and results full"
    );

    // c_dist=0.4 < furthest(0.5) => should NOT terminate
    assert!(
        !state.should_terminate(0.4, ef, stagnation_limit),
        "should not terminate: c_dist < furthest"
    );

    // c_dist=0.6 > furthest but results.len() < ef => should NOT terminate
    // (simulate by creating state with only 2 results)
    let mut state2 = SearchState::new(0);
    state2.push_candidate(1, 0.1);
    state2.push_candidate(2, 0.3);
    assert!(
        !state2.should_terminate(0.6, ef, stagnation_limit),
        "should not terminate: results not yet full"
    );
}

// =========================================================================
// 3. SearchState stagnation tracking
// =========================================================================

#[test]
fn test_search_state_stagnation() {
    let mut state = SearchState::new(0);

    // Fill ef=2 results
    state.push_candidate(1, 0.1);
    state.push_candidate(2, 0.3);

    let ef = 2;
    let stagnation_limit = 3;

    // Initial stagnation count is 0
    assert_eq!(state.stagnation_count, 0);

    // Three rounds without improvement
    state.update_stagnation(false); // count -> 1
    assert_eq!(state.stagnation_count, 1);
    assert!(!state.should_terminate(0.0, ef, stagnation_limit));

    state.update_stagnation(false); // count -> 2
    assert_eq!(state.stagnation_count, 2);
    assert!(!state.should_terminate(0.0, ef, stagnation_limit));

    state.update_stagnation(false); // count -> 3 >= limit
    assert_eq!(state.stagnation_count, 3);
    assert!(
        state.should_terminate(0.0, ef, stagnation_limit),
        "should terminate after reaching stagnation limit"
    );

    // Improvement resets stagnation
    state.update_stagnation(true); // count -> 0
    assert_eq!(state.stagnation_count, 0);
    assert!(!state.should_terminate(0.0, ef, stagnation_limit));
}

// =========================================================================
// 4. SearchState::into_sorted_results
// =========================================================================

#[test]
fn test_search_state_into_sorted_results() {
    let mut state = SearchState::new(0);

    // Insert results in non-sorted order
    state.push_candidate(10, 0.7);
    state.push_candidate(20, 0.2);
    state.push_candidate(30, 0.5);
    state.push_candidate(40, 0.1);
    state.push_candidate(50, 0.9);

    let sorted = state.into_sorted_results(None);

    // Should be sorted ascending by distance
    assert_eq!(sorted.len(), 5);
    assert_eq!(sorted[0].0, 40); // dist 0.1
    assert_eq!(sorted[1].0, 20); // dist 0.2
    assert_eq!(sorted[2].0, 30); // dist 0.5
    assert_eq!(sorted[3].0, 10); // dist 0.7
    assert_eq!(sorted[4].0, 50); // dist 0.9

    // Verify distances are monotonically non-decreasing
    for window in sorted.windows(2) {
        assert!(
            window[0].1 <= window[1].1,
            "results must be sorted by distance ascending: {} <= {}",
            window[0].1,
            window[1].1,
        );
    }
}

// =========================================================================
// 5. gather_unvisited_neighbors filters visited nodes
// =========================================================================

#[test]
fn test_gather_unvisited_neighbors_filters_visited() {
    let dim = 4;
    let mut vectors =
        crate::perf_optimizations::ContiguousVectors::new(dim, 10).expect("alloc should succeed");
    for i in 0..5_usize {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..dim).map(|j| (i * dim + j) as f32).collect();
        vectors.push(&v).expect("push should succeed");
    }

    let neighbors: Vec<NodeId> = vec![0, 1, 2, 3, 4];
    let mut visited = BitVecVisited::with_capacity(5);
    // Pre-mark nodes 1 and 3 as visited
    visited.insert(1);
    visited.insert(3);

    let unvisited: SmallVec<[(NodeId, &[f32]); 32]> =
        gather_unvisited_neighbors(&neighbors, &mut visited, &vectors, false);

    let ids: Vec<NodeId> = unvisited.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids.len(), 3, "should exclude 2 visited nodes");
    assert!(ids.contains(&0));
    assert!(ids.contains(&2));
    assert!(ids.contains(&4));
    assert!(!ids.contains(&1), "visited node 1 must be excluded");
    assert!(!ids.contains(&3), "visited node 3 must be excluded");
}

// =========================================================================
// 6. gather_unvisited_neighbors marks returned nodes as visited
// =========================================================================

#[test]
fn test_gather_unvisited_neighbors_marks_visited() {
    let dim = 4;
    let mut vectors =
        crate::perf_optimizations::ContiguousVectors::new(dim, 10).expect("alloc should succeed");
    for i in 0..3_usize {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..dim).map(|j| (i * dim + j) as f32).collect();
        vectors.push(&v).expect("push should succeed");
    }

    let neighbors: Vec<NodeId> = vec![0, 1, 2];
    let mut visited = BitVecVisited::with_capacity(3);

    let _unvisited = gather_unvisited_neighbors(&neighbors, &mut visited, &vectors, false);

    // All returned neighbors should now be in the visited set
    assert!(visited.contains(0), "node 0 should be marked visited");
    assert!(visited.contains(1), "node 1 should be marked visited");
    assert!(visited.contains(2), "node 2 should be marked visited");
}

// =========================================================================
// 7. process_batch_results updates candidate and result heaps
// =========================================================================

#[test]
fn test_process_batch_results_updates_heaps() {
    let mut state = SearchState::new(0);
    let ef = 10;

    // Simulate a batch of 3 neighbors with their pre-computed distances
    let dim = 4;
    let vecs: Vec<Vec<f32>> = (0..3)
        .map(|i| (0..dim).map(|j| (i * dim + j) as f32).collect())
        .collect();
    let batch: Vec<(NodeId, &[f32])> = vecs
        .iter()
        .enumerate()
        .map(|(i, v)| (i, v.as_slice()))
        .collect();
    let distances = vec![0.3_f32, 0.1, 0.5];

    let improved = process_batch_results(&batch, &distances, ef, &mut state);

    assert!(improved, "first batch should improve empty state");

    // Candidates should contain all 3
    assert_eq!(state.candidates.len(), 3);

    // Results should contain all 3
    assert_eq!(state.results.len(), 3);

    // Min-candidate should be node 1 (dist 0.1)
    let Reverse((OrderedFloat(min_dist), min_node)) = *state.candidates.peek().expect("non-empty");
    assert_eq!(min_node, 1);
    assert!((min_dist - 0.1).abs() < f32::EPSILON);
}

// =========================================================================
// 8. process_batch_results evicts furthest when results exceed ef
// =========================================================================

#[test]
fn test_process_batch_results_evicts_furthest_when_full() {
    let mut state = SearchState::new(0);
    let ef = 3;

    // Pre-fill with 3 results (ef is full)
    state.push_candidate(10, 0.2);
    state.push_candidate(20, 0.4);
    state.push_candidate(30, 0.6);

    // New batch: one candidate closer than the furthest (0.6), one farther
    let dim = 4;
    let v_close: Vec<f32> = vec![1.0; dim];
    let v_far: Vec<f32> = vec![2.0; dim];
    let batch: Vec<(NodeId, &[f32])> = vec![(40, v_close.as_slice()), (50, v_far.as_slice())];
    let distances = vec![0.3_f32, 0.8];

    let improved = process_batch_results(&batch, &distances, ef, &mut state);

    assert!(
        improved,
        "batch with closer candidate should improve results"
    );

    // Results should still be capped at ef=3
    assert_eq!(state.results.len(), ef, "results must not exceed ef");

    // The furthest result should now be 0.4 (node 20), since:
    //   - 0.6 was evicted when 0.3 was inserted
    //   - 0.8 was rejected (> furthest after eviction)
    let result_ids: Vec<NodeId> = state.results.iter().map(|(_, id)| *id).collect();
    assert!(
        !result_ids.contains(&30),
        "node 30 (dist 0.6) should have been evicted"
    );
    assert!(
        result_ids.contains(&40),
        "node 40 (dist 0.3) should have been admitted"
    );
}

// =========================================================================
// 9. Refactored search recall matches original (regression guard)
//
// This test uses only the existing public API and compiles TODAY.
// After the refactoring, search results must remain identical.
// =========================================================================

#[test]
fn test_refactored_search_recall_matches_original() {
    let dim = 32;
    let n = 200;
    let k = 10;
    let ef_search = 64;
    let n_queries = 10;

    // Build index
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, n);

    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("insert should succeed in test");
    }

    // Verify recall against brute-force ground truth
    let mut total_recall = 0.0_f64;

    for q_idx in 0..n_queries {
        let query = &vectors[q_idx * (n / n_queries)];

        let hnsw_results: Vec<NodeId> = hnsw
            .search(query, k, ef_search)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        // Brute-force ground truth (euclidean)
        let mut brute: Vec<(NodeId, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let dist: f32 = v
                    .iter()
                    .zip(query.iter())
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum();
                (i, dist)
            })
            .collect();
        brute.sort_by(|a, b| a.1.total_cmp(&b.1));
        let ground_truth: Vec<NodeId> = brute.iter().take(k).map(|(id, _)| *id).collect();

        let hits = hnsw_results
            .iter()
            .filter(|id| ground_truth.contains(id))
            .count();

        #[allow(clippy::cast_precision_loss)]
        {
            total_recall += hits as f64 / k as f64;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / n_queries as f64;

    assert!(
        avg_recall >= 0.90,
        "search recall must be >= 90% (got {:.1}%); \
         if this fails after refactoring, the extraction broke correctness",
        avg_recall * 100.0,
    );
}

// =========================================================================
// 10. into_sorted_results with partial sort limit (Issue #373)
// =========================================================================

#[test]
fn test_into_sorted_results_with_limit() {
    let mut state = SearchState::new(0);

    // Insert 10 results with known distances
    for i in 0..10_usize {
        #[allow(clippy::cast_precision_loss)]
        state.push_candidate(i, (10 - i) as f32 * 0.1);
    }
    assert_eq!(state.results.len(), 10);

    let sorted = state.into_sorted_results(Some(3));

    // Exactly 3 results returned
    assert_eq!(sorted.len(), 3, "limit=3 should return exactly 3 results");

    // Must be sorted by distance ascending
    for window in sorted.windows(2) {
        assert!(
            window[0].1 <= window[1].1,
            "results must be sorted ascending: {} <= {}",
            window[0].1,
            window[1].1,
        );
    }

    // Must contain the 3 nearest (smallest distances)
    // Distances were: 0.1, 0.2, ..., 1.0 — top-3 are 0.1, 0.2, 0.3
    assert!(
        (sorted[0].1 - 0.1).abs() < f32::EPSILON,
        "first result should be dist 0.1, got {}",
        sorted[0].1,
    );
    assert!(
        (sorted[1].1 - 0.2).abs() < f32::EPSILON,
        "second result should be dist 0.2, got {}",
        sorted[1].1,
    );
    assert!(
        (sorted[2].1 - 0.3).abs() < f32::EPSILON,
        "third result should be dist 0.3, got {}",
        sorted[2].1,
    );
}

#[test]
fn test_into_sorted_results_without_limit() {
    let mut state = SearchState::new(0);

    // Insert 10 results
    for i in 0..10_usize {
        #[allow(clippy::cast_precision_loss)]
        state.push_candidate(i, (10 - i) as f32 * 0.1);
    }

    let sorted = state.into_sorted_results(None);

    // All 10 results returned
    assert_eq!(sorted.len(), 10, "None limit should return all results");

    // Must be sorted by distance ascending
    for window in sorted.windows(2) {
        assert!(
            window[0].1 <= window[1].1,
            "results must be sorted ascending: {} <= {}",
            window[0].1,
            window[1].1,
        );
    }
}

#[test]
fn test_into_sorted_results_limit_greater_than_results() {
    let mut state = SearchState::new(0);

    // Insert only 5 results
    for i in 0..5_usize {
        #[allow(clippy::cast_precision_loss)]
        state.push_candidate(i, (5 - i) as f32 * 0.1);
    }

    // Request limit=10, but only 5 exist — should not panic
    let sorted = state.into_sorted_results(Some(10));

    assert_eq!(
        sorted.len(),
        5,
        "limit > len should return all available results"
    );

    // Must still be sorted ascending
    for window in sorted.windows(2) {
        assert!(
            window[0].1 <= window[1].1,
            "results must be sorted ascending: {} <= {}",
            window[0].1,
            window[1].1,
        );
    }
}

#[test]
fn test_into_sorted_results_limit_zero() {
    let mut state = SearchState::new(0);

    state.push_candidate(0, 0.5);
    state.push_candidate(1, 0.1);
    state.push_candidate(2, 0.9);

    // limit=0 should return empty (truncate to 0)
    let sorted = state.into_sorted_results(Some(0));
    assert!(sorted.is_empty(), "limit=0 should return empty vec");
}

#[test]
fn test_into_sorted_results_empty_state() {
    let state = SearchState::new(0);

    // None limit on empty state
    let sorted = state.into_sorted_results(None);
    assert!(
        sorted.is_empty(),
        "empty state with None should return empty vec"
    );

    let state2 = SearchState::new(0);

    // Some limit on empty state
    let sorted2 = state2.into_sorted_results(Some(5));
    assert!(
        sorted2.is_empty(),
        "empty state with Some(5) should return empty vec"
    );
}

// =========================================================================
// 11. BitVecVisited unit tests (Issue #420, Component 2)
// =========================================================================

#[test]
fn test_bitvec_visited_insert_and_contains() {
    let mut visited = BitVecVisited::with_capacity(1000);
    assert!(!visited.contains(42));
    visited.insert(42);
    assert!(visited.contains(42));
    assert!(!visited.contains(43));
}

#[test]
fn test_bitvec_visited_insert_returns_newly_inserted() {
    let mut visited = BitVecVisited::with_capacity(100);
    // First insert returns true (newly inserted)
    assert!(visited.insert(10));
    // Second insert of same ID returns false (already present)
    assert!(!visited.insert(10));
    // Different ID returns true
    assert!(visited.insert(11));
}

#[test]
fn test_bitvec_visited_clear_resets() {
    let mut visited = BitVecVisited::with_capacity(100);
    visited.insert(50);
    assert!(visited.contains(50));
    visited.clear();
    assert!(!visited.contains(50));
}

#[test]
fn test_bitvec_visited_clear_preserves_capacity() {
    let mut visited = BitVecVisited::with_capacity(1000);
    let words_before = visited.words.len();
    visited.insert(999);
    visited.clear();
    // Capacity (word count) must not shrink after clear
    assert_eq!(visited.words.len(), words_before);
}

#[test]
fn test_bitvec_visited_out_of_bounds_grows() {
    let mut visited = BitVecVisited::with_capacity(10);
    // Inserting beyond capacity should grow, not panic
    visited.insert(100);
    assert!(visited.contains(100));
    assert!(!visited.contains(99));
}

#[test]
fn test_bitvec_visited_zero_capacity() {
    let mut visited = BitVecVisited::with_capacity(0);
    // Should handle zero capacity gracefully
    assert!(!visited.contains(0));
    visited.insert(0);
    assert!(visited.contains(0));
}

#[test]
fn test_bitvec_visited_word_boundary() {
    let mut visited = BitVecVisited::with_capacity(128);
    // Test around the 64-bit word boundary
    for id in [0, 1, 62, 63, 64, 65, 126, 127] {
        visited.insert(id);
    }
    for id in [0, 1, 62, 63, 64, 65, 126, 127] {
        assert!(visited.contains(id), "should contain {id}");
    }
    // IDs NOT inserted should be absent
    for id in [2, 32, 66, 100] {
        assert!(!visited.contains(id), "should not contain {id}");
    }
}

#[test]
fn test_bitvec_visited_identical_to_hashset() {
    // Same sequence of operations must produce same contains() results
    let mut bv = BitVecVisited::with_capacity(10_000);
    let mut hs = FxHashSet::default();
    let ids = [0, 1, 42, 999, 5000, 9999, 7, 128, 255, 256, 1023];
    for &id in &ids {
        let bv_new = bv.insert(id);
        let hs_new = hs.insert(id);
        assert_eq!(
            bv_new, hs_new,
            "insert return mismatch at {id}: bv={bv_new}, hs={hs_new}"
        );
    }
    for i in 0..10_000 {
        assert_eq!(bv.contains(i), hs.contains(&i), "contains mismatch at {i}");
    }
}

#[test]
fn test_bitvec_visited_recall_regression() {
    // End-to-end recall test: build HNSW, search, verify >= 0.95
    let dim = 32;
    let n = 500;
    let k = 10;
    let ef_search = 128;
    let n_queries = 20;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 200, n);

    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("insert should succeed");
    }

    let mut total_recall = 0.0_f64;
    for q_idx in 0..n_queries {
        let query = &vectors[q_idx * (n / n_queries)];

        let hnsw_ids: Vec<NodeId> = hnsw
            .search(query, k, ef_search)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        // Brute-force ground truth
        let mut brute: Vec<(NodeId, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let dist: f32 = v
                    .iter()
                    .zip(query.iter())
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum();
                (i, dist)
            })
            .collect();
        brute.sort_by(|a, b| a.1.total_cmp(&b.1));
        let gt: Vec<NodeId> = brute.iter().take(k).map(|(id, _)| *id).collect();

        let hits = hnsw_ids.iter().filter(|id| gt.contains(id)).count();
        #[allow(clippy::cast_precision_loss)]
        {
            total_recall += hits as f64 / k as f64;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / n_queries as f64;

    assert!(
        avg_recall >= 0.95,
        "BitVecVisited recall@{k} must be >= 95% (got {:.1}%); \
         bitvec visited set regression detected",
        avg_recall * 100.0,
    );
}

// =========================================================================
// 12. Heap pool tests (Issue #421, Component A)
// =========================================================================

#[test]
fn test_heap_pool_reuses_allocations() {
    // First search state allocates fresh heaps, Drop returns them to pool.
    {
        let mut state = SearchState::new(100);
        state.push_candidate(1, 0.5);
        state.push_candidate(2, 0.3);
        // Drop returns heaps to pool
    }

    // Second search state should reuse pooled heaps (no allocation).
    {
        let mut state = SearchState::new(100);
        // Reused heaps must be empty (cleared on return to pool).
        assert!(
            state.candidates.is_empty(),
            "pooled candidate heap must be empty on acquire"
        );
        assert!(
            state.results.is_empty(),
            "pooled result heap must be empty on acquire"
        );

        // Must still function correctly after pool reuse.
        state.push_candidate(10, 0.1);
        state.push_candidate(20, 0.9);
        assert_eq!(state.candidates.len(), 2);
        assert_eq!(state.results.len(), 2);
    }
}

#[test]
fn test_heap_pool_bounded_size() {
    // Create POOL_MAX + 2 states to exceed the pool limit.
    for _ in 0..(POOL_MAX + 2) {
        let mut state = SearchState::new(50);
        state.push_candidate(1, 0.5);
        // Drop returns heaps to pool (bounded at POOL_MAX).
    }

    // Drain the candidate pool to verify it doesn't exceed POOL_MAX.
    let mut count = 0_usize;
    CANDIDATE_HEAP_POOL.with(|pool| {
        count = pool.borrow().len();
    });
    assert!(
        count <= POOL_MAX,
        "candidate pool must not exceed POOL_MAX ({count} > {})",
        POOL_MAX,
    );

    let mut result_count = 0_usize;
    RESULT_HEAP_POOL.with(|pool| {
        result_count = pool.borrow().len();
    });
    assert!(
        result_count <= POOL_MAX,
        "result pool must not exceed POOL_MAX ({result_count} > {})",
        POOL_MAX,
    );
}

#[test]
fn test_heap_pool_recall_regression() {
    // End-to-end recall test with pooled heaps: build HNSW, search
    // multiple times (triggering pool reuse), verify recall >= 0.95.
    let dim = 32;
    let n = 500;
    let k = 10;
    let ef_search = 128;
    let n_queries = 20;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 200, n);

    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("insert should succeed");
    }

    let mut total_recall = 0.0_f64;
    for q_idx in 0..n_queries {
        let query = &vectors[q_idx * (n / n_queries)];

        let hnsw_ids: Vec<NodeId> = hnsw
            .search(query, k, ef_search)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        // Brute-force ground truth
        let mut brute: Vec<(NodeId, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let dist: f32 = v
                    .iter()
                    .zip(query.iter())
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum();
                (i, dist)
            })
            .collect();
        brute.sort_by(|a, b| a.1.total_cmp(&b.1));
        let gt: Vec<NodeId> = brute.iter().take(k).map(|(id, _)| *id).collect();

        let hits = hnsw_ids.iter().filter(|id| gt.contains(id)).count();
        #[allow(clippy::cast_precision_loss)]
        {
            total_recall += hits as f64 / k as f64;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / n_queries as f64;

    assert!(
        avg_recall >= 0.95,
        "Pooled heap recall@{k} must be >= 95% (got {:.1}%); \
         heap pool regression detected",
        avg_recall * 100.0,
    );
}

// =========================================================================
// 13. Prefetch in gather_unvisited_neighbors tests (Issue #421, Component C)
// =========================================================================

#[test]
fn test_gather_unvisited_neighbors_with_prefetch() {
    // Verify that enabling prefetch does not alter the results of
    // gather_unvisited_neighbors (correctness, not performance).
    let dim = 64; // >= 2 cache lines (128B) so should_prefetch returns true
    let n = 10_usize;
    let mut vectors =
        crate::perf_optimizations::ContiguousVectors::new(dim, n).expect("alloc should succeed");
    for i in 0..n {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..dim).map(|j| (i * dim + j) as f32).collect();
        vectors.push(&v).expect("push should succeed");
    }

    let neighbors: Vec<NodeId> = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    let mut visited_no_pf = BitVecVisited::with_capacity(n);
    let mut visited_pf = BitVecVisited::with_capacity(n);

    // Pre-mark some nodes visited in both
    visited_no_pf.insert(2);
    visited_no_pf.insert(5);
    visited_no_pf.insert(8);
    visited_pf.insert(2);
    visited_pf.insert(5);
    visited_pf.insert(8);

    let batch_no_pf = gather_unvisited_neighbors(&neighbors, &mut visited_no_pf, &vectors, false);
    let batch_pf = gather_unvisited_neighbors(&neighbors, &mut visited_pf, &vectors, true);

    // Both must return the same node IDs in the same order.
    let ids_no_pf: Vec<NodeId> = batch_no_pf.iter().map(|(id, _)| *id).collect();
    let ids_pf: Vec<NodeId> = batch_pf.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids_no_pf, ids_pf, "prefetch must not alter gather results");

    // Verify the expected unvisited nodes: 0,1,3,4,6,7,9
    assert_eq!(ids_pf, vec![0, 1, 3, 4, 6, 7, 9]);
}

// =========================================================================
// 14. cached_furthest correctness (Issue #422, Component A)
// =========================================================================

#[test]
#[allow(clippy::float_cmp)] // Exact float comparison is intentional for sentinel/cache values
fn test_cached_furthest_tracks_push_candidate() {
    let mut state = SearchState::new(0);

    // Initially f32::MAX (empty heap)
    assert_eq!(
        state.cached_furthest,
        f32::MAX,
        "cached_furthest must be f32::MAX when results heap is empty"
    );

    // After first push, cached_furthest == that distance
    state.push_candidate(1, 0.3);
    assert!(
        (state.cached_furthest - 0.3).abs() < f32::EPSILON,
        "cached_furthest must track single-element heap root: got {}",
        state.cached_furthest,
    );

    // After pushing a closer candidate, cached_furthest stays at max
    state.push_candidate(2, 0.1);
    assert!(
        (state.cached_furthest - 0.3).abs() < f32::EPSILON,
        "cached_furthest must remain at max distance: got {}",
        state.cached_furthest,
    );

    // After pushing a farther candidate, cached_furthest updates
    state.push_candidate(3, 0.9);
    assert!(
        (state.cached_furthest - 0.9).abs() < f32::EPSILON,
        "cached_furthest must update to new max: got {}",
        state.cached_furthest,
    );
}

#[test]
fn test_cached_furthest_tracks_batch_eviction() {
    let mut state = SearchState::new(0);
    let ef = 3;

    // Fill to ef capacity
    state.push_candidate(10, 0.2);
    state.push_candidate(20, 0.4);
    state.push_candidate(30, 0.6);

    // Insert a closer candidate that causes eviction of 0.6
    let v: Vec<f32> = vec![1.0; 4];
    let batch: Vec<(NodeId, &[f32])> = vec![(40, v.as_slice())];
    let distances = vec![0.1_f32];
    process_batch_results(&batch, &distances, ef, &mut state);

    // After eviction, cached_furthest should be 0.4 (new max)
    assert!(
        (state.cached_furthest - 0.4).abs() < f32::EPSILON,
        "cached_furthest must refresh after eviction: got {}",
        state.cached_furthest,
    );
}

// =========================================================================
// 15. AtomicUsize entry_point (Issue #422, Component B)
// =========================================================================

#[test]
fn test_atomic_entry_point_starts_as_sentinel() {
    use super::super::distance::CpuDistance;
    use std::sync::atomic::Ordering;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 8, 32, 100);

    // Empty index has NO_ENTRY_POINT sentinel
    let ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_eq!(
        ep, NO_ENTRY_POINT,
        "fresh index must have NO_ENTRY_POINT sentinel"
    );

    // Search on empty index returns empty
    let results = hnsw.search(&[1.0, 2.0, 3.0, 4.0], 5, 64);
    assert!(
        results.is_empty(),
        "search on empty index must return empty"
    );
}

#[test]
fn test_atomic_entry_point_set_after_insert() {
    use std::sync::atomic::Ordering;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 8, 32, 100);

    hnsw.insert(&[1.0, 2.0, 3.0, 4.0])
        .expect("insert should succeed");

    let ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(
        ep, NO_ENTRY_POINT,
        "entry_point must be set after first insert"
    );
    assert_eq!(ep, 0, "first inserted node should be entry point");
}

#[test]
fn test_atomic_entry_point_promotes_higher_layer() {
    use std::sync::atomic::Ordering;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 5000);

    // Insert many vectors; the entry point should eventually
    // be promoted to a node at a higher layer.
    for i in 0..2000_usize {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("insert should succeed");
    }

    let ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(ep, NO_ENTRY_POINT);
    // With 2000 nodes, the entry point should have been promoted
    // beyond node 0 (node 0 is layer 0 with high probability).
    let max_layer = hnsw.max_layer.load(Ordering::Relaxed);
    assert!(
        max_layer > 0,
        "max_layer must be > 0 with 2000 nodes (got {max_layer})"
    );
}

#[test]
fn test_prefetch_recall_regression() {
    // End-to-end recall test with prefetch enabled via higher dimension.
    // Uses dim=64 (256 bytes per vector, > 2 cache lines) so
    // should_prefetch(64) == true in the search path.
    let dim = 64;
    let n = 500;
    let k = 10;
    let ef_search = 128;
    let n_queries = 20;

    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 200, n);

    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("insert should succeed");
    }

    let mut total_recall = 0.0_f64;
    for q_idx in 0..n_queries {
        let query = &vectors[q_idx * (n / n_queries)];

        let hnsw_ids: Vec<NodeId> = hnsw
            .search(query, k, ef_search)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        // Brute-force ground truth
        let mut brute: Vec<(NodeId, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let dist: f32 = v
                    .iter()
                    .zip(query.iter())
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum();
                (i, dist)
            })
            .collect();
        brute.sort_by(|a, b| a.1.total_cmp(&b.1));
        let gt: Vec<NodeId> = brute.iter().take(k).map(|(id, _)| *id).collect();

        let hits = hnsw_ids.iter().filter(|id| gt.contains(id)).count();
        #[allow(clippy::cast_precision_loss)]
        {
            total_recall += hits as f64 / k as f64;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / n_queries as f64;

    assert!(
        avg_recall >= 0.95,
        "Prefetch recall@{k} must be >= 95% (got {:.1}%); \
         prefetch regression detected",
        avg_recall * 100.0,
    );
}
