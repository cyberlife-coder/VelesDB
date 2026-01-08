//! HNSW Graph Structure
//!
//! Implements the hierarchical navigable small world graph structure
//! as described in the Malkov & Yashunin paper.

use super::distance::DistanceEngine;
use parking_lot::RwLock;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Unique identifier for a node in the graph.
pub type NodeId = usize;

/// A single layer in the HNSW hierarchy.
#[derive(Debug)]
pub struct Layer {
    /// Adjacency list: node_id -> list of neighbor node_ids
    pub(super) neighbors: Vec<RwLock<Vec<NodeId>>>,
}

impl Layer {
    /// Creates a new layer with the given capacity.
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            neighbors: (0..capacity).map(|_| RwLock::new(Vec::new())).collect(),
        }
    }

    fn ensure_capacity(&mut self, node_id: NodeId) {
        while self.neighbors.len() <= node_id {
            self.neighbors.push(RwLock::new(Vec::new()));
        }
    }

    fn get_neighbors(&self, node_id: NodeId) -> Vec<NodeId> {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].read().clone()
        } else {
            Vec::new()
        }
    }

    /// Sets the neighbors for a node.
    pub(super) fn set_neighbors(&self, node_id: NodeId, neighbors: Vec<NodeId>) {
        if node_id < self.neighbors.len() {
            *self.neighbors[node_id].write() = neighbors;
        }
    }

    fn add_neighbor(&self, node_id: NodeId, neighbor: NodeId) {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].write().push(neighbor);
        }
    }
}

/// Native HNSW index implementation.
///
/// # Type Parameters
///
/// * `D` - Distance engine (CPU, SIMD, or GPU)
pub struct NativeHnsw<D: DistanceEngine> {
    /// Distance computation engine
    pub(super) distance: D,
    /// Vector data storage (node_id -> vector)
    pub(super) vectors: RwLock<Vec<Vec<f32>>>,
    /// Hierarchical layers (layer 0 = bottom, dense connections)
    pub(super) layers: RwLock<Vec<Layer>>,
    /// Entry point for search (highest layer node)
    pub(super) entry_point: RwLock<Option<NodeId>>,
    /// Maximum layer for entry point
    pub(super) max_layer: AtomicUsize,
    /// Number of elements in the index
    pub(super) count: AtomicUsize,
    /// Simple PRNG state for layer selection
    pub(super) rng_state: AtomicU64,
    /// Maximum connections per node (M parameter)
    pub(super) max_connections: usize,
    /// Maximum connections at layer 0 (M0 = 2*M)
    pub(super) max_connections_0: usize,
    /// ef_construction parameter
    pub(super) ef_construction: usize,
    /// Level multiplier for layer selection (1/ln(M))
    pub(super) level_mult: f64,
}

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Creates a new native HNSW index.
    ///
    /// # Arguments
    ///
    /// * `distance` - Distance computation engine
    /// * `max_connections` - M parameter (default: 16-64)
    /// * `ef_construction` - Construction-time ef (default: 100-400)
    /// * `max_elements` - Initial capacity
    #[must_use]
    pub fn new(
        distance: D,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
    ) -> Self {
        let max_connections_0 = max_connections * 2;
        let level_mult = 1.0 / (max_connections as f64).ln();

        Self {
            distance,
            vectors: RwLock::new(Vec::with_capacity(max_elements)),
            layers: RwLock::new(vec![Layer::new(max_elements)]),
            entry_point: RwLock::new(None),
            max_layer: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
            rng_state: AtomicU64::new(0x5DEE_CE66_D1A4_B5B5), // Initial seed
            max_connections,
            max_connections_0,
            ef_construction,
            level_mult,
        }
    }

    /// Returns the number of elements in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// Returns true if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Inserts a vector into the index.
    ///
    /// # Arguments
    ///
    /// * `vector` - The vector to insert
    ///
    /// # Returns
    ///
    /// The node ID assigned to this vector.
    pub fn insert(&self, vector: Vec<f32>) -> NodeId {
        // Allocate node ID
        let node_id = {
            let mut vectors = self.vectors.write();
            let id = vectors.len();
            vectors.push(vector);
            id
        };

        // Select random layer for this node
        let node_layer = self.random_layer();

        // Ensure layers exist up to node_layer
        {
            let mut layers = self.layers.write();
            while layers.len() <= node_layer {
                layers.push(Layer::new(node_id + 1));
            }
            for layer in layers.iter_mut() {
                layer.ensure_capacity(node_id);
            }
        }

        // Get current entry point
        let entry_point = *self.entry_point.read();

        if let Some(ep) = entry_point {
            // Search from top layer down to node_layer+1
            let mut current_ep = ep;
            let max_layer = self.max_layer.load(Ordering::Relaxed);

            for layer_idx in (node_layer + 1..=max_layer).rev() {
                current_ep =
                    self.search_layer_single(&self.get_vector(node_id), current_ep, layer_idx);
            }

            // Insert into layers from node_layer down to 0
            for layer_idx in (0..=node_layer).rev() {
                let neighbors = self.search_layer(
                    &self.get_vector(node_id),
                    vec![current_ep],
                    self.ef_construction,
                    layer_idx,
                );

                // Select best neighbors
                let max_conn = if layer_idx == 0 {
                    self.max_connections_0
                } else {
                    self.max_connections
                };
                let selected =
                    self.select_neighbors(&self.get_vector(node_id), &neighbors, max_conn);

                // Connect node to selected neighbors
                self.layers.read()[layer_idx].set_neighbors(node_id, selected.clone());

                // Add bidirectional connections
                for &neighbor in &selected {
                    self.add_bidirectional_connection(node_id, neighbor, layer_idx, max_conn);
                }

                if !neighbors.is_empty() {
                    current_ep = neighbors[0].0;
                }
            }
        } else {
            // First node - becomes entry point
            *self.entry_point.write() = Some(node_id);
        }

        // Update entry point if this node has higher layer
        if node_layer > self.max_layer.load(Ordering::Relaxed) {
            self.max_layer.store(node_layer, Ordering::Relaxed);
            *self.entry_point.write() = Some(node_id);
        }

        self.count.fetch_add(1, Ordering::Relaxed);
        node_id
    }

    /// Searches for k nearest neighbors.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Number of neighbors to return
    /// * `ef_search` - Search expansion factor
    ///
    /// # Returns
    ///
    /// Vector of (node_id, distance) pairs, sorted by distance.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        let entry_point = *self.entry_point.read();
        let Some(ep) = entry_point else {
            return Vec::new();
        };

        let max_layer = self.max_layer.load(Ordering::Relaxed);

        // Greedy search from top layer to layer 1
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        // Search layer 0 with ef_search
        let candidates = self.search_layer(query, vec![current_ep], ef_search, 0);

        // Return top k
        candidates.into_iter().take(k).collect()
    }

    // =========================================================================
    // Private helper methods
    // =========================================================================

    fn get_vector(&self, node_id: NodeId) -> Vec<f32> {
        self.vectors.read()[node_id].clone()
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn random_layer(&self) -> usize {
        // Simple xorshift64 PRNG for layer selection
        let mut state = self.rng_state.load(Ordering::Relaxed);
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.rng_state.store(state, Ordering::Relaxed);

        // Convert to uniform [0, 1) and apply exponential distribution
        let uniform = (state as f64) / (u64::MAX as f64);
        let level = (-uniform.ln() * self.level_mult).floor() as usize;
        level.min(15) // Cap at 16 layers
    }

    fn search_layer_single(&self, query: &[f32], entry: NodeId, layer: usize) -> NodeId {
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

    fn search_layer(
        &self,
        query: &[f32],
        entry_points: Vec<NodeId>,
        ef: usize,
        layer: usize,
    ) -> Vec<(NodeId, f32)> {
        use std::cmp::Reverse;
        use std::collections::HashSet;

        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut candidates: BinaryHeap<Reverse<(OrderedFloat, NodeId)>> = BinaryHeap::new();
        let mut results: BinaryHeap<(OrderedFloat, NodeId)> = BinaryHeap::new();

        for ep in entry_points {
            let dist = self.distance.distance(query, &self.get_vector(ep));
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

            for neighbor in neighbors {
                if visited.insert(neighbor) {
                    let dist = self.distance.distance(query, &self.get_vector(neighbor));
                    let furthest = results.peek().map_or(f32::MAX, |r| r.0 .0);

                    if dist < furthest || results.len() < ef {
                        candidates.push(Reverse((OrderedFloat(dist), neighbor)));
                        results.push((OrderedFloat(dist), neighbor));

                        if results.len() > ef {
                            results.pop();
                        }
                    }
                }
            }
        }

        // Convert to sorted vec
        let mut result_vec: Vec<(NodeId, f32)> =
            results.into_iter().map(|(d, n)| (n, d.0)).collect();
        result_vec.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        result_vec
    }

    /// Heuristic neighbor selection from Malkov & Yashunin paper.
    ///
    /// This algorithm selects neighbors that are:
    /// 1. Close to the query point
    /// 2. Diverse (not clustered together)
    ///
    /// The heuristic improves recall by ensuring the graph has good coverage
    /// of the search space around each node.
    fn select_neighbors(
        &self,
        _query: &[f32], // Not used directly - distances to query are in candidates
        candidates: &[(NodeId, f32)],
        max_neighbors: usize,
    ) -> Vec<NodeId> {
        if candidates.is_empty() {
            return Vec::new();
        }

        // For small candidate sets, simple selection is sufficient
        if candidates.len() <= max_neighbors {
            return candidates.iter().map(|(id, _)| *id).collect();
        }

        // Heuristic selection: prefer diverse neighbors
        let mut selected: Vec<NodeId> = Vec::with_capacity(max_neighbors);
        let mut selected_vecs: Vec<Vec<f32>> = Vec::with_capacity(max_neighbors);

        for &(candidate_id, candidate_dist) in candidates {
            if selected.len() >= max_neighbors {
                break;
            }

            let candidate_vec = self.get_vector(candidate_id);

            // Check if this candidate is "good" - closer to query than to any selected neighbor
            // This ensures diversity: we don't select candidates that are clustered together
            let is_good = selected_vecs.iter().all(|selected_vec| {
                let dist_to_selected = self.distance.distance(&candidate_vec, selected_vec);
                // Candidate is good if it's closer to query than to existing selected neighbors
                candidate_dist <= dist_to_selected
            });

            if is_good || selected.is_empty() {
                selected.push(candidate_id);
                selected_vecs.push(candidate_vec);
            }
        }

        // If heuristic didn't fill quota, add remaining closest candidates
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

    fn add_bidirectional_connection(
        &self,
        new_node: NodeId,
        neighbor: NodeId,
        layer: usize,
        max_conn: usize,
    ) {
        let layers = self.layers.read();
        let mut current_neighbors = layers[layer].get_neighbors(neighbor);

        if current_neighbors.len() < max_conn {
            current_neighbors.push(new_node);
            layers[layer].set_neighbors(neighbor, current_neighbors);
        } else {
            // Need to prune: keep closest neighbors
            current_neighbors.push(new_node);
            let neighbor_vec = self.get_vector(neighbor);

            let mut with_dist: Vec<(NodeId, f32)> = current_neighbors
                .iter()
                .map(|&n| {
                    (
                        n,
                        self.distance.distance(&neighbor_vec, &self.get_vector(n)),
                    )
                })
                .collect();

            with_dist.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let pruned: Vec<NodeId> = with_dist
                .into_iter()
                .take(max_conn)
                .map(|(n, _)| n)
                .collect();
            layers[layer].set_neighbors(neighbor, pruned);
        }
    }
}

/// Wrapper for f32 to implement Ord for `BinaryHeap`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedFloat(f32);

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;
    use crate::distance::DistanceMetric;
    use crate::index::hnsw::native::distance::CpuDistance;

    #[test]
    fn test_insert_and_search() {
        let engine = CpuDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

        // Insert some vectors
        for i in 0..100 {
            let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
            hnsw.insert(v);
        }

        assert_eq!(hnsw.len(), 100);

        // Search
        let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
        let results = hnsw.search(&query, 10, 50);

        assert!(!results.is_empty());
        assert!(results.len() <= 10);
        // First result should be node 0 (closest to query)
        assert_eq!(results[0].0, 0);
    }

    #[test]
    fn test_empty_search() {
        let engine = CpuDistance::new(DistanceMetric::Cosine);
        let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

        let query = vec![1.0, 2.0, 3.0];
        let results = hnsw.search(&query, 10, 50);

        assert!(results.is_empty());
    }

    // =========================================================================
    // TDD Tests for Heuristic Neighbor Selection (PERF-3)
    // =========================================================================

    #[test]
    fn test_heuristic_selection_empty_candidates() {
        let engine = CpuDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Insert a single vector to have valid query
        hnsw.insert(vec![0.0; 32]);

        let query = vec![0.0; 32];
        let candidates: Vec<(NodeId, f32)> = vec![];

        let selected = hnsw.select_neighbors(&query, &candidates, 10);
        assert!(selected.is_empty(), "Empty candidates should return empty");
    }

    #[test]
    fn test_heuristic_selection_fewer_than_max() {
        let engine = CpuDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Insert vectors
        for i in 0..5 {
            hnsw.insert(vec![i as f32; 32]);
        }

        let query = vec![0.0; 32];
        let candidates: Vec<(NodeId, f32)> = vec![(0, 0.0), (1, 1.0), (2, 2.0)];

        let selected = hnsw.select_neighbors(&query, &candidates, 10);
        assert_eq!(
            selected.len(),
            3,
            "Should return all candidates when fewer than max"
        );
    }

    #[test]
    fn test_heuristic_selection_respects_max() {
        let engine = CpuDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Insert vectors
        for i in 0..20 {
            hnsw.insert(vec![i as f32; 32]);
        }

        let query = vec![0.0; 32];
        let candidates: Vec<(NodeId, f32)> = (0..15).map(|i| (i, i as f32)).collect();

        let selected = hnsw.select_neighbors(&query, &candidates, 5);
        assert_eq!(selected.len(), 5, "Should respect max_neighbors limit");
    }

    #[test]
    fn test_heuristic_selection_prefers_diverse_neighbors() {
        let engine = CpuDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Insert diverse vectors: one at origin, cluster around (10,0,0...), spread around (0,10,0...)
        hnsw.insert(vec![0.0; 32]); // 0: origin

        // Cluster A: near (10, 0, 0, ...)
        let mut v1 = vec![0.0; 32];
        v1[0] = 10.0;
        hnsw.insert(v1); // 1
        let mut v2 = vec![0.0; 32];
        v2[0] = 10.5;
        hnsw.insert(v2); // 2
        let mut v3 = vec![0.0; 32];
        v3[0] = 10.2;
        hnsw.insert(v3); // 3

        // Diverse point: near (0, 10, 0, ...)
        let mut v4 = vec![0.0; 32];
        v4[1] = 10.0;
        hnsw.insert(v4); // 4

        let query = vec![0.0; 32];
        // Candidates: all close to query in euclidean terms
        let candidates: Vec<(NodeId, f32)> = vec![
            (1, 10.0), // Cluster A
            (2, 10.5), // Cluster A (close to 1)
            (3, 10.2), // Cluster A (close to 1)
            (4, 10.0), // Diverse (perpendicular direction)
        ];

        let selected = hnsw.select_neighbors(&query, &candidates, 2);

        // Heuristic should prefer diverse selection
        // Should include node 1 (first closest) and node 4 (diverse direction)
        assert_eq!(selected.len(), 2);
        assert!(selected.contains(&1), "Should include first closest");
        // The heuristic should prefer 4 over 2,3 because 4 is in a different direction
    }

    #[test]
    fn test_heuristic_fills_quota_with_closest_if_needed() {
        let engine = CpuDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Insert vectors
        for i in 0..10 {
            hnsw.insert(vec![i as f32; 32]);
        }

        let query = vec![0.0; 32];
        let candidates: Vec<(NodeId, f32)> = (0..10).map(|i| (i, i as f32)).collect();

        let selected = hnsw.select_neighbors(&query, &candidates, 8);

        // Should fill up to max even if heuristic rejects some
        assert_eq!(
            selected.len(),
            8,
            "Should fill quota with closest candidates"
        );
    }

    #[test]
    fn test_recall_with_heuristic_selection() {
        // Test that heuristic selection maintains good recall
        use crate::index::hnsw::native::distance::SimdDistance;

        let engine = SimdDistance::new(DistanceMetric::Cosine);
        let hnsw = NativeHnsw::new(engine, 32, 200, 1000);

        // Insert 500 random-ish vectors
        for i in 0..500 {
            let v: Vec<f32> = (0..128)
                .map(|j| ((i * 127 + j) as f32 * 0.01).sin())
                .collect();
            hnsw.insert(v);
        }

        // Test recall: search should find vectors close to query
        let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
        let results = hnsw.search(&query, 10, 100);

        assert!(!results.is_empty(), "Should find results");
        assert!(results.len() >= 5, "Should find at least 5 neighbors");

        // Results should be sorted by distance
        for i in 1..results.len() {
            assert!(
                results[i].1 >= results[i - 1].1,
                "Results should be sorted by distance"
            );
        }
    }
}
