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
    ///
    /// Honours `params.storage_mode`: `RaBitQ` selects the binary-traversal
    /// backend; every other mode uses the Standard f32 backend (SQ8/Binary
    /// quantized caches live at the collection layer, not in this index).
    fn with_params_internal(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
        enable_vector_storage: bool,
    ) -> Result<Self> {
        let inner = HnswInner::new_with_storage_mode(
            metric,
            params.max_connections,
            params.max_elements,
            params.ef_construction,
            dimension,
            params.storage_mode,
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
    /// Respects the persisted `meta.storage_mode` (a `RaBitQ` index reloads
    /// with the `RaBitQ` backend) and, when the backend is `RaBitQ`, installs
    /// the trained quantizer from `<path>/rabitq.idx` if present.
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
        Self::load_inner(path.as_ref(), None)
    }

    /// Loads an HNSW index honouring a collection-level storage mode.
    ///
    /// The backend mode is the persisted `meta.storage_mode`, upgraded to
    /// `RaBitQ` when `desired_mode` requests it. The upgrade covers the case
    /// where `TRAIN QUANTIZER 'rabitq'` flipped the collection storage mode
    /// AFTER the last index save (the persisted meta still says `Full`).
    ///
    /// # Errors
    ///
    /// Returns an error if the file doesn't exist or is corrupted.
    pub(crate) fn load_with_storage_mode<P: AsRef<Path>>(
        path: P,
        desired_mode: crate::StorageMode,
    ) -> std::result::Result<Self, std::io::Error> {
        Self::load_inner(path.as_ref(), Some(desired_mode))
    }

    /// Shared load pipeline for [`Self::load`] / [`Self::load_with_storage_mode`].
    ///
    /// When the resulting backend is `RaBitQ`, the trained quantizer persisted
    /// at `<path>/rabitq.idx` is installed, re-encoding every loaded vector in
    /// `NodeId` order — an O(n·d) cost at open, same class as gap recovery.
    fn load_inner(
        path: &Path,
        desired_mode: Option<crate::StorageMode>,
    ) -> std::result::Result<Self, std::io::Error> {
        use crate::index::hnsw::persistence;

        let meta = persistence::load_meta(path)?;
        let storage_mode = if desired_mode == Some(crate::StorageMode::RaBitQ) {
            crate::StorageMode::RaBitQ
        } else {
            meta.storage_mode
        };

        // Load HNSW graph (caller-specific — see persistence::load_sidecars).
        let inner = HnswInner::file_load_with_storage_mode(
            path,
            "native_hnsw",
            meta.metric,
            meta.dimension,
            storage_mode,
        )?;

        // Mappings + vectors in one shared call (RF-DEDUP #448 Group C).
        let (mappings, vectors, enable_vector_storage) = persistence::load_sidecars(path, &meta)?;

        let index = Self {
            dimension: meta.dimension,
            metric: meta.metric,
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings,
            vectors,
            enable_vector_storage,
            rerank_latency_target_us: AtomicU64::new(0),
            rerank_latency_ema_us: AtomicU64::new(0),
            io_holder: None,
        };

        #[cfg(feature = "persistence")]
        index.install_persisted_rabitq(path)?;

        Ok(index)
    }

    /// Installs `<path>/rabitq.idx` into the `RaBitQ` backend, when both exist.
    ///
    /// No-op for the Standard backend or when the file is absent.
    #[cfg(feature = "persistence")]
    fn install_persisted_rabitq(&self, path: &Path) -> std::io::Result<()> {
        let inner = self.inner.read();
        if inner.storage_mode() != crate::StorageMode::RaBitQ {
            return Ok(());
        }
        let Some(rabitq) =
            crate::quantization::RaBitQIndex::load(path).map_err(std::io::Error::other)?
        else {
            return Ok(());
        };
        inner
            .install_trained_rabitq(std::sync::Arc::new(rabitq))
            .map_err(std::io::Error::other)?;
        Ok(())
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
        use crate::index::hnsw::persistence::{self, HnswMeta};

        let path = path.as_ref();
        std::fs::create_dir_all(path)?;

        // #617: stamp every on-disk artefact with the same monotonic generation
        // so that a crash between any two renames (graph, mappings, vectors,
        // meta) is detectable on reload. Errors are propagated rather than
        // silently resetting to generation 1 on corrupted meta (Devin #618
        // follow-up).
        let new_gen = persistence::next_generation(path)?;

        // Dump the HNSW graph itself (caller-specific — see persistence::save_sidecars).
        let storage_mode = {
            let inner = self.inner.read();
            inner.file_dump(path, "native_hnsw")?;
            inner.storage_mode()
        };

        // Graph-generation marker is written IMMEDIATELY after the graph dump
        // and BEFORE the sidecars, so any crash after the graph rename leaves
        // the marker at the new generation while the sidecars still stamp the
        // old one — `load_sidecars` detects the mismatch.
        persistence::save_graph_generation(path, new_gen)?;

        // Mappings + vectors + meta in one shared call (RF-DEDUP #448 Group C).
        // The actual backend storage mode is persisted so save/load round-trips
        // (a RaBitQ index reloads with the RaBitQ backend).
        persistence::save_sidecars(
            path,
            &self.mappings,
            &self.vectors,
            &HnswMeta {
                dimension: self.dimension,
                metric: self.metric,
                enable_vector_storage: self.enable_vector_storage,
                storage_mode,
                // `save_sidecars` overwrites this with `new_gen` (#617).
                generation: 0,
            },
            new_gen,
        )
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

    /// Installs a trained `RaBitQ` quantizer into the live backend.
    ///
    /// Returns `Ok(true)` when the backend is `RaBitQ` and the quantizer was
    /// installed (existing vectors re-encoded in `NodeId` order, O(n·d)),
    /// `Ok(false)` when the backend is Standard (no-op).
    ///
    /// # Errors
    ///
    /// Returns an error if re-encoding a stored vector fails.
    #[cfg(feature = "persistence")]
    pub(crate) fn install_trained_rabitq(
        &self,
        rabitq: std::sync::Arc<crate::quantization::RaBitQIndex>,
    ) -> crate::error::Result<bool> {
        self.inner.read().install_trained_rabitq(rabitq)
    }

    /// Returns true when the backend is `RaBitQ` with a trained quantizer.
    #[cfg(feature = "persistence")]
    #[must_use]
    pub(crate) fn is_rabitq_quantizer_trained(&self) -> bool {
        self.inner.read().is_rabitq_quantizer_trained()
    }
}
