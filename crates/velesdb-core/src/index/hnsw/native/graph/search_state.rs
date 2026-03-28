//! Search state and helper functions for HNSW layer traversal.
//!
//! [`SearchState`] bundles the candidate/result heaps, visited set,
//! stagnation counter, and cached furthest distance into a single
//! struct. Helper functions [`gather_unvisited_neighbors`] and
//! [`process_batch_results`] operate on the search state to keep
//! each function under Codacy complexity limits.

use super::super::layer::NodeId;
use super::super::ordered_float::OrderedFloat;
use super::search_pools::{
    acquire_candidate_heap, acquire_result_heap, acquire_visited_set, release_candidate_heap,
    release_result_heap, release_visited_set, BitVecVisited, CandidateHeap, ResultHeap,
};
use crate::perf_optimizations::ContiguousVectors;
use smallvec::SmallVec;
use std::cmp::Reverse;

// =============================================================================
// Extracted search helpers (Issue #366, Phase A.1 GREEN)
// =============================================================================

/// Encapsulates the mutable state of a `search_layer` traversal.
///
/// Bundles the candidate/result heaps, visited set, stagnation counter,
/// and a cached copy of the furthest result distance into a single
/// struct to reduce argument counts and keep helper functions under
/// the Codacy complexity limit.
///
/// `cached_furthest` mirrors `results.peek().0` and is updated by
/// [`push_candidate`] and [`evict_furthest`] so that the hot-path
/// termination check in [`should_terminate`] and the admission test
/// in [`process_batch_results`] avoid repeated heap peeks.
pub(super) struct SearchState {
    pub(super) candidates: CandidateHeap,
    pub(super) results: ResultHeap,
    pub(super) visited: BitVecVisited,
    pub(super) stagnation_count: usize,
    /// Cached distance of the furthest (worst) result in the max-heap.
    /// Kept in sync with `results.peek().map(|r| r.0.0)`.
    /// Initialized to `f32::MAX` when the result set is empty.
    pub(super) cached_furthest: f32,
}

impl SearchState {
    /// Creates a fresh search state, acquiring pooled data structures.
    ///
    /// `capacity_hint` is the current node count of the HNSW index, used
    /// to size the bitset so most inserts avoid reallocation. Candidate
    /// and result heaps are also acquired from thread-local pools to
    /// avoid repeated allocation/deallocation during batch searches.
    pub(super) fn new(capacity_hint: usize) -> Self {
        Self {
            candidates: acquire_candidate_heap(),
            results: acquire_result_heap(),
            visited: acquire_visited_set(capacity_hint),
            stagnation_count: 0,
            cached_furthest: f32::MAX,
        }
    }

    /// Pushes a candidate node into both heaps and marks it visited.
    ///
    /// Refreshes `cached_furthest` from the heap root because the newly
    /// pushed node may become the furthest result. Called only during
    /// entry-point seeding (1-4 calls), so the `peek()` cost is negligible.
    #[inline]
    pub(super) fn push_candidate(&mut self, node: NodeId, dist: f32) {
        self.candidates.push(Reverse((OrderedFloat(dist), node)));
        self.results.push((OrderedFloat(dist), node));
        self.cached_furthest = self.results.peek().map_or(f32::MAX, |r| r.0 .0);
        self.visited.insert(node);
    }

    /// Returns `true` if the search should terminate.
    ///
    /// Termination conditions:
    /// 1. The current candidate distance exceeds `cached_furthest` and
    ///    the result set has reached `ef` capacity.
    /// 2. Stagnation limit is enabled and the counter has reached it.
    ///
    /// Uses `cached_furthest` instead of `results.peek()` to avoid a
    /// heap pointer chase on every candidate evaluation (Issue #422).
    #[inline]
    pub(super) fn should_terminate(&self, c_dist: f32, ef: usize, stagnation_limit: usize) -> bool {
        if c_dist > self.cached_furthest && self.results.len() >= ef {
            return true;
        }
        stagnation_limit > 0 && self.stagnation_count >= stagnation_limit
    }

    /// Updates the stagnation counter: resets on improvement, increments otherwise.
    #[inline]
    pub(super) fn update_stagnation(&mut self, improved: bool) {
        if improved {
            self.stagnation_count = 0;
        } else {
            self.stagnation_count += 1;
        }
    }

    /// Consumes the state and returns results sorted by distance ascending.
    ///
    /// When `limit` is `Some(k)`, uses partial sort (`select_nth_unstable_by`)
    /// to efficiently return only the top-k nearest results in O(n + k log k)
    /// instead of O(n log n).
    ///
    /// The visited set is released back to the thread-local pool by the
    /// [`Drop`] impl when `self` goes out of scope.
    pub(super) fn into_sorted_results(mut self, limit: Option<usize>) -> Vec<(NodeId, f32)> {
        let results = std::mem::take(&mut self.results);
        let mut result_vec: Vec<(NodeId, f32)> =
            results.into_iter().map(|(d, n)| (n, d.0)).collect();
        let cmp = |a: &(NodeId, f32), b: &(NodeId, f32)| a.1.total_cmp(&b.1);
        if let Some(k) = limit {
            crate::index::top_k_partial_sort(&mut result_vec, k, cmp);
        } else {
            result_vec.sort_by(cmp);
        }
        result_vec
    }
}

impl Drop for SearchState {
    fn drop(&mut self) {
        // Return pooled data structures. `std::mem::take` leaves a Default
        // (empty) value in `self`, so the subsequent Drop of `self` fields
        // is a no-op.

        let visited = std::mem::take(&mut self.visited);
        if !visited.words.is_empty() {
            release_visited_set(visited);
        }

        let candidates = std::mem::take(&mut self.candidates);
        if candidates.capacity() > 0 {
            release_candidate_heap(candidates);
        }

        let results = std::mem::take(&mut self.results);
        if results.capacity() > 0 {
            release_result_heap(results);
        }
    }
}

/// Speculative prefetch lookahead distance for `gather_unvisited_neighbors`.
///
/// Prefetches this many neighbors ahead in the list. Speculative: a prefetched
/// neighbor may turn out to be already visited, in which case the prefetch is
/// harmless (only wastes a cache line slot, ~64 bytes on x86_64).
const GATHER_PREFETCH_AHEAD: usize = 2;

/// Gathers vector slices for unvisited neighbors, marking them visited.
///
/// When `use_prefetch` is `true`, speculatively prefetches neighbor vectors
/// [`GATHER_PREFETCH_AHEAD`] positions ahead to hide memory latency during
/// the `get_unchecked` calls.
#[inline]
pub(super) fn gather_unvisited_neighbors<'a>(
    neighbors: &[NodeId],
    visited: &mut BitVecVisited,
    vectors: &'a ContiguousVectors,
    use_prefetch: bool,
) -> SmallVec<[(NodeId, &'a [f32]); 32]> {
    let mut batch = SmallVec::new();

    // Speculatively prefetch the first GATHER_PREFETCH_AHEAD neighbor vectors.
    if use_prefetch {
        for &neighbor in neighbors.iter().take(GATHER_PREFETCH_AHEAD) {
            vectors.prefetch(neighbor);
        }
    }

    for (i, &neighbor) in neighbors.iter().enumerate() {
        // Speculatively prefetch the vector GATHER_PREFETCH_AHEAD positions
        // ahead while processing the current neighbor.
        if use_prefetch {
            if let Some(&ahead) = neighbors.get(i + GATHER_PREFETCH_AHEAD) {
                vectors.prefetch(ahead);
            }
        }

        if visited.insert(neighbor) {
            // SAFETY: neighbor is a valid node_id from the graph's neighbor list,
            // only containing IDs of successfully inserted nodes.
            // - Condition 1: neighbor < vectors.len().
            // Reason: Batch gathering of unvisited neighbor vectors.
            let vec = unsafe { vectors.get_unchecked(neighbor) };
            batch.push((neighbor, vec));
        }
    }
    batch
}

/// Processes batch distance results into the search state heaps.
///
/// Uses `state.cached_furthest` for the admission test instead of
/// `results.peek()`, and refreshes it after each eviction (Issue #422).
///
/// Returns `true` if any neighbor improved the result set.
#[inline]
pub(super) fn process_batch_results(
    batch: &[(NodeId, &[f32])],
    distances: &[f32],
    ef: usize,
    state: &mut SearchState,
) -> bool {
    let mut improved = false;
    for (&(node_id, _), &dist) in batch.iter().zip(distances.iter()) {
        if dist < state.cached_furthest || state.results.len() < ef {
            state
                .candidates
                .push(Reverse((OrderedFloat(dist), node_id)));
            state.results.push((OrderedFloat(dist), node_id));
            if state.results.len() > ef {
                state.results.pop();
                // Refresh cache: the evicted node was the previous furthest,
                // so the new furthest is the current heap root.
                state.cached_furthest = state.results.peek().map_or(f32::MAX, |r| r.0 .0);
            } else if dist > state.cached_furthest {
                state.cached_furthest = dist;
            }
            improved = true;
        }
    }
    improved
}
