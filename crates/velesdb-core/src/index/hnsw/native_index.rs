//! Native HNSW index - standalone implementation without `hnsw_rs` dependency.
//!
//! This module provides `NativeHnswIndex`, a complete HNSW index using our native
//! implementation. It can be used as a drop-in replacement for `HnswIndex` when
//! the `native-hnsw` feature is enabled.
//!
//! # Feature Flag
//!
//! Enable with `native-hnsw` feature in `Cargo.toml`:
//! ```toml
//! [dependencies]
//! velesdb-core = { version = "0.8", features = ["native-hnsw"] }
//! ```

use super::native_inner::NativeHnswInner;
use super::params::{HnswParams, SearchQuality};
use super::sharded_mappings::ShardedMappings;
use super::sharded_vectors::ShardedVectors;
use super::upsert::{self, UpsertResult};
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use crate::scored_result::ScoredResult;
use crate::validation::validate_dimension_match;
use parking_lot::RwLock;
use std::path::Path;

/// Native HNSW index for efficient approximate nearest neighbor search.
///
/// This is a standalone implementation that doesn't depend on `hnsw_rs`.
/// It provides the same API as `HnswIndex` for easy migration.
///
/// # Performance Characteristics
///
/// - **Recall**: ~99% parity with `hnsw_rs` (verified by parity tests)
/// - **Insert**: Comparable performance with SIMD distance calculations
/// - **Search**: Optimized with `SimdDistance` engine
/// - **Persistence**: Native binary format (not compatible with `hnsw_rs` format)
pub struct NativeHnswIndex {
    dimension: usize,
    metric: DistanceMetric,
    inner: RwLock<NativeHnswInner>,
    pub(crate) mappings: ShardedMappings,
    vectors: ShardedVectors,
    enable_vector_storage: bool,
    #[allow(dead_code)] // Retained for future vacuum/rebuild operations
    params: HnswParams,
}

impl NativeHnswIndex {
    /// Creates a new native HNSW index with auto-tuned parameters.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new(dimension: usize, metric: DistanceMetric) -> crate::error::Result<Self> {
        Self::with_params(dimension, metric, HnswParams::auto(dimension))
    }

    /// Creates a new native HNSW index with custom parameters.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn with_params(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
    ) -> crate::error::Result<Self> {
        let inner = NativeHnswInner::new_with_options(
            metric,
            params.max_connections,
            params.max_elements,
            params.ef_construction,
            dimension,
            params.storage_mode,
            params.alpha,
        )?;

        Ok(Self {
            dimension,
            metric,
            inner: RwLock::new(inner),
            mappings: ShardedMappings::new(),
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage: true,
            params,
        })
    }

    /// Creates a turbo mode index for maximum insert throughput.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new_turbo(dimension: usize, metric: DistanceMetric) -> crate::error::Result<Self> {
        Self::with_params(dimension, metric, HnswParams::turbo())
    }

    /// Creates an index optimized for fast inserts (no vector storage).
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new_fast_insert(dimension: usize, metric: DistanceMetric) -> crate::error::Result<Self> {
        let mut index = Self::new(dimension, metric)?;
        index.enable_vector_storage = false;
        Ok(index)
    }

    /// Saves the index to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        use super::persistence::{self, HnswMappingsData, HnswMeta};

        let path = path.as_ref();
        std::fs::create_dir_all(path)?;

        // Save HNSW graph
        let inner = self.inner.read();
        inner.file_dump(path, "native_hnsw")?;

        // Save mappings
        let (id_to_idx, idx_to_id, next_idx) = self.mappings.as_parts();
        persistence::save_mappings(
            path,
            &HnswMappingsData {
                id_to_idx,
                idx_to_id,
                next_idx,
            },
        )?;

        // Save or clean up vectors (shared helper)
        persistence::save_or_cleanup_vectors(path, self.enable_vector_storage, &self.vectors)?;

        // Save metadata
        persistence::save_meta(
            path,
            &HnswMeta {
                dimension: self.dimension,
                metric: self.metric,
                enable_vector_storage: self.enable_vector_storage,
                storage_mode: self.inner.read().storage_mode(),
            },
        )?;

        Ok(())
    }

    /// Loads the index from disk.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the index directory
    /// * `_dimension` - Ignored (read from metadata) - for API compatibility
    /// * `_metric` - Ignored (read from metadata) - for API compatibility
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail or data is corrupted.
    pub fn load<P: AsRef<Path>>(
        path: P,
        _dimension: usize,
        _metric: DistanceMetric,
    ) -> std::io::Result<Self> {
        use super::persistence;

        let path = path.as_ref();

        let meta = persistence::load_meta(path)?;

        // Load HNSW graph (with storage mode for RaBitQ backend support)
        let inner = NativeHnswInner::file_load_with_storage_mode(
            path,
            "native_hnsw",
            meta.metric,
            meta.dimension,
            meta.storage_mode,
        )?;

        // Load mappings
        let mappings_data = persistence::load_mappings(path)?;
        let mappings = ShardedMappings::from_parts(
            mappings_data.id_to_idx,
            mappings_data.idx_to_id,
            mappings_data.next_idx,
        );

        // Load vectors (gracefully disables if file missing)
        let (vectors, enable_vector_storage) = persistence::load_vectors_or_disable(path, &meta)?;

        Ok(Self {
            dimension: meta.dimension,
            metric: meta.metric,
            inner: RwLock::new(inner),
            mappings,
            vectors,
            enable_vector_storage,
            params: HnswParams::auto(meta.dimension),
        })
    }

    /// Returns the dimension of vectors in this index.
    #[inline]
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the distance metric used by this index.
    #[inline]
    #[must_use]
    pub fn metric(&self) -> DistanceMetric {
        self.metric
    }

    /// Returns the number of live vectors in the index.
    ///
    /// This reflects the mapping count (excluding tombstones), consistent
    /// with `HnswIndex::len()`.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    /// Returns true if the index contains no live vectors.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }

    /// Returns whether vector storage is enabled.
    #[inline]
    #[must_use]
    pub fn has_vector_storage(&self) -> bool {
        self.enable_vector_storage
    }

    /// Searches for the k nearest neighbors.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        self.search_with_quality(query, k, SearchQuality::Balanced)
    }

    /// Searches with a specific quality profile.
    #[must_use]
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<ScoredResult> {
        let ef_search = quality.ef_search(k);
        let inner = self.inner.read();
        let neighbors = inner.search(query, k, ef_search);

        neighbors
            .into_iter()
            .filter_map(|(node_id, raw_dist)| {
                self.mappings.get_id(node_id).map(|id| {
                    let score = inner.transform_score(raw_dist);
                    ScoredResult::new(id, score)
                })
            })
            .collect()
    }

    /// Registers an ID with upsert semantics and cleans up stale vector data.
    ///
    /// Returns an [`UpsertResult`] with the new internal index and optional
    /// old index for rollback. If the ID already existed, the old mapping is
    /// replaced and the stale sidecar vector is removed.
    #[must_use]
    fn upsert_mapping(&self, id: u64) -> UpsertResult {
        upsert::upsert_mapping(
            &self.mappings,
            &self.vectors,
            self.enable_vector_storage,
            id,
        )
    }

    /// Rolls back a mapping upsert after a failed graph insertion.
    fn rollback_upsert(&self, id: u64, result: &UpsertResult) {
        upsert::rollback_upsert(&self.mappings, id, result);
    }

    /// Inserts or updates a single vector (upsert semantics).
    ///
    /// If `id` already exists, the old mapping is atomically replaced and
    /// stale vector data is cleaned up. The old HNSW graph node becomes a
    /// tombstone, filtered out during search via the reverse mapping.
    ///
    /// # Errors
    ///
    /// Returns an error if allocation or graph insertion fails.
    pub fn insert(&self, id: u64, vector: &[f32]) -> crate::error::Result<()> {
        // Validate dimension BEFORE upsert_mapping to avoid destroying the old
        // mapping for an invalid vector (Devin review finding).
        validate_dimension_match(self.dimension, vector.len())?;

        let result = self.upsert_mapping(id);

        if let Err(e) = self.inner.read().insert((vector, result.idx)) {
            self.rollback_upsert(id, &result);
            return Err(e);
        }

        if self.enable_vector_storage {
            self.vectors.insert(result.idx, vector);
        }
        Ok(())
    }

    /// Batch insert or update multiple vectors (upsert semantics).
    ///
    /// For each item, the mapping is atomically replaced if the ID already
    /// exists. Stale vector data is cleaned up before the graph insertion.
    ///
    /// On graph insertion failure, all IDs in this batch are removed from
    /// mappings. For replaced IDs, the old mapping is already gone — the
    /// caller should retry the full batch.
    ///
    /// # Errors
    ///
    /// Returns an error if any insertion fails.
    pub fn insert_batch(&self, items: &[(u64, Vec<f32>)]) -> crate::error::Result<()> {
        // Validate all dimensions upfront before any upsert_mapping side effects.
        for (_id, vec) in items {
            validate_dimension_match(self.dimension, vec.len())?;
        }

        let ids: Vec<u64> = items.iter().map(|(id, _)| *id).collect();
        let upsert_results = upsert::upsert_mapping_batch(
            &self.mappings,
            &self.vectors,
            self.enable_vector_storage,
            &ids,
        );

        let mut data: Vec<(&[f32], usize)> = Vec::with_capacity(items.len());
        let mut rollback_info: Vec<(u64, UpsertResult)> = Vec::with_capacity(items.len());

        for ((id, vec), result) in items.iter().zip(upsert_results) {
            data.push((vec.as_slice(), result.idx));
            rollback_info.push((*id, result));
        }

        let assigned_ids = match self.inner.read().parallel_insert(&data) {
            Ok(ids) => ids,
            Err(e) => {
                // Reverse order: undo last upsert first so duplicate-ID chains
                // restore correctly (each rollback depends on the previous state).
                for (id, result) in rollback_info.iter().rev() {
                    self.rollback_upsert(*id, result);
                }
                return Err(e);
            }
        };

        // Reconcile mappings: the graph may assign different node IDs than
        // upsert_mapping_batch pre-registered. Fix mismatches.
        let storage_ids = self.reconcile_batch_mappings(&rollback_info, &assigned_ids);

        if self.enable_vector_storage {
            for (vec_slice, idx) in data.iter().map(|(v, _)| *v).zip(storage_ids) {
                self.vectors.insert(idx, vec_slice);
            }
        }

        Ok(())
    }

    /// Reconciles pre-registered mapping indices with graph-assigned node IDs.
    ///
    /// For each item, if the graph-assigned ID differs from the pre-registered
    /// index, removes the stale reverse mapping and restores the correct one.
    /// Returns the authoritative storage index for each item.
    fn reconcile_batch_mappings(
        &self,
        rollback_info: &[(u64, UpsertResult)],
        assigned_ids: &[usize],
    ) -> Vec<usize> {
        let mut storage_ids = Vec::with_capacity(assigned_ids.len());
        for (assigned_id, (ext_id, result)) in assigned_ids.iter().zip(rollback_info) {
            if *assigned_id == result.idx {
                storage_ids.push(result.idx);
            } else {
                self.mappings.remove_reverse(result.idx);
                self.mappings.restore(*ext_id, *assigned_id);
                storage_ids.push(*assigned_id);
            }
        }
        storage_ids
    }

    /// Removes a vector by ID (soft delete).
    ///
    /// Removes the ID from mappings and cleans up stored vector data.
    /// The HNSW graph node becomes a tombstone, filtered out during search.
    pub fn remove(&self, id: u64) -> bool {
        if let Some(old_idx) = self.mappings.remove(id) {
            if self.enable_vector_storage {
                self.vectors.remove(old_idx);
            }
            true
        } else {
            false
        }
    }

    /// Sets searching mode (no-op for native implementation).
    ///
    /// This method exists for API compatibility with `HnswIndex`.
    /// The native implementation doesn't require mode switching.
    #[allow(clippy::unused_self)]
    pub fn set_searching_mode(&self) {}

    /// Parallel batch insert - API compatible with `HnswIndex`.
    ///
    /// # Returns
    ///
    /// Number of vectors inserted.
    #[allow(clippy::needless_pass_by_value)]
    pub fn insert_batch_parallel<I>(&self, items: I) -> usize
    where
        I: IntoIterator<Item = (u64, Vec<f32>)>,
    {
        let items: Vec<_> = items.into_iter().collect();
        let count = items.len();
        if let Err(e) = self.insert_batch(items.as_slice()) {
            tracing::error!("insert_batch_parallel failed: {e}");
            return 0;
        }
        count
    }

    /// Batch search with parallel execution.
    ///
    /// # Arguments
    ///
    /// * `queries` - Slice of query vector slices
    /// * `k` - Number of nearest neighbors per query
    /// * `quality` - Search quality profile
    ///
    /// # Returns
    ///
    /// Vector of results for each query.
    #[must_use]
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<Vec<ScoredResult>> {
        use rayon::prelude::*;

        queries
            .par_iter()
            .map(|q| self.search_with_quality(q, k, quality))
            .collect()
    }

    /// Brute-force exact nearest neighbor search with parallel execution.
    ///
    /// Computes distances to all vectors in the index and returns the k nearest.
    /// This provides 100% recall but O(n) complexity.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Number of nearest neighbors to return
    ///
    /// # Returns
    ///
    /// Vector of (id, distance) tuples sorted by distance.
    ///
    /// # Use Cases
    ///
    /// - **Recall validation**: Compare HNSW results against brute-force
    /// - **Small datasets**: When n < 10k, brute-force may be faster
    /// - **Critical accuracy**: When 100% recall is required
    #[must_use]
    pub fn brute_force_search_parallel(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        use rayon::prelude::*;

        let vectors_snapshot = self.vectors.collect_for_parallel();

        if vectors_snapshot.is_empty() {
            return Vec::new();
        }

        let inner = self.inner.read();

        let mut results: Vec<ScoredResult> = vectors_snapshot
            .par_iter()
            .filter_map(|(idx, vec)| {
                let id = self.mappings.get_id(*idx)?;
                let raw_distance = inner.compute_distance(query, vec);
                // Reason: compute_distance returns squared L2 for Euclidean
                // (CachedSimdDistance optimization). Apply transform_score to
                // restore actual Euclidean distance for user-visible scores.
                let score = inner.transform_score(raw_distance);
                Some(ScoredResult::new(id, score))
            })
            .collect();

        self.metric.sort_scored_results(&mut results);

        results.truncate(k);
        results
    }
}

impl VectorIndex for NativeHnswIndex {
    fn insert(&self, id: u64, vector: &[f32]) {
        if let Err(e) = NativeHnswIndex::insert(self, id, vector) {
            tracing::error!("NativeHnswIndex::insert failed for id={id}: {e}");
        }
    }

    fn remove(&self, id: u64) -> bool {
        NativeHnswIndex::remove(self, id)
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        NativeHnswIndex::search(self, query, k)
    }

    fn len(&self) -> usize {
        NativeHnswIndex::len(self)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

// ============================================================================
// Tests moved to native_index_tests.rs per project rules
