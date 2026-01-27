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

#![allow(dead_code)] // Will be used when feature flag is enabled
#![allow(clippy::cast_precision_loss)] // Test code uses simple casts

use super::native_inner::NativeHnswInner;
use super::params::{HnswParams, SearchQuality};
use super::sharded_mappings::ShardedMappings;
use super::sharded_vectors::ShardedVectors;
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
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
    mappings: ShardedMappings,
    vectors: ShardedVectors,
    enable_vector_storage: bool,
    params: HnswParams,
}

impl NativeHnswIndex {
    /// Creates a new native HNSW index with auto-tuned parameters.
    #[must_use]
    pub fn new(dimension: usize, metric: DistanceMetric) -> Self {
        Self::with_params(dimension, metric, HnswParams::auto(dimension))
    }

    /// Creates a new native HNSW index with custom parameters.
    #[must_use]
    pub fn with_params(dimension: usize, metric: DistanceMetric, params: HnswParams) -> Self {
        let inner = NativeHnswInner::new(
            metric,
            params.max_connections,
            params.max_elements,
            params.ef_construction,
        );

        Self {
            dimension,
            metric,
            inner: RwLock::new(inner),
            mappings: ShardedMappings::new(),
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage: true,
            params,
        }
    }

    /// Creates a turbo mode index for maximum insert throughput.
    #[must_use]
    pub fn new_turbo(dimension: usize, metric: DistanceMetric) -> Self {
        Self::with_params(dimension, metric, HnswParams::turbo())
    }

    /// Creates an index optimized for fast inserts (no vector storage).
    #[must_use]
    pub fn new_fast_insert(dimension: usize, metric: DistanceMetric) -> Self {
        let mut index = Self::new(dimension, metric);
        index.enable_vector_storage = false;
        index
    }

    /// Saves the index to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;

        // Save HNSW graph
        let inner = self.inner.read();
        inner.file_dump(path, "native_hnsw")?;

        // Save mappings
        let mappings_path = path.join("native_mappings.bin");
        let file = std::fs::File::create(mappings_path)?;
        let writer = std::io::BufWriter::new(file);

        let (id_to_idx, idx_to_id, next_idx) = self.mappings.as_parts();
        bincode::serialize_into(writer, &(id_to_idx, idx_to_id, next_idx))
            .map_err(std::io::Error::other)?;

        // Save metadata
        let meta_path = path.join("native_meta.bin");
        let meta_file = std::fs::File::create(meta_path)?;
        let meta_writer = std::io::BufWriter::new(meta_file);
        bincode::serialize_into(
            meta_writer,
            &(
                self.dimension,
                self.metric as u8,
                self.enable_vector_storage,
            ),
        )
        .map_err(std::io::Error::other)?;

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
        let path = path.as_ref();

        // Load metadata
        let meta_path = path.join("native_meta.bin");
        let meta_file = std::fs::File::open(meta_path)?;
        let meta_reader = std::io::BufReader::new(meta_file);
        let (dimension, metric_u8, enable_vector_storage): (usize, u8, bool) =
            bincode::deserialize_from(meta_reader).map_err(std::io::Error::other)?;

        // Match enum order: Cosine=0, Euclidean=1, DotProduct=2, Hamming=3, Jaccard=4
        let metric = match metric_u8 {
            0 => DistanceMetric::Cosine,
            1 => DistanceMetric::Euclidean,
            2 => DistanceMetric::DotProduct,
            3 => DistanceMetric::Hamming,
            4 => DistanceMetric::Jaccard,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Unknown distance metric",
                ))
            }
        };

        // Load HNSW graph
        let inner = NativeHnswInner::file_load(path, "native_hnsw", metric)?;

        // Load mappings
        let mappings_path = path.join("native_mappings.bin");
        let file = std::fs::File::open(mappings_path)?;
        let reader = std::io::BufReader::new(file);

        let (id_to_idx, idx_to_id, next_idx): (
            std::collections::HashMap<u64, usize>,
            std::collections::HashMap<usize, u64>,
            usize,
        ) = bincode::deserialize_from(reader).map_err(std::io::Error::other)?;

        let mappings = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);

        Ok(Self {
            dimension,
            metric,
            inner: RwLock::new(inner),
            mappings,
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage,
            params: HnswParams::auto(dimension),
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

    /// Returns the number of vectors in the index.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Returns true if the index is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }

    /// Searches for the k nearest neighbors.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        self.search_with_quality(query, k, SearchQuality::Balanced)
    }

    /// Searches with a specific quality profile.
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<(u64, f32)> {
        let ef_search = quality.ef_search(k);
        let inner = self.inner.read();
        let neighbors = inner.search(query, k, ef_search);

        neighbors
            .into_iter()
            .filter_map(|n| {
                self.mappings.get_id(n.d_id).map(|id| {
                    let score = inner.transform_score(n.distance);
                    (id, score)
                })
            })
            .collect()
    }

    /// Inserts a single vector.
    ///
    /// # Panics
    ///
    /// Panics if internal ID allocation fails (should never happen in practice).
    pub fn insert(&self, id: u64, vector: &[f32]) {
        // Register ID and get internal index (or get existing)
        let internal_idx = self.mappings.register(id).unwrap_or_else(|| {
            self.mappings
                .get_idx(id)
                .expect("ID should exist after register attempt")
        });
        self.inner.read().insert((vector, internal_idx));

        if self.enable_vector_storage {
            self.vectors.insert(internal_idx, vector);
        }
    }

    /// Batch insert multiple vectors.
    ///
    /// # Panics
    ///
    /// Panics if internal ID allocation fails (should never happen in practice).
    pub fn insert_batch(&self, items: &[(u64, Vec<f32>)]) {
        let data: Vec<(Vec<f32>, usize)> = items
            .iter()
            .map(|(id, vec)| {
                let idx = self
                    .mappings
                    .register(*id)
                    .unwrap_or_else(|| self.mappings.get_idx(*id).expect("ID should exist"));
                (vec.clone(), idx)
            })
            .collect();

        let refs: Vec<(&Vec<f32>, usize)> = data.iter().map(|(v, i)| (v, *i)).collect();
        self.inner.read().parallel_insert(&refs);

        if self.enable_vector_storage {
            for (vec, idx) in data {
                self.vectors.insert(idx, &vec);
            }
        }
    }

    /// Removes a vector by ID (marks as deleted in mappings).
    pub fn remove(&self, id: u64) -> bool {
        self.mappings.remove(id).is_some()
    }

    /// Sets searching mode (no-op for native implementation).
    ///
    /// This method exists for API compatibility with `HnswIndex`.
    /// The native implementation doesn't require mode switching.
    #[allow(clippy::unused_self)]
    pub fn set_searching_mode(&self) {
        // No-op - native implementation doesn't need this
    }

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
        self.insert_batch(items.as_slice());
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
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<Vec<(u64, f32)>> {
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
    pub fn brute_force_search_parallel(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        use rayon::prelude::*;

        // Collect all vectors for parallel iteration
        let vectors_snapshot = self.vectors.collect_for_parallel();

        if vectors_snapshot.is_empty() {
            return Vec::new();
        }

        // Get distance calculator from inner
        let inner = self.inner.read();

        // Compute distances in parallel
        let mut results: Vec<(u64, f32)> = vectors_snapshot
            .par_iter()
            .filter_map(|(idx, vec)| {
                let id = self.mappings.get_id(*idx)?;
                let distance = inner.compute_distance(query, vec);
                Some((id, distance))
            })
            .collect();

        // Sort by distance (ascending for most metrics)
        self.metric.sort_results(&mut results);

        results.truncate(k);
        results
    }
}

impl VectorIndex for NativeHnswIndex {
    fn insert(&self, id: u64, vector: &[f32]) {
        NativeHnswIndex::insert(self, id, vector);
    }

    fn remove(&self, id: u64) -> bool {
        NativeHnswIndex::remove(self, id)
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
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
