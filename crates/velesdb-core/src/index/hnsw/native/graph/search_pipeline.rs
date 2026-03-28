//! Software-pipelined HNSW search.
//!
//! Overlaps prefetch of the *next* candidate's neighbor vectors with
//! distance computation of the *current* batch, hiding main-memory
//! latency behind useful ALU work.
//!
//! Activated automatically when vectors span >= 2 cache lines **and**
//! the dataset exceeds ~8 MB (so data is not fully L3-resident).
//! Produces **identical** results to the non-pipelined path — only
//! memory access order differs.
//!
//! # Pipeline strategy
//!
//! After gathering the current candidate's unvisited neighbors, the
//! algorithm **peeks** (without popping) at the next candidate in the
//! min-heap and speculatively prefetches that candidate's neighbor
//! vectors. Distance computation for the current batch then executes
//! while the prefetched data migrates through the cache hierarchy.
//!
//! Because the next candidate is only peeked — never consumed before
//! the current batch is fully processed — the heap exploration order
//! is identical to the non-pipelined loop, preserving recall.

use super::super::distance::{batch_distance_with_prefetch, DistanceEngine};
use super::super::layer::{Layer, NodeId};
use super::super::ordered_float::OrderedFloat;
use super::search_state::{gather_unvisited_neighbors, process_batch_results, SearchState};
use crate::perf_optimizations::ContiguousVectors;
use smallvec::SmallVec;
use std::cmp::Reverse;

/// Pipelined search loop for a single HNSW layer.
///
/// The pop-gather-compute-process order is identical to the sequential
/// loop. The only addition is a speculative prefetch of the next
/// candidate's neighbor vectors between gather and compute, so that
/// DRAM latency is hidden behind the current batch's ALU work.
///
/// Called from [`search_layer`] when `should_prefetch()` returns `true`.
///
/// [`search_layer`]: super::NativeHnsw::search_layer
#[inline]
#[allow(clippy::too_many_arguments)]
pub(in crate::index::hnsw::native::graph) fn search_layer_pipelined<D: DistanceEngine>(
    distance: &D,
    query: &[f32],
    vectors: &ContiguousVectors,
    layers: &[Layer],
    state: &mut SearchState,
    ef: usize,
    layer: usize,
    stagnation_limit: usize,
) {
    while let Some(Reverse((OrderedFloat(c_dist), c_node))) = state.candidates.pop() {
        if state.should_terminate(c_dist, ef, stagnation_limit) {
            break;
        }

        let improved =
            gather_prefetch_compute(distance, query, vectors, layers, state, ef, layer, c_node);

        state.update_stagnation(improved);
    }
}

/// Gathers the current candidate's neighbors, speculatively prefetches
/// the next candidate's neighbor vectors, then computes distances for
/// the current batch.
///
/// Returns `true` if any neighbor improved the result set.
#[inline]
#[allow(clippy::too_many_arguments)]
fn gather_prefetch_compute<D: DistanceEngine>(
    distance: &D,
    query: &[f32],
    vectors: &ContiguousVectors,
    layers: &[Layer],
    state: &mut SearchState,
    ef: usize,
    layer: usize,
    c_node: NodeId,
) -> bool {
    let batch = layers[layer]
        .with_neighbors(c_node, |neighbors| {
            gather_unvisited_neighbors(neighbors, &mut state.visited, vectors, true)
        })
        .unwrap_or_default();

    if batch.is_empty() {
        return false;
    }

    // Speculative prefetch: peek at the next candidate and prefetch
    // its neighbor vectors while we compute distances for `batch`.
    prefetch_next_candidate(state, layers, layer, vectors);

    compute_and_process(distance, query, &batch, ef, state)
}

/// Peeks at the next candidate in the min-heap and prefetches its
/// neighbor vectors into CPU cache.
///
/// This is speculative: if processing the current batch adds a closer
/// candidate that displaces the peeked one, the prefetch is wasted
/// but harmless (only occupies a few cache lines).
#[inline]
fn prefetch_next_candidate(
    state: &SearchState,
    layers: &[Layer],
    layer: usize,
    vectors: &ContiguousVectors,
) {
    let Some(Reverse((_, peek_node))) = state.candidates.peek() else {
        return;
    };
    let peek_node = *peek_node;
    layers[layer].with_neighbors(peek_node, |neighbors| {
        prefetch_neighbor_vectors(neighbors, vectors);
    });
}

/// Issues prefetch hints for each neighbor's vector data.
///
/// Bounded by the neighbor list size (typically M=16..64), so the
/// number of prefetch instructions is small and predictable.
#[inline]
fn prefetch_neighbor_vectors(neighbors: &[NodeId], vectors: &ContiguousVectors) {
    for &neighbor in neighbors {
        vectors.prefetch(neighbor);
    }
}

/// Computes batch distances and processes results into the search state.
///
/// Returns `true` if any neighbor improved the result set.
#[inline]
fn compute_and_process<D: DistanceEngine>(
    distance: &D,
    query: &[f32],
    batch: &[(NodeId, &[f32])],
    ef: usize,
    state: &mut SearchState,
) -> bool {
    let vecs: SmallVec<[&[f32]; 32]> = batch.iter().map(|(_, v)| *v).collect();
    let distances = batch_distance_with_prefetch(distance, query, &vecs);
    process_batch_results(batch, &distances, ef, state)
}
