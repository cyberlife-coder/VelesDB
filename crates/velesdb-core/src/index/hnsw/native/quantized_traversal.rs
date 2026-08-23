//! Codec-generic graph traversal for [`QuantizedPrecisionHnsw`].
//!
//! Implements the layer-0 expansion and greedy upper-layer descent using the
//! codec's compact distances (`RaBitQ` XOR + popcount, SQ8 int8 L2).
//!
//! Separated from `quantized_precision.rs` to keep each file under 500 NLOC.

use super::distance::DistanceEngine;
use super::graph::NO_ENTRY_POINT;
use super::layer::NodeId;
use super::quantized_precision::{QuantizedPrecisionHnsw, TraversalCodec};
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::Ordering;

/// Ordered distance wrapper for the traversal heaps.
///
/// Compares by distance only; [`TraversalCodec::Dist`] is totally ordered
/// (`Ord`), so the heaps stay coherent for every codec.
#[derive(Clone, Copy)]
struct DistNode<T> {
    dist: T,
    node: NodeId,
}

impl<T: Ord> PartialEq for DistNode<T> {
    fn eq(&self, other: &Self) -> bool {
        self.dist == other.dist
    }
}

impl<T: Ord> Eq for DistNode<T> {}

impl<T: Ord> PartialOrd for DistNode<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Ord> Ord for DistNode<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.dist.cmp(&other.dist)
    }
}

impl<D: DistanceEngine, C: TraversalCodec> QuantizedPrecisionHnsw<D, C> {
    /// Search using codec distances for graph traversal.
    ///
    /// Phase 1: Greedy descent through upper layers using codec distances.
    /// Phase 2: Layer-0 expansion with `ef_search` candidates.
    pub(super) fn search_layer_quantized(
        &self,
        prepared: &C::Prepared,
        k: usize,
        ef_search: usize,
        quantizer: &C::Quantizer,
        store: &C::Store,
    ) -> Vec<(NodeId, C::Dist)> {
        let ep = self.inner.entry_point.load(Ordering::Acquire);
        if ep == NO_ENTRY_POINT {
            return Vec::new();
        }

        let max_layer = self.inner.max_layer.load(Ordering::Relaxed);

        // Phase 1: Greedy descent from top layer to layer 1
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep =
                self.greedy_search_quantized(prepared, current_ep, layer_idx, quantizer, store);
        }

        // Phase 2: Layer 0 expansion
        self.expand_layer0_quantized(prepared, current_ep, ef_search.max(k), k, quantizer, store)
    }

    /// Greedy search in a single upper layer using codec distances.
    fn greedy_search_quantized(
        &self,
        prepared: &C::Prepared,
        entry: NodeId,
        layer: usize,
        quantizer: &C::Quantizer,
        store: &C::Store,
    ) -> NodeId {
        let mut current = entry;
        let mut current_dist =
            C::distance(quantizer, store, prepared, current).unwrap_or(C::MAX_DIST);

        loop {
            let mut improved = false;
            let layers = self.inner.layers.read();
            let _ = layers[layer].with_neighbors(current, |neighbors| {
                for &neighbor in neighbors {
                    if let Some(dist) = C::distance(quantizer, store, prepared, neighbor) {
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

    /// Expands layer 0 with `ef` candidates using codec distances.
    ///
    /// Returns the top-k candidates sorted by codec distance.
    fn expand_layer0_quantized(
        &self,
        prepared: &C::Prepared,
        ep: NodeId,
        ef: usize,
        k: usize,
        quantizer: &C::Quantizer,
        store: &C::Store,
    ) -> Vec<(NodeId, C::Dist)> {
        let mut visited: FxHashSet<NodeId> = FxHashSet::default();
        let mut candidates: BinaryHeap<Reverse<DistNode<C::Dist>>> = BinaryHeap::new();
        let mut results: BinaryHeap<DistNode<C::Dist>> = BinaryHeap::new();

        Self::init_quantized_search(
            prepared,
            ep,
            quantizer,
            store,
            &mut visited,
            &mut candidates,
            &mut results,
        );

        while let Some(Reverse(closest)) = candidates.pop() {
            let furthest_dist = results.peek().map_or(C::MAX_DIST, |r| r.dist);
            if closest.dist > furthest_dist && results.len() >= ef {
                break;
            }

            let layers = self.inner.layers.read();
            let _ = layers[0].with_neighbors(closest.node, |neighbors| {
                Self::process_quantized_neighbors(
                    prepared,
                    neighbors,
                    quantizer,
                    store,
                    ef,
                    &mut visited,
                    &mut candidates,
                    &mut results,
                );
            });
        }

        let mut result_vec: Vec<(NodeId, C::Dist)> =
            results.into_iter().map(|dn| (dn.node, dn.dist)).collect();
        result_vec.sort_unstable_by(|a, b| a.1.cmp(&b.1));
        result_vec.truncate(k);
        result_vec
    }

    /// Seeds the search state with the entry point.
    fn init_quantized_search(
        prepared: &C::Prepared,
        ep: NodeId,
        quantizer: &C::Quantizer,
        store: &C::Store,
        visited: &mut FxHashSet<NodeId>,
        candidates: &mut BinaryHeap<Reverse<DistNode<C::Dist>>>,
        results: &mut BinaryHeap<DistNode<C::Dist>>,
    ) {
        if let Some(dist) = C::distance(quantizer, store, prepared, ep) {
            let dn = DistNode { dist, node: ep };
            candidates.push(Reverse(dn));
            results.push(dn);
            visited.insert(ep);
        }
    }

    /// Evaluates neighbor candidates using codec distances.
    #[allow(clippy::too_many_arguments)]
    fn process_quantized_neighbors(
        prepared: &C::Prepared,
        neighbors: &[NodeId],
        quantizer: &C::Quantizer,
        store: &C::Store,
        ef: usize,
        visited: &mut FxHashSet<NodeId>,
        candidates: &mut BinaryHeap<Reverse<DistNode<C::Dist>>>,
        results: &mut BinaryHeap<DistNode<C::Dist>>,
    ) {
        for &neighbor in neighbors {
            if !visited.insert(neighbor) {
                continue;
            }
            let Some(dist) = C::distance(quantizer, store, prepared, neighbor) else {
                continue;
            };
            let furthest = results.peek().map_or(C::MAX_DIST, |r| r.dist);

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
