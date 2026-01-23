//! HNSW Graph Structure
//!
//! Implements the hierarchical navigable small world graph structure
//! as described in the Malkov & Yashunin paper.

use super::distance::DistanceEngine;
use super::layer::{Layer, NodeId};
use super::ordered_float::OrderedFloat;
use parking_lot::RwLock;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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
    /// VAMANA alpha parameter for neighbor diversification (default: 1.0)
    /// Higher values (1.1-1.2) increase graph diversity for better recall at scale
    pub(super) alpha: f32,
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
            alpha: 1.0, // Default: standard HNSW behavior
        }
    }

    /// Creates a new native HNSW index with VAMANA-style diversification.
    ///
    /// # Arguments
    ///
    /// * `distance` - Distance computation engine
    /// * `max_connections` - M parameter
    /// * `ef_construction` - Construction-time ef
    /// * `max_elements` - Initial capacity
    /// * `alpha` - Diversification parameter (1.0 = standard, 1.1-1.2 = more diverse)
    ///
    /// # VAMANA Algorithm
    ///
    /// Higher alpha values favor neighbors that are more spread out in the vector space,
    /// improving recall on large datasets (100K+) at the cost of slightly more graph edges.
    #[must_use]
    pub fn with_alpha(
        distance: D,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
        alpha: f32,
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
            rng_state: AtomicU64::new(0x5DEE_CE66_D1A4_B5B5),
            max_connections,
            max_connections_0,
            ef_construction,
            level_mult,
            alpha,
        }
    }

    /// Returns the alpha diversification parameter.
    #[must_use]
    pub fn get_alpha(&self) -> f32 {
        self.alpha
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

    /// Computes the distance between two vectors using this index's distance engine.
    ///
    /// This is useful for brute-force search operations.
    #[inline]
    #[must_use]
    pub fn compute_distance(&self, a: &[f32], b: &[f32]) -> f32 {
        self.distance.distance(a, b)
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

    /// Multi-entry point search for improved recall on hard queries.
    ///
    /// Uses multiple entry points to explore different regions of the graph,
    /// improving recall on datasets with clusters or hard queries.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Number of neighbors to return
    /// * `ef_search` - Search expansion factor
    /// * `num_probes` - Number of entry points to use (2-4 recommended)
    ///
    /// # Returns
    ///
    /// Vector of (node_id, distance) pairs, sorted by distance.
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

        // Get primary entry point via standard HNSW traversal
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        // Generate additional random entry points for diversity
        let mut entry_points = vec![current_ep];
        if num_probes > 1 && count > 10 {
            let mut state = self.rng_state.load(Ordering::Relaxed);
            for _ in 1..num_probes.min(4) {
                // Simple xorshift for random selection
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let random_id = (state as usize) % count;
                if !entry_points.contains(&random_id) {
                    entry_points.push(random_id);
                }
            }
            self.rng_state.store(state, Ordering::Relaxed);
        }

        // Search from all entry points - use full ef_search budget
        // Multiple entry points expand search coverage, not reduce ef
        let candidates = self.search_layer(query, entry_points, ef_search, 0);

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

    /// Search a single layer with ef candidates.
    ///
    /// # Performance Optimizations (v0.9+)
    ///
    /// - **FxHashSet**: Faster visited set (FNV-1a hash vs SipHash)
    /// - **Cached lock**: Single vectors lock acquisition for entire search
    /// - **Early termination**: Break when candidate distance exceeds result threshold
    fn search_layer(
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

        // Perf: Cache vectors read lock for entire search (avoids repeated lock acquisition)
        let vectors = self.vectors.read();

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

            for neighbor in neighbors {
                if visited.insert(neighbor) {
                    let dist = self.distance.distance(query, &vectors[neighbor]);
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
        result_vec.sort_by(|a, b| a.1.total_cmp(&b.1));
        result_vec
    }

    /// VAMANA-style neighbor selection with alpha diversification.
    ///
    /// Based on DiskANN/VAMANA algorithm. Selects diverse neighbors using:
    /// α × d(q,c) <= d(c,s) condition. Alpha=1.0 is standard HNSW, >1.0 favors diversity.
    pub(crate) fn select_neighbors(
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

        // VAMANA-style heuristic selection with alpha diversification
        let mut selected: Vec<NodeId> = Vec::with_capacity(max_neighbors);
        let mut selected_vecs: Vec<Vec<f32>> = Vec::with_capacity(max_neighbors);

        for &(candidate_id, candidate_dist) in candidates {
            if selected.len() >= max_neighbors {
                break;
            }

            let candidate_vec = self.get_vector(candidate_id);

            // VAMANA condition: α × d(q,c) <= d(c,s) for all selected s
            // With alpha=1.0: standard HNSW heuristic (<=)
            // With alpha>1.0: more selective, favors diversity
            let is_diverse = selected_vecs.iter().all(|selected_vec| {
                let dist_to_selected = self.distance.distance(&candidate_vec, selected_vec);
                // Candidate is diverse if α × dist_to_query <= dist_to_selected
                // Using <= to match original HNSW behavior when alpha=1.0
                self.alpha * candidate_dist <= dist_to_selected
            });

            if is_diverse || selected.is_empty() {
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

    /// Adds a bidirectional connection between nodes.
    ///
    /// # Lock Ordering (BUG-CORE-001 fix)
    ///
    /// This method respects the global lock order: `vectors` → `layers` → `neighbors`
    /// to prevent deadlocks with `search_layer()` which also follows this order.
    ///
    /// **Critical**: We NEVER hold `layers.read()` while calling `get_vector()`.
    /// All vector fetches happen BEFORE or AFTER the layers lock is held.
    fn add_bidirectional_connection(
        &self,
        new_node: NodeId,
        neighbor: NodeId,
        layer: usize,
        max_conn: usize,
    ) {
        // BUG-CORE-001 FIX Phase 1: Pre-fetch neighbor vector (vectors lock only)
        let neighbor_vec = self.get_vector(neighbor);

        // BUG-CORE-001 FIX Phase 2: Get current neighbors (layers lock only, released immediately)
        let current_neighbors = self.layers.read()[layer].get_neighbors(neighbor);

        if current_neighbors.len() < max_conn {
            // Simple case: just add the new node
            let layers = self.layers.read();
            let mut neighbors = layers[layer].get_neighbors(neighbor);
            neighbors.push(new_node);
            layers[layer].set_neighbors(neighbor, neighbors);
        } else {
            // Pruning case: need to compute distances
            // BUG-CORE-001 FIX Phase 3: Pre-fetch ALL vectors BEFORE acquiring layers lock
            let mut all_neighbors = current_neighbors.clone();
            all_neighbors.push(new_node);

            let neighbor_vecs: Vec<(NodeId, Vec<f32>)> = all_neighbors
                .iter()
                .map(|&n| (n, self.get_vector(n)))
                .collect();

            // Compute distances (no locks held)
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

            // BUG-CORE-001 FIX Phase 4: Now acquire layers lock to write (no vectors lock needed)
            self.layers.read()[layer].set_neighbors(neighbor, pruned);
        }
    }
}
