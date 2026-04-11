//! Constructor and persistence methods for `VectorCollection`.

use std::path::PathBuf;

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::quantization::StorageMode;

use super::VectorCollection;

impl VectorCollection {
    /// Creates a new `VectorCollection` at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create(
        path: PathBuf,
        _name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<Self> {
        Ok(Self {
            inner: Collection::create_with_options(path, dimension, metric, storage_mode)?,
        })
    }

    /// Creates a new `VectorCollection` with custom HNSW parameters.
    ///
    /// When `m` or `ef_construction` are `Some`, those values override the
    /// auto-tuned defaults. When both are `None`, this is equivalent to
    /// [`VectorCollection::create`].
    ///
    /// Shortcut for [`VectorCollection::create_with_params`] that only
    /// overrides `max_connections` and `ef_construction`; every other
    /// HNSW field stays at the dimension-based auto-tuned default, and
    /// `pq_rescore_oversampling` uses the engine default of `Some(4)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create_with_hnsw(
        path: PathBuf,
        _name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        m: Option<usize>,
        ef_construction: Option<usize>,
    ) -> Result<Self> {
        let mut params = crate::index::hnsw::HnswParams::auto(dimension);
        if let Some(m) = m {
            params.max_connections = m;
        }
        if let Some(ef) = ef_construction {
            params.ef_construction = ef;
        }
        params.storage_mode = storage_mode;
        Self::create_with_params(path, dimension, metric, storage_mode, params, None)
    }

    /// Creates a new `VectorCollection` with a fully specified
    /// [`HnswParams`](crate::index::hnsw::HnswParams) and an explicit
    /// `pq_rescore_oversampling` override.
    ///
    /// This is the most expressive constructor exposed by
    /// `VectorCollection`: callers pass the full params object directly,
    /// including `alpha` (VAMANA neighbour diversification),
    /// `max_elements` (initial HNSW capacity), and any future field added
    /// to `HnswParams`, without going through the `(m, ef_construction)`
    /// shortcut. Passing `pq_rescore_oversampling = None` keeps the
    /// persisted config in "no explicit override" mode so later migrations
    /// can recompute the factor from dataset shape.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create_with_params(
        path: PathBuf,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        mut hnsw_params: crate::index::hnsw::HnswParams,
        pq_rescore_oversampling: Option<u32>,
    ) -> Result<Self> {
        // Make sure the storage mode baked into the params matches the
        // per-collection storage mode argument. If a caller passed
        // mismatching values we deliberately let the function argument
        // win — it is the more direct, less ambiguous source.
        hnsw_params.storage_mode = storage_mode;
        Ok(Self {
            inner: Collection::create_with_full_config(
                path,
                dimension,
                metric,
                storage_mode,
                hnsw_params,
                pq_rescore_oversampling,
            )?,
        })
    }

    /// Opens an existing `VectorCollection` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or storage cannot be opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        Ok(Self {
            inner: Collection::open(path)?,
        })
    }

    /// Creates a new `VectorCollection` with an async index builder configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create_with_async_builder(
        path: PathBuf,
        dimension: usize,
        metric: DistanceMetric,
        async_builder_config: crate::collection::streaming::AsyncIndexBuilderConfig,
    ) -> Result<Self> {
        Ok(Self {
            inner: Collection::create_with_async_builder(
                path,
                dimension,
                metric,
                async_builder_config,
            )?,
        })
    }

    /// Flushes all engines to disk and saves the config.
    ///
    /// Issue #423: This fast-path flush skips `vectors.idx` serialization.
    /// The WAL provides crash recovery for the vector index.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush(&self) -> Result<()> {
        self.inner.flush()
    }

    /// Full durability flush including `vectors.idx` serialization.
    ///
    /// Issue #423: Use on graceful shutdown to avoid a full WAL replay
    /// on the next startup.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush_full(&self) -> Result<()> {
        self.inner.flush_full()
    }
}
