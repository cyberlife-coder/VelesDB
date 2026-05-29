//! HNSW neighbor selection and bidirectional connection management.

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::NativeHnsw;
use crate::perf_optimizations::ContiguousVectors;
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

                debug_assert!(
                    candidate_id < vectors.len(),
                    "candidate_id {candidate_id} out of bounds (len {})",
                    vectors.len()
                );
                // SAFETY: candidate_id < vectors.len() — verified by debug_assert above.
                // - candidate_id comes from search results (only successfully inserted nodes).
                // Reason: neighbor-selection hot path; bounds check elided after assert.
                let candidate_vec = unsafe { vectors.get_unchecked(candidate_id) };

                // The first neighbor is always accepted; the empty-`selected`
                // short-circuit matches the original vacuous `all()` (which
                // returns true over an empty set), avoiding a needless scan.
                if selected.is_empty()
                    || self.is_candidate_diverse(vectors, candidate_vec, candidate_dist, &selected)
                {
                    selected.push(candidate_id);
                    selected_set.insert(candidate_id);
                }
            }
        });

        Self::backfill_neighbors(candidates, &mut selected, &mut selected_set, max_neighbors);

        selected
    }

    /// VAMANA diversity test: true iff `candidate_vec` is at least `alpha *
    /// candidate_dist` away from every already-selected neighbor.
    #[inline]
    fn is_candidate_diverse(
        &self,
        vectors: &ContiguousVectors,
        candidate_vec: &[f32],
        candidate_dist: f32,
        selected: &[NodeId],
    ) -> bool {
        selected.iter().all(|&selected_id| {
            debug_assert!(
                selected_id < vectors.len(),
                "selected_id {selected_id} out of bounds (len {})",
                vectors.len()
            );
            // SAFETY: selected_id < vectors.len() — verified by debug_assert above.
            // - selected_id was previously inserted via the same code path as candidate_id.
            // Reason: diversity inner loop; bounds check elided after assert.
            let selected_vec = unsafe { vectors.get_unchecked(selected_id) };
            let dist_to_selected = self.distance.distance(candidate_vec, selected_vec);
            self.alpha * candidate_dist <= dist_to_selected
        })
    }

    /// Fills any remaining neighbor slots in candidate order, ignoring
    /// diversity, until `max_neighbors` is reached (skipping duplicates).
    fn backfill_neighbors(
        candidates: &[(NodeId, f32)],
        selected: &mut Vec<NodeId>,
        selected_set: &mut FxHashSet<NodeId>,
        max_neighbors: usize,
    ) {
        if selected.len() >= max_neighbors {
            return;
        }
        for &(candidate_id, _) in candidates {
            if selected.len() >= max_neighbors {
                break;
            }
            if selected_set.insert(candidate_id) {
                selected.push(candidate_id);
            }
        }
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
    ///
    /// When the neighbor already has `max_conn` connections, this performs
    /// redundancy-aware eviction in O(M): find the existing neighbor most
    /// redundant with `new_node` (closest to it), then evict it if `new_node`
    /// is closer to the anchor than the farthest existing neighbor.
    /// This preserves directional diversity without the O(M^2) cost of
    /// full pairwise diversity scoring.
    ///
    /// # Complexity trade-off
    ///
    /// An O(M log M) sort-based approach would rank all candidates by quality
    /// before eviction, but M is small (16-64) and this function is called
    /// once per neighbor per insert — on the hot path of index construction.
    /// The O(M) scan-based eviction was chosen for construction throughput.
    /// Recall quality is enforced by tests: >= 0.80 in unit tests (1K vectors),
    /// >= 0.90 at 100K scale.
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
        debug_assert!(
            neighbor < vectors.len(),
            "neighbor {neighbor} out of bounds (len {})",
            vectors.len()
        );
        // SAFETY: neighbor < vectors.len() — verified by debug_assert above.
        // - neighbor is a NodeId already returned by the layer's neighbor list.
        // Reason: backward-connection write path; bounds check elided after assert.
        let neighbor_vec = unsafe { vectors.get_unchecked(neighbor) };

        let _ = layers[layer].with_neighbors_mut(neighbor, |neighbors| {
            if neighbors.contains(&new_node) {
                return;
            }

            if neighbors.len() < max_conn {
                neighbors.push(new_node);
                return;
            }

            debug_assert!(
                new_node < vectors.len(),
                "new_node {new_node} out of bounds (len {})",
                vectors.len()
            );
            // SAFETY: new_node < vectors.len() — verified by debug_assert above.
            // - new_node was just inserted into the vectors store by the caller.
            // Reason: pruning loop distance computation.
            let new_node_vec = unsafe { vectors.get_unchecked(new_node) };
            let new_dist = self.distance.distance(neighbor_vec, new_node_vec);

            self.evict_most_redundant(neighbors, neighbor_vec, new_node, new_dist, vectors);
        });
    }

    /// Evicts the existing neighbor most redundant with `new_node` (closest
    /// to `new_node`), but only if `new_node` is closer to the anchor than
    /// the farthest existing neighbor. This is an O(M) scan.
    ///
    /// Rationale: replacing the neighbor most similar to `new_node` preserves
    /// directional coverage. The alpha condition (`alpha * new_dist`) ensures
    /// only a truly improving swap happens.
    #[inline]
    fn evict_most_redundant(
        &self,
        neighbors: &mut Vec<NodeId>,
        anchor_vec: &[f32],
        new_node: NodeId,
        new_dist: f32,
        vectors: &crate::perf_optimizations::ContiguousVectors,
    ) {
        debug_assert!(
            new_node < vectors.len(),
            "new_node {new_node} out of bounds (len {})",
            vectors.len()
        );
        // SAFETY: new_node < vectors.len() — verified by debug_assert above.
        // - new_node was just inserted into the vectors store by the caller.
        // Reason: anchor distance for redundancy eviction.
        let new_vec = unsafe { vectors.get_unchecked(new_node) };

        let mut worst_idx = 0;
        let mut worst_dist: f32 = 0.0;
        let mut closest_to_new_idx = 0;
        let mut closest_to_new_dist = f32::MAX;

        for (i, &n) in neighbors.iter().enumerate() {
            debug_assert!(
                n < vectors.len(),
                "n {n} out of bounds (len {})",
                vectors.len()
            );
            // SAFETY: n < vectors.len() — verified by debug_assert above.
            // - n iterates over an existing neighbor list whose entries were inserted previously.
            // Reason: O(M) eviction scan inner loop.
            let n_vec = unsafe { vectors.get_unchecked(n) };
            let d_to_anchor = self.distance.distance(anchor_vec, n_vec);
            let d_to_new = self.distance.distance(new_vec, n_vec);

            if d_to_anchor > worst_dist {
                worst_dist = d_to_anchor;
                worst_idx = i;
            }
            if d_to_new < closest_to_new_dist {
                closest_to_new_dist = d_to_new;
                closest_to_new_idx = i;
            }
        }

        // Strategy: if new_node is closer to anchor than the farthest neighbor,
        // evict the neighbor most redundant with new_node (closest to it).
        // Otherwise fall back to standard farthest-eviction.
        if new_dist < worst_dist {
            let evict_idx = if self.alpha * new_dist <= closest_to_new_dist {
                closest_to_new_idx // Diverse: evict the most redundant
            } else {
                worst_idx // Not diverse enough: evict the farthest
            };
            neighbors.swap_remove(evict_idx);
            neighbors.push(new_node);
        }
    }
}
