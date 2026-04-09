//! Int8 graph traversal for `DualPrecisionHnsw`.
//!
//! Implements layer-0 expansion and greedy upper-layer descent using
//! SQ8 quantized distances (L2 over `u8` vectors) for 4x bandwidth
//! reduction during graph exploration.
//!
//! Separated from `dual_precision.rs` to keep each file under 500 NLOC.

use super::distance::DistanceEngine;
use super::graph::NO_ENTRY_POINT;
use super::layer::NodeId;
use super::quantization::QuantizedVectorStore;
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::Ordering;

use super::dual_precision::DualPrecisionHnsw;

impl<D: DistanceEngine> DualPrecisionHnsw<D> {
    /// Search using int8 quantized distances for graph traversal.
    ///
    /// This is the key optimization: uses 4x less memory bandwidth during
    /// graph exploration by using u8 vectors instead of f32.
    pub(super) fn search_layer_int8(
        &self,
        query_int8: &[u8],
        k: usize,
        ef_search: usize,
        store: &QuantizedVectorStore,
    ) -> Vec<(NodeId, u32)> {
        let ep = self.inner.entry_point.load(Ordering::Acquire);
        if ep == NO_ENTRY_POINT {
            return Vec::new();
        }

        let max_layer = self.inner.max_layer.load(Ordering::Relaxed);

        // Greedy search from top layer to layer 1 using int8 distances
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.greedy_search_int8(query_int8, current_ep, layer_idx, store);
        }

        // Layer 0 expansion with ef_search candidates
        self.expand_layer0_int8(query_int8, current_ep, ef_search.max(k), k, store)
    }

    /// Expands layer 0 with `ef` candidates using int8 distances.
    ///
    /// Returns the top-k candidates sorted by quantized L2 distance.
    fn expand_layer0_int8(
        &self,
        query_int8: &[u8],
        ep: NodeId,
        ef: usize,
        k: usize,
        store: &QuantizedVectorStore,
    ) -> Vec<(NodeId, u32)> {
        let mut visited: FxHashSet<NodeId> = FxHashSet::default();
        let mut candidates: BinaryHeap<Reverse<(u32, NodeId)>> = BinaryHeap::new();
        let mut results: BinaryHeap<(u32, NodeId)> = BinaryHeap::new();

        Self::init_search_from_ep(
            store,
            query_int8,
            ep,
            &mut visited,
            &mut candidates,
            &mut results,
        );

        while let Some(Reverse((c_dist, c_node))) = candidates.pop() {
            if c_dist > results.peek().map_or(u32::MAX, |r| r.0) && results.len() >= ef {
                break;
            }

            let layers = self.inner.layers.read();
            let _ = layers[0].with_neighbors(c_node, |neighbors| {
                Self::process_int8_neighbors(
                    store,
                    query_int8,
                    neighbors,
                    ef,
                    &mut visited,
                    &mut candidates,
                    &mut results,
                );
            });
        }

        let mut result_vec: Vec<(NodeId, u32)> = results.into_iter().map(|(d, n)| (n, d)).collect();
        result_vec.sort_by_key(|&(_, d)| d);
        result_vec.truncate(k);
        result_vec
    }

    /// Seeds the search state with the entry point for layer-0 int8 search.
    fn init_search_from_ep(
        store: &QuantizedVectorStore,
        query_int8: &[u8],
        ep: NodeId,
        visited: &mut FxHashSet<NodeId>,
        candidates: &mut BinaryHeap<Reverse<(u32, NodeId)>>,
        results: &mut BinaryHeap<(u32, NodeId)>,
    ) {
        if let Some(ep_slice) = store.get_slice(ep) {
            let dist = store
                .quantizer()
                .distance_l2_quantized_slice(query_int8, ep_slice);
            candidates.push(Reverse((dist, ep)));
            results.push((dist, ep));
            visited.insert(ep);
        }
    }

    /// Evaluates neighbor candidates using int8 distances, updating the search state.
    fn process_int8_neighbors(
        store: &QuantizedVectorStore,
        query_int8: &[u8],
        neighbors: &[NodeId],
        ef: usize,
        visited: &mut FxHashSet<NodeId>,
        candidates: &mut BinaryHeap<Reverse<(u32, NodeId)>>,
        results: &mut BinaryHeap<(u32, NodeId)>,
    ) {
        let quantizer = store.quantizer();
        for &neighbor in neighbors {
            if !visited.insert(neighbor) {
                continue;
            }
            let Some(neighbor_slice) = store.get_slice(neighbor) else {
                continue;
            };
            let dist = quantizer.distance_l2_quantized_slice(query_int8, neighbor_slice);
            let furthest = results.peek().map_or(u32::MAX, |r| r.0);

            if dist < furthest || results.len() < ef {
                candidates.push(Reverse((dist, neighbor)));
                results.push((dist, neighbor));
                if results.len() > ef {
                    results.pop();
                }
            }
        }
    }

    /// Greedy search in a single layer using int8 distances.
    fn greedy_search_int8(
        &self,
        query_int8: &[u8],
        entry: NodeId,
        layer: usize,
        store: &QuantizedVectorStore,
    ) -> NodeId {
        let quantizer = store.quantizer();
        let mut current = entry;
        let mut current_dist = store.get_slice(entry).map_or(u32::MAX, |s| {
            quantizer.distance_l2_quantized_slice(query_int8, s)
        });

        loop {
            let mut improved = false;
            let layers = self.inner.layers.read();
            let _ = layers[layer].with_neighbors(current, |neighbors| {
                for &neighbor in neighbors {
                    if let Some(neighbor_slice) = store.get_slice(neighbor) {
                        let dist =
                            quantizer.distance_l2_quantized_slice(query_int8, neighbor_slice);
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
}
