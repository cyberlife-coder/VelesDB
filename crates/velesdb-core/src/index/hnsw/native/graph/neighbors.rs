//! HNSW neighbor selection and bidirectional connection management.

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::NativeHnsw;

impl<D: DistanceEngine> NativeHnsw<D> {
    /// VAMANA-style neighbor selection with alpha diversification.
    pub(crate) fn select_neighbors(
        &self,
        candidates: &[(NodeId, f32)],
        max_neighbors: usize,
    ) -> Vec<NodeId> {
        if candidates.is_empty() {
            return Vec::new();
        }

        if candidates.len() <= max_neighbors {
            return candidates.iter().map(|(id, _)| *id).collect();
        }

        use rustc_hash::FxHashSet;

        let mut selected: Vec<NodeId> = Vec::with_capacity(max_neighbors);
        let mut selected_set: FxHashSet<NodeId> = FxHashSet::default();

        self.with_vectors_read(|vectors| {
            for &(candidate_id, candidate_dist) in candidates {
                if selected.len() >= max_neighbors {
                    break;
                }

                let candidate_vec = &vectors[candidate_id];

                let is_diverse = selected.iter().all(|&selected_id| {
                    let dist_to_selected =
                        self.distance.distance(candidate_vec, &vectors[selected_id]);
                    self.alpha * candidate_dist <= dist_to_selected
                });

                if is_diverse || selected.is_empty() {
                    selected.push(candidate_id);
                    selected_set.insert(candidate_id);
                }
            }
        });

        if selected.len() < max_neighbors {
            for &(candidate_id, _) in candidates {
                if selected.len() >= max_neighbors {
                    break;
                }
                if selected_set.insert(candidate_id) {
                    selected.push(candidate_id);
                }
            }
        }

        selected
    }

    /// Adds a bidirectional connection between nodes.
    ///
    /// # Lock Ordering (BUG-CORE-001 fix)
    ///
    /// This method respects the global lock order: `vectors` → `layers` → `neighbors`
    /// to prevent deadlocks with `search_layer()` which also follows this order.
    pub(in crate::index::hnsw::native::graph) fn add_bidirectional_connection(
        &self,
        new_node: NodeId,
        neighbor: NodeId,
        layer: usize,
        max_conn: usize,
    ) {
        // Phase 1: Get current neighbors
        let current_neighbors =
            self.with_layers_read(|layers| layers[layer].get_neighbors(neighbor));

        if current_neighbors.len() < max_conn {
            // Simple case: append if absent under a single node write lock
            self.with_layers_read(|layers| {
                let _ = layers[layer].with_neighbors_mut(neighbor, |neighbors| {
                    if !neighbors.contains(&new_node) {
                        neighbors.push(new_node);
                    }
                });
            });
        } else {
            // Pruning case: compute distances while holding vectors read lock only
            let mut all_neighbors = current_neighbors;
            all_neighbors.push(new_node);

            let mut with_dist: Vec<(NodeId, f32)> = self.with_vectors_read(|vectors| {
                let neighbor_vec = &vectors[neighbor];
                all_neighbors
                    .iter()
                    .map(|&n| (n, self.distance.distance(neighbor_vec, &vectors[n])))
                    .collect()
            });

            with_dist.sort_by(|a, b| a.1.total_cmp(&b.1));
            let pruned: Vec<NodeId> = with_dist
                .into_iter()
                .take(max_conn)
                .map(|(n, _)| n)
                .collect();

            // Phase 3: Write pruned neighbors under single node write lock
            self.with_layers_read(|layers| {
                let _ = layers[layer].with_neighbors_mut(neighbor, |neighbors| {
                    *neighbors = pruned;
                });
            });
        }
    }
}
