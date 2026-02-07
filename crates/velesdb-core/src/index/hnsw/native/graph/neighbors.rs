//! HNSW neighbor selection and bidirectional connection management.

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::NativeHnsw;

impl<D: DistanceEngine> NativeHnsw<D> {
    /// VAMANA-style neighbor selection with alpha diversification.
    pub(crate) fn select_neighbors(
        &self,
        _query: &[f32],
        candidates: &[(NodeId, f32)],
        max_neighbors: usize,
    ) -> Vec<NodeId> {
        if candidates.is_empty() {
            return Vec::new();
        }

        if candidates.len() <= max_neighbors {
            return candidates.iter().map(|(id, _)| *id).collect();
        }

        let mut selected: Vec<NodeId> = Vec::with_capacity(max_neighbors);
        let mut selected_vecs: Vec<Vec<f32>> = Vec::with_capacity(max_neighbors);

        for &(candidate_id, candidate_dist) in candidates {
            if selected.len() >= max_neighbors {
                break;
            }

            let candidate_vec = self.get_vector(candidate_id);

            let is_diverse = selected_vecs.iter().all(|selected_vec| {
                let dist_to_selected = self.distance.distance(&candidate_vec, selected_vec);
                self.alpha * candidate_dist <= dist_to_selected
            });

            if is_diverse || selected.is_empty() {
                selected.push(candidate_id);
                selected_vecs.push(candidate_vec);
            }
        }

        if selected.len() < max_neighbors {
            for &(candidate_id, _) in candidates {
                if selected.len() >= max_neighbors {
                    break;
                }
                if !selected.contains(&candidate_id) {
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
    ///
    /// **Critical**: We NEVER hold `layers.read()` while calling `get_vector()`.
    /// All vector fetches happen BEFORE or AFTER the layers lock is held.
    pub(in crate::index::hnsw::native::graph) fn add_bidirectional_connection(
        &self,
        new_node: NodeId,
        neighbor: NodeId,
        layer: usize,
        max_conn: usize,
    ) {
        let neighbor_vec = self.get_vector(neighbor);

        let current_neighbors = self.layers.read()[layer].get_neighbors(neighbor);

        if current_neighbors.len() < max_conn {
            let layers = self.layers.read();
            let mut neighbors = layers[layer].get_neighbors(neighbor);
            neighbors.push(new_node);
            layers[layer].set_neighbors(neighbor, neighbors);
        } else {
            let mut all_neighbors = current_neighbors.clone();
            all_neighbors.push(new_node);

            let neighbor_vecs: Vec<(NodeId, Vec<f32>)> = all_neighbors
                .iter()
                .map(|&n| (n, self.get_vector(n)))
                .collect();

            let mut with_dist: Vec<(NodeId, f32)> = neighbor_vecs
                .iter()
                .map(|(n, n_vec)| (*n, self.distance.distance(&neighbor_vec, n_vec)))
                .collect();

            with_dist.sort_by(|a, b| a.1.total_cmp(&b.1));
            let pruned: Vec<NodeId> = with_dist
                .into_iter()
                .take(max_conn)
                .map(|(n, _)| n)
                .collect();

            self.layers.read()[layer].set_neighbors(neighbor, pruned);
        }
    }
}
