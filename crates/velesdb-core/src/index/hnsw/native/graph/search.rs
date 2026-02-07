//! HNSW search operations.

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::super::ordered_float::OrderedFloat;
use super::NativeHnsw;
use std::collections::BinaryHeap;
use std::sync::atomic::Ordering;

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Searches for k nearest neighbors.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        let entry_point = *self.entry_point.read();
        let Some(ep) = entry_point else {
            return Vec::new();
        };

        let max_layer = self.max_layer.load(Ordering::Relaxed);

        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        let candidates = self.search_layer(query, vec![current_ep], ef_search, 0);
        candidates.into_iter().take(k).collect()
    }

    /// Multi-entry point search for improved recall on hard queries.
    #[must_use]
    pub fn search_multi_entry(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        num_probes: usize,
    ) -> Vec<(NodeId, f32)> {
        let entry_point = *self.entry_point.read();
        let Some(ep) = entry_point else {
            return Vec::new();
        };

        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return Vec::new();
        }

        let max_layer = self.max_layer.load(Ordering::Relaxed);

        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        let mut entry_points = vec![current_ep];
        if num_probes > 1 && count > 10 {
            for _ in 1..num_probes.min(4) {
                let old_state = self
                    .rng_state
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |mut state| {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        Some(state)
                    })
                    .unwrap_or_else(|s| s);

                let mut state = old_state;
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;

                let random_id = (state as usize) % count;
                if !entry_points.contains(&random_id) {
                    entry_points.push(random_id);
                }
            }
        }

        let candidates = self.search_layer(query, entry_points, ef_search, 0);
        candidates.into_iter().take(k).collect()
    }

    // =========================================================================
    // Layer-level search helpers
    // =========================================================================

    pub(in crate::index::hnsw::native::graph) fn search_layer_single(
        &self,
        query: &[f32],
        entry: NodeId,
        layer: usize,
    ) -> NodeId {
        let mut best = entry;
        let mut best_dist = self.distance.distance(query, &self.get_vector(entry));

        loop {
            let neighbors = self.layers.read()[layer].get_neighbors(best);
            let mut improved = false;

            for neighbor in neighbors {
                let dist = self.distance.distance(query, &self.get_vector(neighbor));
                if dist < best_dist {
                    best = neighbor;
                    best_dist = dist;
                    improved = true;
                }
            }

            if !improved {
                break;
            }
        }

        best
    }

    /// Search a single layer with ef candidates.
    pub(in crate::index::hnsw::native::graph) fn search_layer(
        &self,
        query: &[f32],
        entry_points: Vec<NodeId>,
        ef: usize,
        layer: usize,
    ) -> Vec<(NodeId, f32)> {
        use rustc_hash::FxHashSet;
        use std::cmp::Reverse;

        let mut visited: FxHashSet<NodeId> = FxHashSet::default();
        let mut candidates: BinaryHeap<Reverse<(OrderedFloat, NodeId)>> = BinaryHeap::new();
        let mut results: BinaryHeap<(OrderedFloat, NodeId)> = BinaryHeap::new();

        let vectors = self.vectors.read();

        let dimension = if vectors.is_empty() {
            0
        } else {
            vectors[0].len()
        };
        let prefetch_distance = crate::simd_native::calculate_prefetch_distance(dimension);

        for ep in entry_points {
            let dist = self.distance.distance(query, &vectors[ep]);
            candidates.push(Reverse((OrderedFloat(dist), ep)));
            results.push((OrderedFloat(dist), ep));
            visited.insert(ep);
        }

        while let Some(Reverse((OrderedFloat(c_dist), c_node))) = candidates.pop() {
            let furthest_dist = results.peek().map_or(f32::MAX, |r| r.0 .0);

            if c_dist > furthest_dist && results.len() >= ef {
                break;
            }

            let neighbors = self.layers.read()[layer].get_neighbors(c_node);

            if dimension >= 384 && neighbors.len() > prefetch_distance {
                for &neighbor_id in neighbors.iter().take(prefetch_distance) {
                    if neighbor_id < vectors.len() {
                        crate::simd_native::prefetch_vector(&vectors[neighbor_id]);
                    }
                }
            }

            for (i, neighbor) in neighbors.iter().enumerate() {
                if dimension >= 384 && i + prefetch_distance < neighbors.len() {
                    let prefetch_id = neighbors[i + prefetch_distance];
                    if prefetch_id < vectors.len() {
                        crate::simd_native::prefetch_vector(&vectors[prefetch_id]);
                    }
                }

                if visited.insert(*neighbor) {
                    let dist = self.distance.distance(query, &vectors[*neighbor]);
                    let furthest = results.peek().map_or(f32::MAX, |r| r.0 .0);

                    if dist < furthest || results.len() < ef {
                        candidates.push(Reverse((OrderedFloat(dist), *neighbor)));
                        results.push((OrderedFloat(dist), *neighbor));

                        if results.len() > ef {
                            results.pop();
                        }
                    }
                }
            }
        }

        let mut result_vec: Vec<(NodeId, f32)> =
            results.into_iter().map(|(d, n)| (n, d.0)).collect();
        result_vec.sort_by(|a, b| a.1.total_cmp(&b.1));
        result_vec
    }
}
