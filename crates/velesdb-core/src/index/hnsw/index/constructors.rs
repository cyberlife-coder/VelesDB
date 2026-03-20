//! HnswIndex constructors and initialization methods.

use super::{HnswIndex, HnswInner};
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::index::hnsw::params::HnswParams;
use crate::index::hnsw::sharded_mappings::ShardedMappings;
use crate::index::hnsw::sharded_vectors::ShardedVectors;
use parking_lot::RwLock;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::sync::atomic::AtomicU64;

impl HnswIndex {
    /// Creates a new HNSW index with auto-tuned parameters based on dimension.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension (e.g., 768 for OpenAI embeddings)
    /// * `metric` - Distance metric for similarity computation
    ///
    /// # Errors
    ///
    /// Returns an error if the HNSW graph allocation fails (invalid dimension
    /// or insufficient memory).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// let index = HnswIndex::new(768, DistanceMetric::Cosine)?;
    /// ```
    pub fn new(dimension: usize, metric: DistanceMetric) -> Result<Self> {
        let params = HnswParams::auto(dimension);
        Self::with_params(dimension, metric, params)
    }

    /// Creates a new HNSW index optimized for fast insert throughput.
    ///
    /// # Performance
    ///
    /// - **~2-3x faster inserts** than `new()` (M/2, ef/2 + no vector storage)
    /// - **~50% less memory** (no `ShardedVectors` duplication)
    /// - **Recall**: ~90% (vs ≥95% with standard params)
    ///
    /// # Limitations
    ///
    /// - No SIMD re-ranking support (`search_with_rerank` falls back to standard search)
    /// - No brute-force search (`search_brute_force` returns empty)
    /// - Cannot `vacuum()` the index (returns error)
    ///
    /// # Use Cases
    ///
    /// - High-velocity streaming data
    /// - Large-scale indexing where recall is more important than perfect precision
    /// - Memory-constrained environments
    ///
    /// # Errors
    ///
    /// Returns an error if the HNSW graph allocation fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// // Fast insert mode: 2x faster, 50% less memory
    /// let index = HnswIndex::new_fast_insert(768, DistanceMetric::Cosine)?;
    /// ```
    pub fn new_fast_insert(dimension: usize, metric: DistanceMetric) -> Result<Self> {
        let params = HnswParams::fast_indexing(dimension);
        Self::with_params_internal(dimension, metric, params, false)
    }

    /// Creates a new HNSW index optimized for maximum insert throughput.
    ///
    /// # Trade-offs
    ///
    /// - **~3-5x faster inserts** than `new()` (M=12, ef=100 vs M=32, ef=400)
    /// - **Recall**: ~85% (vs ≥95% with standard params)
    /// - **Best for**: Bulk loading, development, benchmarking
    ///
    /// After bulk loading, consider rebuilding with higher params for production.
    ///
    /// # Errors
    ///
    /// Returns an error if the HNSW graph allocation fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// // Turbo mode: M=12, ef=100 for maximum insert speed
    /// let index = HnswIndex::new_turbo(768, DistanceMetric::Cosine)?;
    /// ```
    pub fn new_turbo(dimension: usize, metric: DistanceMetric) -> Result<Self> {
        let params = HnswParams::turbo();
        Self::with_params(dimension, metric, params)
    }

    /// Creates a new HNSW index with custom parameters.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `params` - Custom HNSW parameters
    ///
    /// # Errors
    ///
    /// Returns an error if the HNSW graph allocation fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    /// use velesdb_core::index::hnsw::HnswParams;
    ///
    /// let params = HnswParams {
    ///     max_connections: 32,
    ///     ef_construction: 400,
    ///     max_elements: 100_000,
    /// };
    /// let index = HnswIndex::with_params(768, DistanceMetric::Cosine, params)?;
    /// ```
    pub fn with_params(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
    ) -> Result<Self> {
        Self::with_params_internal(dimension, metric, params, true)
    }

    /// Internal constructor with vector storage toggle.
    fn with_params_internal(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
        enable_vector_storage: bool,
    ) -> Result<Self> {
        let inner = HnswInner::new(
            metric,
            params.max_connections,
            params.max_elements,
            params.ef_construction,
            dimension,
        )?;

        let mappings = ShardedMappings::with_capacity(params.max_elements);

        Ok(Self {
            dimension,
            metric,
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings,
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage,
            rerank_latency_target_us: AtomicU64::new(0),
            rerank_latency_ema_us: AtomicU64::new(0),
            io_holder: None,
        })
    }

    /// Creates a new HNSW index with fully customized parameters.
    ///
    /// This is the most flexible constructor, allowing control over all aspects.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `params` - Custom HNSW parameters
    /// * `enable_vector_storage` - Whether to store vectors for re-ranking
    ///
    /// # Errors
    ///
    /// Returns an error if the HNSW graph allocation fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    /// use velesdb_core::index::hnsw::HnswParams;
    ///
    /// // Full control: custom params + fast insert mode
    /// let params = HnswParams::auto(768);
    /// let index = HnswIndex::with_params_full(768, DistanceMetric::Cosine, params, false)?;
    /// ```
    pub fn with_params_full(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
        enable_vector_storage: bool,
    ) -> Result<Self> {
        Self::with_params_internal(dimension, metric, params, enable_vector_storage)
    }

    /// Loads an HNSW index from disk.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the index directory
    /// * `dimension` - Expected vector dimension (for API compatibility, read from metadata)
    /// * `metric` - Distance metric (for API compatibility, read from metadata)
    ///
    /// # Errors
    ///
    /// Returns an error if the file doesn't exist or is corrupted.
    pub fn load<P: AsRef<Path>>(
        path: P,
        _dimension: usize,
        _metric: DistanceMetric,
    ) -> std::result::Result<Self, std::io::Error> {
        use crate::index::hnsw::persistence;

        let path = path.as_ref();

        let meta = persistence::load_meta(path)?;

        // Load HNSW graph
        let inner = HnswInner::file_load(path, "native_hnsw", meta.metric, meta.dimension)?;

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
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings,
            vectors,
            enable_vector_storage,
            rerank_latency_target_us: AtomicU64::new(0),
            rerank_latency_ema_us: AtomicU64::new(0),
            io_holder: None,
        })
    }

    /// Saves the HNSW index to disk.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the index directory
    ///
    /// # Errors
    ///
    /// Returns an error if the write fails.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> std::result::Result<(), std::io::Error> {
        use crate::index::hnsw::persistence::{self, HnswMappingsData, HnswMeta};

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
            },
        )?;

        Ok(())
    }

    /// Returns the vector dimension.
    #[inline]
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the distance metric.
    #[inline]
    #[must_use]
    pub fn metric(&self) -> DistanceMetric {
        self.metric
    }

    /// Returns the number of vectors in the index.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    /// Returns true if the index is empty.
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
}
