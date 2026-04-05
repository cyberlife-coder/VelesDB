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
        Ok(Self {
            inner: Collection::create_with_hnsw_params(
                path,
                dimension,
                metric,
                storage_mode,
                params,
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
