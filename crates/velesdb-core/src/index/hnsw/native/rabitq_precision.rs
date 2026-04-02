//! `RaBitQ`-Precision HNSW Search
//!
//! Uses `RaBitQ` binary distances (32x compression) for graph traversal
//! and float32 exact distances for final re-ranking. Follows the same
//! dual-precision architecture as `DualPrecisionHnsw` (SQ8) but achieves
//! 8x higher compression at the cost of O(D^2) query preparation.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │               RaBitQPrecisionHnsw<D>                        │
//! ├──────────────────────────────────────────────────────────────┤
//! │  inner: NativeHnsw<D>          (graph structure + float32)  │
//! │  rabitq_index: RaBitQIndex     (rotation + centroid)        │
//! │  rabitq_store: RaBitQVectorStore  (bits + corrections)      │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Performance
//!
//! - **32x memory bandwidth reduction** during traversal (vs 4x for SQ8)
//! - **XOR + popcount** distance: ~2 ns per candidate (vs ~10 ns for f32)
//! - **Query preparation overhead**: ~60 us for 768D (amortized over hundreds
//!   of distance evaluations per search)

use super::distance::DistanceEngine;
use super::graph::{NativeHnsw, NO_ENTRY_POINT};
use super::layer::NodeId;
use crate::quantization::{RaBitQIndex, RaBitQVectorStore};
use parking_lot::{Mutex, RwLock};
use std::sync::Arc;

/// Configuration for `RaBitQ`-precision search.
#[derive(Debug, Clone)]
pub struct RaBitQPrecisionConfig {
    /// Oversampling ratio for coarse search (default: 6).
    ///
    /// `RaBitQ` distances are coarser than SQ8, so a higher ratio (6 vs 4)
    /// compensates for the lower fidelity during graph traversal.
    pub oversampling_ratio: usize,
    /// Minimum index size to activate `RaBitQ` traversal (default: 5000).
    ///
    /// Smaller indexes fall back to f32-only search because the rotation
    /// overhead dominates at low vector counts.
    pub min_index_size: usize,
}

impl Default for RaBitQPrecisionConfig {
    fn default() -> Self {
        Self {
            oversampling_ratio: 6,
            min_index_size: 5000,
        }
    }
}

/// `RaBitQ`-precision HNSW index with binary traversal and float32 re-ranking.
///
/// Graph traversal uses `RaBitQ` binary distances (XOR + popcount, 32x
/// compression). Final re-ranking uses exact float32 distances from the
/// inner `NativeHnsw` vector store.
pub struct RaBitQPrecisionHnsw<D: DistanceEngine> {
    /// Inner HNSW index (graph + float32 vectors).
    pub(in crate::index::hnsw) inner: NativeHnsw<D>,
    /// Trained `RaBitQ` index (rotation matrix + centroid).
    /// Write-locked once during training, then read-only.
    rabitq_index: RwLock<Option<Arc<RaBitQIndex>>>,
    /// Contiguous `RaBitQ`-encoded vector storage.
    rabitq_store: RwLock<Option<RaBitQVectorStore>>,
    /// Vector dimension.
    dimension: usize,
    /// Number of vectors to accumulate before training.
    training_sample_size: usize,
    /// Buffer for vectors awaiting quantizer training.
    training_buffer: Mutex<Vec<Vec<f32>>>,
}

impl<D: DistanceEngine> RaBitQPrecisionHnsw<D> {
    /// Creates a new `RaBitQ`-precision HNSW index with default alpha (1.2).
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new(
        distance: D,
        dimension: usize,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
    ) -> crate::error::Result<Self> {
        Self::new_with_alpha(
            distance,
            dimension,
            max_connections,
            ef_construction,
            max_elements,
            super::graph::DEFAULT_ALPHA,
        )
    }

    /// Creates a new `RaBitQ`-precision HNSW index with a custom alpha.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_alpha(
        distance: D,
        dimension: usize,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
        alpha: f32,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            inner: NativeHnsw::new_with_dimension_and_alpha(
                distance,
                max_connections,
                ef_construction,
                max_elements,
                dimension,
                alpha,
            )?,
            rabitq_index: RwLock::new(None),
            rabitq_store: RwLock::new(None),
            dimension,
            training_sample_size: 1000.min(max_elements),
            training_buffer: Mutex::new(Vec::with_capacity(1000)),
        })
    }

    /// Creates a `RaBitQ`-precision HNSW from a pre-loaded `NativeHnsw` graph.
    ///
    /// The quantizer is NOT trained — it trains lazily from new inserts.
    /// Until trained, search falls back to standard f32 distances.
    #[must_use]
    pub fn from_inner(inner: NativeHnsw<D>, _distance: D, dimension: usize) -> Self {
        Self {
            inner,
            rabitq_index: RwLock::new(None),
            rabitq_store: RwLock::new(None),
            dimension,
            training_sample_size: 1000,
            training_buffer: Mutex::new(Vec::with_capacity(1000)),
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

    /// Returns true if the `RaBitQ` quantizer is trained.
    #[must_use]
    pub fn is_quantizer_trained(&self) -> bool {
        self.rabitq_index.read().is_some()
    }

    /// Inserts a vector into the index.
    ///
    /// The quantizer is trained lazily after `training_sample_size` vectors.
    /// After training, all subsequent vectors are encoded into the `RaBitQ` store.
    ///
    /// Uses interior mutability so the index can be shared across threads.
    ///
    /// # Errors
    ///
    /// Returns an error if allocation, insertion, or encoding fails.
    pub fn insert(&self, vector: &[f32]) -> crate::error::Result<NodeId> {
        debug_assert_eq!(vector.len(), self.dimension);

        let index_guard = self.rabitq_index.read();
        if let Some(rabitq) = index_guard.as_ref().map(Arc::clone) {
            // Drop read lock BEFORE encoding — holding it blocks training.
            drop(index_guard);
            let encoded = rabitq.encode(vector)?;
            if let Some(store) = self.rabitq_store.write().as_mut() {
                store.push(&encoded.bits, encoded.correction);
            }
        } else {
            drop(index_guard);
            self.insert_training_phase(vector)?;
        }

        self.inner.insert(vector)
    }

    /// Handles insert during the pre-training phase.
    ///
    /// Buffers the vector and triggers training when the sample size is reached.
    fn insert_training_phase(&self, vector: &[f32]) -> crate::error::Result<()> {
        let mut buffer = self.training_buffer.lock();
        buffer.push(vector.to_vec());
        if buffer.len() >= self.training_sample_size {
            drop(buffer);
            self.train_rabitq()?;
        }
        Ok(())
    }

    /// Searches for k nearest neighbors using `RaBitQ`-precision.
    ///
    /// If the quantizer is trained, uses `RaBitQ` binary distances for graph
    /// traversal and re-ranks with exact float32 distances. Otherwise, falls
    /// back to standard float32 search.
    ///
    /// All returned distances are in user-visible metric space
    /// (`transform_score` applied).
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        if self.rabitq_index.read().is_none() {
            return self.search_and_transform(query, k, ef_search);
        }

        self.search_rabitq_precision(query, k, ef_search)
    }

    /// Runs `inner.search()` and applies `transform_score` to each result.
    fn search_and_transform(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<(NodeId, f32)> {
        self.inner
            .search(query, k, ef_search)
            .into_iter()
            .map(|(id, raw)| (id, self.inner.transform_score(raw)))
            .collect()
    }

    /// Forces quantizer training with current samples.
    ///
    /// Useful when you have fewer vectors than `training_sample_size`
    /// but want to enable `RaBitQ`-precision search.
    ///
    /// # Errors
    ///
    /// Returns an error if `RaBitQ` training or encoding fails.
    pub fn force_train_quantizer(&self) -> crate::error::Result<()> {
        if self.rabitq_index.read().is_none() && !self.training_buffer.lock().is_empty() {
            self.train_rabitq()?;
        }
        Ok(())
    }
}

// --- Private training and search implementation ---

impl<D: DistanceEngine> RaBitQPrecisionHnsw<D> {
    /// Trains `RaBitQ` from accumulated samples and encodes them.
    ///
    /// Double-checks `rabitq_index` under write lock to prevent concurrent
    /// training races.
    #[cfg(feature = "persistence")]
    fn train_rabitq(&self) -> crate::error::Result<()> {
        // Re-check under write lock: another thread may have trained already
        let mut index_guard = self.rabitq_index.write();
        if index_guard.is_some() {
            return Ok(());
        }

        // Drain buffer atomically — no window for vectors to be pushed
        // then cleared without encoding (fixes race reported by Devin Review).
        let training_data = {
            let mut buffer = self.training_buffer.lock();
            if buffer.is_empty() {
                return Ok(());
            }
            let data = std::mem::take(&mut *buffer);
            buffer.shrink_to_fit();
            data
        };

        let rabitq = Arc::new(RaBitQIndex::train(&training_data, 42)?);
        let mut store = RaBitQVectorStore::new(self.dimension, self.inner.len() + 1000);

        for vec in &training_data {
            let encoded = rabitq.encode(vec)?;
            store.push(&encoded.bits, encoded.correction);
        }

        // Store MUST be visible before index: search checks index first,
        // and a Some(index) + None store would silently skip RaBitQ encoding.
        *self.rabitq_store.write() = Some(store);
        *index_guard = Some(rabitq);
        Ok(())
    }

    /// Stub for non-persistence builds (training requires ndarray/rayon).
    #[cfg(not(feature = "persistence"))]
    fn train_rabitq(&self) -> crate::error::Result<()> {
        Ok(())
    }

    /// `RaBitQ`-precision search: binary traversal + f32 re-ranking.
    fn search_rabitq_precision(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<(NodeId, f32)> {
        let index_guard = self.rabitq_index.read();
        let Some(rabitq) = index_guard.as_ref() else {
            return self.search_and_transform(query, k, ef_search);
        };
        let rabitq = Arc::clone(rabitq);
        drop(index_guard);

        let store_guard = self.rabitq_store.read();
        let Some(store) = store_guard.as_ref() else {
            return self.search_and_transform(query, k, ef_search);
        };

        let Some(prepared) = rabitq.prepare_query(query) else {
            return self.search_and_transform(query, k, ef_search);
        };

        let config = RaBitQPrecisionConfig::default();
        let candidates_k = k * config.oversampling_ratio;
        let coarse = self.search_layer_rabitq(&prepared, candidates_k, ef_search, &rabitq, store);

        if coarse.is_empty() {
            return Vec::new();
        }

        let candidate_ids: Vec<NodeId> = coarse.into_iter().map(|(id, _)| id).collect();
        self.rerank_with_exact_f32(query, &candidate_ids, k)
    }

    /// Re-ranks candidate node IDs using exact f32 distances.
    ///
    /// RF-DEDUP: Mirrors `DualPrecisionHnsw::rerank_with_exact_f32`.
    fn rerank_with_exact_f32(
        &self,
        query: &[f32],
        candidate_ids: &[NodeId],
        k: usize,
    ) -> Vec<(NodeId, f32)> {
        let vectors_guard = self.inner.vectors.read();
        let mut reranked: Vec<(NodeId, f32)> = if let Some(vectors) = vectors_guard.as_ref() {
            candidate_ids
                .iter()
                .filter_map(|&node_id| {
                    let vec = vectors.get(node_id)?;
                    let raw_dist = self.inner.compute_distance(query, vec);
                    let final_dist = self.inner.transform_score(raw_dist);
                    Some((node_id, final_dist))
                })
                .collect()
        } else {
            Vec::new()
        };

        reranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        reranked.truncate(k);
        reranked
    }
}
