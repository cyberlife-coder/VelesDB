//! `RaBitQ` graph traversal for `RaBitQPrecisionHnsw`.
//!
//! Implements the layer-0 expansion and greedy upper-layer descent using
//! `RaBitQ` binary distances (XOR + popcount with affine correction).
//!
//! Separated from `rabitq_precision.rs` to keep each file under 500 NLOC.

use super::distance::DistanceEngine;
use super::graph::NO_ENTRY_POINT;
use super::layer::NodeId;
use super::rabitq_precision::RaBitQPrecisionHnsw;
use crate::quantization::{PreparedQuery, RaBitQIndex, RaBitQVectorStore};
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::Ordering;

/// Ordered distance wrapper for the `RaBitQ` traversal heaps.
///
/// Uses `total_cmp` for consistent NaN-safe ordering.
#[derive(Clone, Copy)]
struct DistNode {
    dist: f32,
    node: NodeId,
}

impl PartialEq for DistNode {
    fn eq(&self, other: &Self) -> bool {
        self.dist.total_cmp(&other.dist) == std::cmp::Ordering::Equal
    }
}

impl Eq for DistNode {}

impl PartialOrd for DistNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DistNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.dist.total_cmp(&other.dist)
    }
}

impl<D: DistanceEngine> RaBitQPrecisionHnsw<D> {
    /// Search using `RaBitQ` binary distances for graph traversal.
    ///
    /// Phase 1: Greedy descent through upper layers using `RaBitQ` distances.
    /// Phase 2: Layer-0 expansion with `ef_search` candidates.
    pub(super) fn search_layer_rabitq(
        &self,
        prepared: &PreparedQuery,
        k: usize,
        ef_search: usize,
        rabitq: &RaBitQIndex,
        store: &RaBitQVectorStore,
    ) -> Vec<(NodeId, f32)> {
        let ep = self.inner.entry_point.load(Ordering::Acquire);
        if ep == NO_ENTRY_POINT {
            return Vec::new();
        }

        let max_layer = self.inner.max_layer.load(Ordering::Relaxed);

        // Phase 1: Greedy descent from top layer to layer 1
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.greedy_search_rabitq(prepared, current_ep, layer_idx, rabitq, store);
        }

        // Phase 2: Layer 0 expansion
        self.expand_layer0_rabitq(prepared, current_ep, ef_search.max(k), k, rabitq, store)
    }

    /// Greedy search in a single upper layer using `RaBitQ` distances.
    fn greedy_search_rabitq(
        &self,
        prepared: &PreparedQuery,
        entry: NodeId,
        layer: usize,
        rabitq: &RaBitQIndex,
        store: &RaBitQVectorStore,
    ) -> NodeId {
        let mut current = entry;
        let mut current_dist =
            rabitq_distance(prepared, store, rabitq, current).unwrap_or(f32::MAX);

        loop {
            let mut improved = false;
            let layers = self.inner.layers.read();
            let _ = layers[layer].with_neighbors(current, |neighbors| {
                for &neighbor in neighbors {
                    if let Some(dist) = rabitq_distance(prepared, store, rabitq, neighbor) {
                        if dist < current_dist {
                            current = neighbor;
                            current_dist = dist;
                            improved = true;
                        }
                    }
                }
            });

            if !improved {
                break;
            }
        }

        current
    }

    /// Expands layer 0 with `ef` candidates using `RaBitQ` distances.
    ///
    /// Returns the top-k candidates sorted by `RaBitQ` distance.
    fn expand_layer0_rabitq(
        &self,
        prepared: &PreparedQuery,
        ep: NodeId,
        ef: usize,
        k: usize,
        rabitq: &RaBitQIndex,
        store: &RaBitQVectorStore,
    ) -> Vec<(NodeId, f32)> {
        let mut visited: FxHashSet<NodeId> = FxHashSet::default();
        let mut candidates: BinaryHeap<Reverse<DistNode>> = BinaryHeap::new();
        let mut results: BinaryHeap<DistNode> = BinaryHeap::new();

        Self::init_rabitq_search(
            prepared,
            ep,
            rabitq,
            store,
            &mut visited,
            &mut candidates,
            &mut results,
        );

        while let Some(Reverse(closest)) = candidates.pop() {
            let furthest_dist = results.peek().map_or(f32::MAX, |r| r.dist);
            if closest.dist > furthest_dist && results.len() >= ef {
                break;
            }

            let layers = self.inner.layers.read();
            let _ = layers[0].with_neighbors(closest.node, |neighbors| {
                Self::process_rabitq_neighbors(
                    prepared,
                    neighbors,
                    rabitq,
                    store,
                    ef,
                    &mut visited,
                    &mut candidates,
                    &mut results,
                );
            });
        }

        let mut result_vec: Vec<(NodeId, f32)> =
            results.into_iter().map(|dn| (dn.node, dn.dist)).collect();
        result_vec.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
        result_vec.truncate(k);
        result_vec
    }

    /// Seeds the search state with the entry point.
    fn init_rabitq_search(
        prepared: &PreparedQuery,
        ep: NodeId,
        rabitq: &RaBitQIndex,
        store: &RaBitQVectorStore,
        visited: &mut FxHashSet<NodeId>,
        candidates: &mut BinaryHeap<Reverse<DistNode>>,
        results: &mut BinaryHeap<DistNode>,
    ) {
        if let Some(dist) = rabitq_distance(prepared, store, rabitq, ep) {
            let dn = DistNode { dist, node: ep };
            candidates.push(Reverse(dn));
            results.push(dn);
            visited.insert(ep);
        }
    }

    /// Evaluates neighbor candidates using `RaBitQ` distances.
    #[allow(clippy::too_many_arguments)]
    fn process_rabitq_neighbors(
        prepared: &PreparedQuery,
        neighbors: &[NodeId],
        rabitq: &RaBitQIndex,
        store: &RaBitQVectorStore,
        ef: usize,
        visited: &mut FxHashSet<NodeId>,
        candidates: &mut BinaryHeap<Reverse<DistNode>>,
        results: &mut BinaryHeap<DistNode>,
    ) {
        for &neighbor in neighbors {
            if !visited.insert(neighbor) {
                continue;
            }
            let Some(dist) = rabitq_distance(prepared, store, rabitq, neighbor) else {
                continue;
            };
            let furthest = results.peek().map_or(f32::MAX, |r| r.dist);

            if dist < furthest || results.len() < ef {
                let dn = DistNode {
                    dist,
                    node: neighbor,
                };
                candidates.push(Reverse(dn));
                results.push(dn);
                if results.len() > ef {
                    results.pop();
                }
            }
        }
    }
}

/// Computes the `RaBitQ` distance from a prepared query to a stored vector.
///
/// Returns `None` if the node is not in the `RaBitQ` store (inserted before
/// training, not yet encoded).
fn rabitq_distance(
    prepared: &PreparedQuery,
    store: &RaBitQVectorStore,
    rabitq: &RaBitQIndex,
    node: NodeId,
) -> Option<f32> {
    let bits = store.get_bits_slice(node)?;
    let correction = *store.get_correction(node)?;
    Some(rabitq.distance_from_prepared_slice(prepared, bits, correction))
}
