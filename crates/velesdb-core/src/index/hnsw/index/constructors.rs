//! HnswIndex constructors and initialization methods.

use super::{HnswIndex, HnswInner};
use crate::distance::DistanceMetric;
use crate::index::hnsw::params::HnswParams;
use crate::index::hnsw::sharded_mappings::ShardedMappings;
use crate::index::hnsw::sharded_vectors::ShardedVectors;
use parking_lot::RwLock;
use std::mem::ManuallyDrop;
use std::path::Path;

impl HnswIndex {
    /// Creates a new HNSW index with auto-tuned parameters based on dimension.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension (e.g., 768 for OpenAI embeddings)
    /// * `metric` - Distance metric for similarity computation
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// let index = HnswIndex::new(768, DistanceMetric::Cosine);
    /// ```
    #[must_use]
    pub fn new(dimension: usize, metric: DistanceMetric) -> Self {
        let params = HnswParams::auto(dimension);
        Self::with_params(dimension, metric, params)
    }

    /// Creates a new HNSW index optimized for maximum insert throughput.
    ///
    /// # Performance
    ///
    /// - **~2x faster inserts** than `new()` (vectors not stored for re-ranking)
    /// - **~50% less memory** (no ShardedVectors duplication)
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
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// // Fast insert mode: 2x faster, 50% less memory
    /// let index = HnswIndex::new_fast_insert(768, DistanceMetric::Cosine);
    /// ```
    #[must_use]
    pub fn new_fast_insert(dimension: usize, metric: DistanceMetric) -> Self {
        let params = HnswParams::auto(dimension);
        Self::with_params_internal(dimension, metric, params, false)
    }

    /// Creates a new HNSW index optimized for high performance.
    ///
    /// # Parameters
    ///
    /// Uses auto-tuned parameters for the dimension, plus:
    /// - Higher ef_construction for better graph quality
    /// - Optimized for modern hardware
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// // Turbo mode: auto-tuned for high-performance workloads
    /// let index = HnswIndex::new_turbo(768, DistanceMetric::Cosine);
    /// ```
    #[must_use]
    pub fn new_turbo(dimension: usize, metric: DistanceMetric) -> Self {
        let mut params = HnswParams::auto(dimension);
        // Turbo: increase ef_construction by 50% for better graph quality
        params.ef_construction = (params.ef_construction * 3) / 2;
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
    /// let index = HnswIndex::with_params(768, DistanceMetric::Cosine, params);
    /// ```
    #[must_use]
    pub fn with_params(dimension: usize, metric: DistanceMetric, params: HnswParams) -> Self {
        Self::with_params_internal(dimension, metric, params, true)
    }

    /// Internal constructor with vector storage toggle.
    fn with_params_internal(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
        enable_vector_storage: bool,
    ) -> Self {
        let inner = HnswInner::new(
            metric,
            params.max_connections,
            params.max_elements,
            params.ef_construction,
        );

        let mappings = ShardedMappings::with_capacity(params.max_elements);

        Self {
            dimension,
            metric,
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings,
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage,
            io_holder: None,
        }
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
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    /// use velesdb_core::index::hnsw::HnswParams;
    ///
    /// // Full control: custom params + fast insert mode
    /// let params = HnswParams::auto(768);
    /// let index = HnswIndex::with_params_full(768, DistanceMetric::Cosine, params, false);
    /// ```
    #[must_use]
    pub fn with_params_full(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
        enable_vector_storage: bool,
    ) -> Self {
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
    ) -> Result<Self, std::io::Error> {
        let path = path.as_ref();

        // Load metadata
        let meta_path = path.join("native_meta.bin");
        let meta_file = std::fs::File::open(&meta_path)?;
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
        let inner = HnswInner::file_load(path, "native_hnsw", metric)?;

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
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings,
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage,
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
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), std::io::Error> {
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
