//! Backend adapter for NativeHnsw to replace hnsw_rs dependency.
//!
//! This module provides:
//! - `NativeNeighbour`: Drop-in replacement for `hnsw_rs::prelude::Neighbour`
//! - `NativeHnswBackend`: Trait for HNSW operations without hnsw_rs dependency
//! - Additional methods for `NativeHnsw` to match backend trait
//! - Parallel insertion using rayon
//!
//! Graph persistence (dump/load) is in [`super::graph_io`].
//! `BatchEfSchedule` is in [`super::batch_schedule`].

use super::batch_schedule::compute_batch_ef_schedule;
use super::distance::DistanceEngine;
use super::graph::{NativeHnsw, NO_ENTRY_POINT};
use super::layer::NodeId;
use crate::distance::DistanceMetric;
use rayon::prelude::*;
use std::path::Path;

// ============================================================================
// NativeHnswBackend Trait - Independent of hnsw_rs
// ============================================================================

/// Trait for HNSW backend operations - independent of `hnsw_rs`.
///
/// This trait mirrors `HnswBackend` but uses our own `NativeNeighbour` type,
/// allowing complete independence from the `hnsw_rs` crate.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to support concurrent access.
pub trait NativeHnswBackend: Send + Sync {
    /// Searches the HNSW graph and returns neighbors with distances.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `ef_search` - Search expansion factor (higher = more accurate, slower)
    fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<NativeNeighbour>;

    /// Inserts a single vector into the HNSW graph.
    ///
    /// # Arguments
    ///
    /// * `data` - Tuple of (vector slice, internal index)
    ///
    /// # Errors
    ///
    /// Returns an error if allocation or insertion fails.
    fn insert(&self, data: (&[f32], usize)) -> crate::error::Result<()>;

    /// Batch parallel insert into the HNSW graph.
    ///
    /// Returns a vector of graph-assigned node IDs, one per input vector,
    /// in the same order as `data`. Callers must reconcile these against
    /// their pre-registered mapping indices.
    ///
    /// # Errors
    ///
    /// Returns an error if any insertion fails.
    fn parallel_insert(&self, data: &[(&[f32], usize)]) -> crate::error::Result<Vec<usize>>;

    /// Sets the index to searching mode after bulk insertions.
    fn set_searching_mode(&mut self, mode: bool);

    /// Dumps the HNSW graph to files for persistence.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail.
    fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()>;

    /// Transforms raw distance to appropriate score based on metric type.
    ///
    /// For Euclidean metric, assumes the input is **squared L2** as produced
    /// by `CachedSimdDistance`. Other distance engines that already return
    /// actual Euclidean distance
    /// should **not** have their results passed through this function, as
    /// it would incorrectly apply `sqrt()` to an already-sqrt'd value.
    fn transform_score(&self, raw_distance: f32) -> f32;

    /// Returns the number of elements in the index.
    fn len(&self) -> usize;

    /// Returns true if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Native neighbour type - drop-in replacement for `hnsw_rs::prelude::Neighbour`.
///
/// This allows `NativeHnsw` to implement `HnswBackend` without depending on `hnsw_rs`.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeNeighbour {
    /// Data index (maps to external ID via `HnswIndex` mappings)
    pub d_id: usize,
    /// Distance from query vector
    pub distance: f32,
}

impl NativeNeighbour {
    /// Creates a new neighbour result.
    #[must_use]
    pub fn new(d_id: usize, distance: f32) -> Self {
        Self { d_id, distance }
    }
}

// ============================================================================
// Extended NativeHnsw methods for HnswBackend compatibility
// ============================================================================

impl<D: DistanceEngine + Send + Sync> NativeHnsw<D> {
    /// Parallel batch insert using rayon.
    ///
    /// Inserts multiple vectors in parallel for better throughput on multi-core systems.
    /// Returns a vector of graph-assigned node IDs, one per input vector in order.
    ///
    /// # Arguments
    ///
    /// * `data` - Slice of (vector reference, internal index) pairs
    ///
    /// # Errors
    ///
    /// Returns an error if any insertion fails.
    ///
    /// # Note
    ///
    /// Graph structure may differ from sequential insertion due to concurrent
    /// neighbor selection. This does not affect search correctness.
    pub fn parallel_insert(&self, data: &[(&[f32], usize)]) -> crate::error::Result<Vec<usize>> {
        // For small batches, sequential is faster due to parallelization overhead
        if data.len() < 100 {
            let mut assigned_ids = Vec::with_capacity(data.len());
            for (vec, _idx) in data {
                assigned_ids.push(self.insert(vec)?);
            }
            return Ok(assigned_ids);
        }

        // Phase A: Batch allocate — stores vectors, assigns layers (single lock scopes)
        let vectors: Vec<&[f32]> = data.iter().map(|(v, _)| *v).collect();
        let (assignments, processed) = self.allocate_batch(&vectors)?;
        if assignments.is_empty() {
            return Ok(Vec::new());
        }

        let first_node = assignments[0].0;
        let connect_start = self.bootstrap_entry_point(&assignments);

        self.connect_batch_chunked(&assignments[connect_start..], &processed, first_node)?;
        self.finalize_batch(&assignments, connect_start);

        // Return the graph-assigned node IDs in input order
        let assigned_ids: Vec<usize> = assignments.iter().map(|(node_id, _)| *node_id).collect();
        Ok(assigned_ids)
    }

    /// Establishes the first node as entry point if the index is empty.
    ///
    /// Returns the number of nodes consumed by bootstrapping (0 or 1).
    /// Consumed nodes are excluded from the parallel connect phase because
    /// they have no valid entry point to search from.
    fn bootstrap_entry_point(&self, assignments: &[(NodeId, usize)]) -> usize {
        if self.entry_point.load(std::sync::atomic::Ordering::Acquire) == NO_ENTRY_POINT {
            let (node_id, layer) = assignments[0];
            self.promote_entry_point(node_id, layer);
            1
        } else {
            0
        }
    }

    /// Final promotion of the highest-layer node and bootstrap count update.
    ///
    /// Called after `connect_batch_chunked` completes. Ensures the global
    /// entry point reflects the best candidate across the entire batch, and
    /// accounts for any bootstrapped node that was not counted by the
    /// chunked phase.
    fn finalize_batch(&self, assignments: &[(NodeId, usize)], connect_start: usize) {
        if let Some(best) = assignments.iter().max_by_key(|(_, layer)| *layer) {
            self.promote_entry_point(best.0, best.1);
        }
        if connect_start > 0 {
            self.count
                .fetch_add(connect_start, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Computes the chunk size for batched Phase B insertion.
    ///
    /// Balances parallelism (larger chunks) against entry-point staleness
    /// (smaller chunks refresh the EP more often). The formula scales
    /// linearly with batch size, clamped to `[1000, 5000]`.
    #[must_use]
    pub(in crate::index::hnsw::native) fn compute_chunk_size(batch_len: usize) -> usize {
        const DEFAULT_CHUNK: usize = 1000;
        const MAX_CHUNK: usize = 5000;
        (batch_len / 50).clamp(DEFAULT_CHUNK, MAX_CHUNK)
    }

    /// Computes the effective `ef_construction` for a batch of the given size.
    ///
    /// For large batches, the full `ef_construction` search budget is wasteful
    /// because the graph scaffold built by earlier vectors already provides
    /// sufficient connectivity for neighbor discovery. Reducing the beam width
    /// proportionally to batch size matches the strategy used by Qdrant and
    /// hnswlib for bulk loading.
    ///
    /// The returned value is always >= `max_connections` to guarantee that
    /// each inserted node can discover enough neighbors for a well-connected
    /// graph.
    ///
    /// Returns `(effective_ef, stagnation_limit)`.
    #[must_use]
    pub(in crate::index::hnsw::native) fn adaptive_ef_for_batch(
        &self,
        batch_size: usize,
    ) -> (usize, usize) {
        let base = self.ef_construction;

        // Conservative scaling: the original 0.25/0.50 reduction destroyed
        // graph quality at 100K+ (recall dropped from 97% to 64%).
        // Malkov & Yashunin 2018 recommends ef_construction >= 2*M;
        // these floors keep ef well above that while still accelerating
        // bulk loads vs single-insert.
        let scale = if batch_size > 50_000 {
            0.60
        } else if batch_size > 10_000 {
            0.75
        } else if batch_size > 1_000 {
            0.85
        } else {
            return (base, 0);
        };

        // Reason: f64 product of two small positive values fits in usize.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (base as f64 * scale) as usize;

        // Floor at 4*M to guarantee adequate neighbor diversity budget.
        let effective_ef = scaled.max(self.max_connections * 4);

        // Stagnation-based early termination: ef/2 gives the beam search
        // enough runway to escape local clusters at scale (was ef/3, which
        // caused premature termination at 100K+).
        let stagnation = effective_ef / 2;

        (effective_ef, stagnation)
    }

    /// Connects nodes in chunks, refreshing the entry point between chunks.
    ///
    /// Each chunk runs `par_iter` over its assignments, then promotes the
    /// highest-layer node and increments the count. This keeps the entry
    /// point fresher than a single monolithic `par_iter` over the entire
    /// batch, improving recall for large insertions.
    ///
    /// For batches > 1K vectors, uses adaptive `ef_construction` reduction
    /// to lower the search budget proportionally, matching the bulk-loading
    /// strategies of Qdrant and hnswlib. Single-vector insert is unaffected.
    ///
    /// # Errors
    ///
    /// Returns an error if any node connection fails.
    /// Connects nodes in chunks with graduated ef\_construction.
    ///
    /// Uses a 3-phase schedule (VAMANA/DiskANN pattern):
    /// - **Phase 1** (first 10%): full ef — builds a quality scaffold
    /// - **Phase 2** (10%-90%): reduced ef (0.5x) — graph is dense enough
    /// - **Phase 3** (last 10%): moderate ef (0.75x) — finalizes connections
    fn connect_batch_chunked(
        &self,
        assignments: &[(NodeId, usize)],
        processed: &[std::borrow::Cow<'_, [f32]>],
        first_node: NodeId,
    ) -> crate::error::Result<()> {
        let chunk_size = Self::compute_chunk_size(assignments.len());
        let schedule = compute_batch_ef_schedule(
            self.ef_construction,
            assignments.len(),
            self.max_connections,
        );
        let mut nodes_connected: usize = 0;

        for chunk in assignments.chunks(chunk_size) {
            let loaded = self.entry_point.load(std::sync::atomic::Ordering::Acquire);
            let ep_id = if loaded == NO_ENTRY_POINT {
                first_node
            } else {
                loaded
            };

            let chunk_offset = nodes_connected;

            chunk.par_iter().enumerate().try_for_each(
                |(i, (node_id, layer))| -> crate::error::Result<()> {
                    let batch_idx = node_id - first_node;
                    let query: &[f32] = &processed[batch_idx];
                    let current_ep = self.greedy_descent_upper_layers(query, *layer, ep_id);
                    let ef = schedule.ef_for_position(chunk_offset + i);
                    let stagnation = ef / 2;
                    self.connect_node_with_ef(*node_id, query, *layer, current_ep, ef, stagnation);
                    Ok(())
                },
            )?;

            if let Some(best) = chunk.iter().max_by_key(|(_, layer)| *layer) {
                self.promote_entry_point(best.0, best.1);
            }
            self.count
                .fetch_add(chunk.len(), std::sync::atomic::Ordering::Relaxed);
            nodes_connected += chunk.len();
        }
        Ok(())
    }

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// For `NativeHnsw`, this is currently a no-op as our implementation
    /// doesn't require mode switching. Kept for API compatibility.
    ///
    /// # Arguments
    ///
    /// * `_mode` - `true` to enable searching mode, `false` to disable
    pub fn set_searching_mode(&mut self, _mode: bool) {
        // No-op for NativeHnsw - our implementation doesn't need mode switching
        // hnsw_rs uses this to optimize internal data structures after bulk insert
    }

    /// Searches and returns results in `NativeNeighbour` format.
    ///
    /// This is the HnswBackend-compatible search method.
    #[must_use]
    pub fn search_neighbours(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<NativeNeighbour> {
        self.search(query, k, ef_search)
            .into_iter()
            .map(|(id, dist)| NativeNeighbour::new(id, dist))
            .collect()
    }

    /// Transforms raw distance to appropriate score based on metric type.
    ///
    /// - **Cosine**: `(1.0 - distance).clamp(0.0, 1.0)` (similarity in `[0,1]`)
    /// - **Euclidean**: `sqrt(raw_distance)` — the search loop stores squared L2
    ///   to skip redundant sqrt during traversal; this restores the actual
    ///   Euclidean distance for user-visible scores.
    /// - **Hamming**/**Jaccard**: raw distance (lower is better)
    /// - **DotProduct**: `-distance` (negated for consistency)
    #[must_use]
    pub fn transform_score(&self, raw_distance: f32) -> f32 {
        match self.distance.metric() {
            DistanceMetric::Cosine => (1.0 - raw_distance).clamp(0.0, 1.0),
            // Reason: CachedSimdDistance stores squared L2 during HNSW traversal
            // to avoid per-comparison sqrt. Apply sqrt here on the final k results.
            DistanceMetric::Euclidean => raw_distance.sqrt(),
            DistanceMetric::Hamming | DistanceMetric::Jaccard => raw_distance,
            DistanceMetric::DotProduct => -raw_distance,
        }
    }
}

// ============================================================================
// NativeHnswBackend implementation for NativeHnsw
// ============================================================================

impl<D: DistanceEngine + Send + Sync> NativeHnswBackend for NativeHnsw<D> {
    fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<NativeNeighbour> {
        self.search_neighbours(query, k, ef_search)
    }

    fn insert(&self, data: (&[f32], usize)) -> crate::error::Result<()> {
        let (vector, expected_idx) = data;
        let assigned_id = self.insert(vector)?;
        if assigned_id != expected_idx {
            tracing::warn!(
                "NativeHnsw node_id mismatch: expected {expected_idx}, got {assigned_id}"
            );
        }
        Ok(())
    }

    fn parallel_insert(&self, data: &[(&[f32], usize)]) -> crate::error::Result<Vec<usize>> {
        NativeHnsw::parallel_insert(self, data)
    }

    fn set_searching_mode(&mut self, mode: bool) {
        NativeHnsw::set_searching_mode(self, mode);
    }

    fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()> {
        NativeHnsw::file_dump(self, path, basename)
    }

    fn transform_score(&self, raw_distance: f32) -> f32 {
        NativeHnsw::transform_score(self, raw_distance)
    }

    fn len(&self) -> usize {
        NativeHnsw::len(self)
    }
}
