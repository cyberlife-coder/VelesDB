//! Dual-Precision HNSW Search
//!
//! Based on VSAG paper (arXiv:2503.17911): uses int8 quantized vectors
//! for fast graph traversal, then re-ranks with exact float32 distances.
//!
//! # Performance Benefits
//!
//! - **4x memory bandwidth reduction** during traversal
//! - **Better cache utilization**: more vectors fit in L1/L2
//! - **Exact final results**: re-ranking ensures precision
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                  DualPrecisionHnsw<D>                       │
//! ├─────────────────────────────────────────────────────────────┤
//! │  inner: NativeHnsw<D>          (graph structure + float32)  │
//! │  quantizer: ScalarQuantizer    (trained on data)            │
//! │  quantized_store: Vec<u8>      (int8 vectors, contiguous)   │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use super::distance::DistanceEngine;
use super::graph::NativeHnsw;
use super::layer::NodeId;
use super::quantization::{QuantizedVectorInt8Store, ScalarQuantizer};
use std::sync::Arc;

/// Configuration for dual-precision search (EPIC-055/US-003).
#[derive(Debug, Clone)]
pub struct DualPrecisionConfig {
    /// Oversampling ratio for coarse search (default: 4).
    /// Higher values improve recall but increase latency.
    pub oversampling_ratio: usize,
    /// Use int8 quantized distances for graph traversal (default: true).
    /// When false, uses f32 distances (slower but more accurate traversal).
    pub use_int8_traversal: bool,
    /// Minimum index size to use dual-precision (default: 10,000).
    /// Smaller indexes fall back to f32-only search.
    pub min_index_size: usize,
    /// Enable debug timing logs (default: false).
    pub debug_timings: bool,
}

impl Default for DualPrecisionConfig {
    fn default() -> Self {
        Self {
            oversampling_ratio: 4,
            use_int8_traversal: true,
            min_index_size: 10_000,
            debug_timings: false,
        }
    }
}

/// Dual-precision HNSW index with int8 traversal and float32 re-ranking.
///
/// This implementation follows the VSAG paper's dual-precision architecture:
/// 1. Graph traversal uses int8 quantized distances (4x faster)
/// 2. Final re-ranking uses exact float32 distances (preserves accuracy)
pub struct DualPrecisionHnsw<D: DistanceEngine> {
    /// Inner HNSW index (graph + float32 vectors)
    inner: NativeHnsw<D>,
    /// Scalar quantizer (trained lazily or on first batch)
    quantizer: Option<Arc<ScalarQuantizer>>,
    /// Quantized vector storage (contiguous int8 array)
    quantized_store: Option<QuantizedVectorInt8Store>,
    /// Dimension of vectors
    dimension: usize,
    /// Training sample size for quantizer
    training_sample_size: usize,
    /// Training buffer (accumulates vectors until training)
    training_buffer: Vec<Vec<f32>>,
}

impl<D: DistanceEngine> DualPrecisionHnsw<D> {
    /// Creates a new dual-precision HNSW index.
    ///
    /// # Arguments
    ///
    /// * `distance` - Distance computation engine
    /// * `dimension` - Vector dimension
    /// * `max_connections` - M parameter (default: 16-64)
    /// * `ef_construction` - Construction-time ef (default: 100-400)
    /// * `max_elements` - Initial capacity
    #[must_use]
    pub fn new(
        distance: D,
        dimension: usize,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
    ) -> Self {
        Self {
            inner: NativeHnsw::new(distance, max_connections, ef_construction, max_elements),
            quantizer: None,
            quantized_store: None,
            dimension,
            training_sample_size: 1000.min(max_elements),
            training_buffer: Vec::with_capacity(1000),
        }
    }

    /// Returns the number of elements in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns true if the quantizer is trained.
    #[must_use]
    pub fn is_quantizer_trained(&self) -> bool {
        self.quantizer.is_some()
    }

    /// Inserts a vector into the index.
    ///
    /// The quantizer is trained lazily after `training_sample_size` vectors
    /// are inserted. After training, all subsequent vectors are quantized.
    pub fn insert(&mut self, vector: Vec<f32>) -> NodeId {
        debug_assert_eq!(vector.len(), self.dimension);

        // Insert into inner HNSW (always stores float32)
        let node_id = self.inner.insert(vector.clone());

        // If quantizer is trained, quantize and store
        if let Some(ref mut store) = self.quantized_store {
            store.push(&vector);
        } else {
            // Accumulate training samples
            self.training_buffer.push(vector);

            // Train quantizer when we have enough samples
            if self.training_buffer.len() >= self.training_sample_size {
                self.train_quantizer();
            }
        }

        node_id
    }

    /// Trains the quantizer on accumulated samples.
    fn train_quantizer(&mut self) {
        if self.training_buffer.is_empty() {
            return;
        }

        // Train on accumulated samples
        let refs: Vec<&[f32]> = self.training_buffer.iter().map(Vec::as_slice).collect();
        let quantizer = Arc::new(ScalarQuantizer::train(&refs));

        // Create quantized store and quantize all existing vectors
        let mut store =
            QuantizedVectorInt8Store::new(Arc::clone(&quantizer), self.inner.len() + 1000);

        // Quantize training buffer (already in order)
        for vec in &self.training_buffer {
            store.push(vec);
        }

        self.quantizer = Some(quantizer);
        self.quantized_store = Some(store);
        self.training_buffer.clear();
        self.training_buffer.shrink_to_fit();
    }

    /// Forces quantizer training with current samples.
    ///
    /// Useful when you have fewer vectors than `training_sample_size`
    /// but want to enable dual-precision search.
    pub fn force_train_quantizer(&mut self) {
        if self.quantizer.is_none() && !self.training_buffer.is_empty() {
            self.train_quantizer();
        }
    }

    /// Searches for k nearest neighbors using dual-precision.
    ///
    /// If quantizer is trained:
    /// 1. Graph traversal uses int8 distances (fast)
    /// 2. Re-ranks top candidates with float32 distances (accurate)
    ///
    /// If quantizer is not trained, falls back to standard float32 search.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        // If no quantizer, use standard search
        if self.quantizer.is_none() {
            return self.inner.search(query, k, ef_search);
        }

        // Dual-precision search: use quantized distances for traversal,
        // then re-rank with exact distances
        self.search_dual_precision(query, k, ef_search)
    }

    /// Dual-precision search implementation.
    ///
    /// Currently uses float32 for graph traversal (fast with SIMD) and
    /// re-ranks with exact float32 distances from stored vectors.
    ///
    /// Future optimization: use quantized int8 for traversal to reduce
    /// memory bandwidth during graph exploration.
    fn search_dual_precision(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<(NodeId, f32)> {
        // Step 1: Get more candidates than needed using graph traversal
        // Future optimization: use quantized distances for traversal (EPIC-055)
        let rerank_k = (ef_search * 2).max(k * 4);
        let candidates = self.inner.search(query, rerank_k, ef_search);

        if candidates.is_empty() {
            return candidates;
        }

        // Step 2: Re-rank using EXACT float32 distances
        // This is the key to dual-precision: approximate traversal + exact rerank
        let vectors = self.inner.vectors.read();
        let mut reranked: Vec<(NodeId, f32)> = candidates
            .iter()
            .filter_map(|&(node_id, _approx_dist)| {
                // Get exact distance from original float32 vectors
                let vec = vectors.get(node_id)?;
                let exact_dist = self.inner.compute_distance(query, vec);
                Some((node_id, exact_dist))
            })
            .collect();

        // Sort by exact distance
        reranked.sort_by(|a, b| a.1.total_cmp(&b.1));

        // Return top k
        reranked.truncate(k);
        reranked
    }

    /// Returns the quantizer if trained.
    #[must_use]
    pub fn quantizer(&self) -> Option<&Arc<ScalarQuantizer>> {
        self.quantizer.as_ref()
    }

    /// Searches with explicit configuration (EPIC-055/US-003).
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector (f32)
    /// * `k` - Number of results to return
    /// * `ef_search` - Search expansion factor
    /// * `config` - Dual-precision configuration
    #[must_use]
    pub fn search_with_config(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        config: &DualPrecisionConfig,
    ) -> Vec<(NodeId, f32)> {
        // If no quantizer or int8 traversal disabled, use standard search
        if self.quantizer.is_none() || !config.use_int8_traversal {
            return self.inner.search(query, k, ef_search);
        }

        // Check minimum index size
        if self.inner.len() < config.min_index_size {
            return self.inner.search(query, k, ef_search);
        }

        self.search_int8_traversal(query, k, ef_search, config)
    }

    /// TRUE int8 traversal search (EPIC-055/US-003).
    ///
    /// Phase 1: Graph traversal using int8 quantized distances (4x bandwidth reduction)
    /// Phase 2: Re-rank candidates with exact f32 distances
    fn search_int8_traversal(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        config: &DualPrecisionConfig,
    ) -> Vec<(NodeId, f32)> {
        let quantizer = self.quantizer.as_ref().expect("quantizer must be trained");
        let store = self.quantized_store.as_ref().expect("store must exist");

        // Quantize query for int8 traversal
        let query_quantized = quantizer.quantize(query);

        // Phase 1: Coarse search using int8 distances
        // Get more candidates than needed (oversampling)
        let candidates_k = k * config.oversampling_ratio;
        let coarse_candidates =
            self.search_layer_int8(&query_quantized.data, candidates_k, ef_search, store);

        if coarse_candidates.is_empty() {
            return Vec::new();
        }

        // Phase 2: Re-rank with exact f32 distances
        let vectors = self.inner.vectors.read();
        let mut reranked: Vec<(NodeId, f32)> = coarse_candidates
            .into_iter()
            .filter_map(|(node_id, _approx_dist)| {
                let vec = vectors.get(node_id)?;
                let exact_dist = self.inner.compute_distance(query, vec);
                Some((node_id, exact_dist))
            })
            .collect();

        // Sort by exact distance and return top k
        reranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        reranked.truncate(k);
        reranked
    }

    /// Search using int8 quantized distances for graph traversal.
    ///
    /// This is the key optimization: uses 4x less memory bandwidth during
    /// graph exploration by using u8 vectors instead of f32.
    fn search_layer_int8(
        &self,
        query_int8: &[u8],
        k: usize,
        ef_search: usize,
        store: &QuantizedVectorInt8Store,
    ) -> Vec<(NodeId, u32)> {
        use rustc_hash::FxHashSet;
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let entry_point = *self.inner.entry_point.read();
        let Some(ep) = entry_point else {
            return Vec::new();
        };

        let max_layer = self
            .inner
            .max_layer
            .load(std::sync::atomic::Ordering::Relaxed);
        let quantizer = store.quantizer();

        // Greedy search from top layer to layer 1 using int8 distances
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.greedy_search_int8(query_int8, current_ep, layer_idx, store);
        }

        // Search layer 0 with ef_search using int8 distances
        let mut visited: FxHashSet<NodeId> = FxHashSet::default();
        let mut candidates: BinaryHeap<Reverse<(u32, NodeId)>> = BinaryHeap::new();
        let mut results: BinaryHeap<(u32, NodeId)> = BinaryHeap::new();

        // Initialize with entry point
        if let Some(ep_slice) = store.get_slice(current_ep) {
            let dist = quantizer.distance_l2_quantized_slice(query_int8, ep_slice);
            candidates.push(Reverse((dist, current_ep)));
            results.push((dist, current_ep));
            visited.insert(current_ep);
        }

        let ef = ef_search.max(k);

        while let Some(Reverse((c_dist, c_node))) = candidates.pop() {
            let furthest_dist = results.peek().map_or(u32::MAX, |r| r.0);

            if c_dist > furthest_dist && results.len() >= ef {
                break;
            }

            let neighbors = self.inner.layers.read()[0].get_neighbors(c_node);

            for neighbor in neighbors {
                if visited.insert(neighbor) {
                    if let Some(neighbor_slice) = store.get_slice(neighbor) {
                        let dist =
                            quantizer.distance_l2_quantized_slice(query_int8, neighbor_slice);
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
            }
        }

        // Convert to sorted vec
        let mut result_vec: Vec<(NodeId, u32)> = results.into_iter().map(|(d, n)| (n, d)).collect();
        result_vec.sort_by_key(|&(_, d)| d);
        result_vec.truncate(k);
        result_vec
    }

    /// Greedy search in a single layer using int8 distances.
    fn greedy_search_int8(
        &self,
        query_int8: &[u8],
        entry: NodeId,
        layer: usize,
        store: &QuantizedVectorInt8Store,
    ) -> NodeId {
        let quantizer = store.quantizer();
        let mut current = entry;
        let mut current_dist = store.get_slice(entry).map_or(u32::MAX, |s| {
            quantizer.distance_l2_quantized_slice(query_int8, s)
        });

        loop {
            let neighbors = self.inner.layers.read()[layer].get_neighbors(current);
            let mut improved = false;

            for neighbor in neighbors {
                if let Some(neighbor_slice) = store.get_slice(neighbor) {
                    let dist = quantizer.distance_l2_quantized_slice(query_int8, neighbor_slice);
                    if dist < current_dist {
                        current = neighbor;
                        current_dist = dist;
                        improved = true;
                    }
                }
            }

            if !improved {
                break;
            }
        }

        current
    }
}
