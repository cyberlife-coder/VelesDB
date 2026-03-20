//! HNSW neighbor selection and bidirectional connection management.

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::NativeHnsw;
use rustc_hash::FxHashSet;

impl<D: DistanceEngine> NativeHnsw<D> {
    /// VAMANA-style neighbor selection with alpha diversification.
    #[inline]
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

        let mut selected: Vec<NodeId> = Vec::with_capacity(max_neighbors);
        let mut selected_set: FxHashSet<NodeId> = FxHashSet::default();

        self.with_vectors_read(|vectors| {
            for &(candidate_id, candidate_dist) in candidates {
                if selected.len() >= max_neighbors {
                    break;
                }

                // SAFETY: candidate_id is a valid node_id from the search results,
                // which only contains IDs of successfully inserted nodes.
                // - Condition 1: candidate_id < vectors.len().
                // Reason: Neighbor selection after search — bounds check eliminated.
                let candidate_vec = unsafe { vectors.get_unchecked(candidate_id) };

                let is_diverse = selected.iter().all(|&selected_id| {
                    // SAFETY: selected_id is a valid node_id already confirmed above.
                    // - Condition 1: selected_id < vectors.len().
                    // Reason: Pairwise diversity check in neighbor selection.
                    let dist_to_selected = self
                        .distance
                        .distance(candidate_vec, unsafe { vectors.get_unchecked(selected_id) });
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

    /// Batch-connects a new node to all its selected neighbors in a single lock scope.
    ///
    /// Acquires vectors + layers read locks ONCE, sets forward neighbors for the
    /// new node, then connects back each neighbor (with pruning if needed).
    /// This reduces lock acquisitions from ~2-4 per neighbor to 1 total.
    ///
    /// # Lock Ordering
    ///
    /// Respects `vectors (10) → layers (20) → neighbors (30)`.
    #[inline]
    pub(in crate::index::hnsw::native::graph) fn connect_neighbors_batch(
        &self,
        new_node: NodeId,
        selected: &[NodeId],
        layer: usize,
        max_conn: usize,
    ) {
        self.with_vectors_and_layers_read(|vectors, layers| {
            // Forward: set the new node's neighbor list
            layers[layer].set_neighbors(new_node, selected.to_vec());

            // Backward: connect each neighbor back to the new node
            for &neighbor in selected {
                self.connect_back_with_pruning(
                    new_node, neighbor, layer, max_conn, vectors, layers,
                );
            }
        });
    }

    /// Connects a neighbor back to `new_node`, pruning if the neighbor's list
    /// exceeds `max_conn`. Called under an existing vectors+layers read lock.
    #[inline]
    fn connect_back_with_pruning(
        &self,
        new_node: NodeId,
        neighbor: NodeId,
        layer: usize,
        max_conn: usize,
        vectors: &crate::perf_optimizations::ContiguousVectors,
        layers: &[super::super::layer::Layer],
    ) {
        let neighbor_count = layers[layer]
            .with_neighbors(neighbor, <[usize]>::len)
            .unwrap_or(0);

        if neighbor_count < max_conn {
            // Simple case: append under the per-node write lock (rank 30)
            let _ = layers[layer].with_neighbors_mut(neighbor, |neighbors| {
                if !neighbors.contains(&new_node) {
                    neighbors.push(new_node);
                }
            });
        } else {
            // Pruning case: collect all candidates, compute distances, keep best
            let mut all_neighbors = layers[layer].get_neighbors(neighbor);
            all_neighbors.push(new_node);

            // SAFETY: neighbor is a valid node_id from the graph's neighbor list.
            // - Condition 1: neighbor < vectors.len().
            // Reason: Distance computation for neighbor pruning.
            let neighbor_vec = unsafe { vectors.get_unchecked(neighbor) };
            let mut with_dist: Vec<(NodeId, f32)> = all_neighbors
                .iter()
                .map(|&n| {
                    // SAFETY: n is a valid node_id from the graph's neighbor list
                    // or a just-inserted node_id.
                    // - Condition 1: n < vectors.len().
                    // Reason: Pairwise distance for pruning decision.
                    (
                        n,
                        self.distance
                            .distance(neighbor_vec, unsafe { vectors.get_unchecked(n) }),
                    )
                })
                .collect();

            with_dist.sort_by(|a, b| a.1.total_cmp(&b.1));
            let pruned: Vec<NodeId> = with_dist
                .into_iter()
                .take(max_conn)
                .map(|(n, _)| n)
                .collect();

            let _ = layers[layer].with_neighbors_mut(neighbor, |neighbors| {
                *neighbors = pruned;
            });
        }
    }
}
