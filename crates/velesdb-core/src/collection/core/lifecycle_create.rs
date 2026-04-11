//! Collection creation constructors.
//!
//! Extracted from `lifecycle.rs` to reduce NLOC below the 500 threshold.
//! Contains all `create_*` public constructors and their shared helpers.

use crate::collection::types::{Collection, CollectionConfig, CURRENT_SCHEMA_VERSION};
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::quantization::StorageMode;
use crate::validation::validate_dimension;

use std::path::PathBuf;

impl Collection {
    /// Creates a new collection at the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    #[allow(dead_code)] // Reason: Called in velesql tests and test_fixtures
    pub fn create(path: PathBuf, dimension: usize, metric: DistanceMetric) -> Result<Self> {
        Self::create_with_options(path, dimension, metric, StorageMode::default())
    }

    /// Derives the collection name from the directory path.
    fn name_from_path(path: &std::path::Path) -> String {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    }

    /// Shared init-and-persist pipeline for all `create_*` constructors.
    ///
    /// Validates dimensions (when non-zero), creates the directory, assembles
    /// the collection from the supplied config, and persists `config.json`.
    pub(super) fn create_from_config(
        path: PathBuf,
        config: CollectionConfig,
        hnsw_params: Option<crate::index::hnsw::HnswParams>,
    ) -> Result<Self> {
        // dimension=0 is valid for metadata-only and graph-without-embedding
        let skip_dimension_check = config.metadata_only
            || (config.graph_schema.is_some() && config.embedding_dimension.is_none());
        if !skip_dimension_check {
            validate_dimension(config.dimension)?;
        }
        std::fs::create_dir_all(&path)?;

        let collection = Self::assemble(Self::init_collection_parts(path, config, hnsw_params)?);
        collection.save_config()?;
        Ok(collection)
    }

    /// Creates a new collection with custom storage options.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the collection directory
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `storage_mode` - Vector storage mode (Full, SQ8, Binary)
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create_with_options(
        path: PathBuf,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<Self> {
        let config = CollectionConfig {
            name: Self::name_from_path(&path),
            dimension,
            metric,
            point_count: 0,
            schema_version: CURRENT_SCHEMA_VERSION,
            storage_mode,
            metadata_only: false,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: Some(4),
            hnsw_params: None,
            #[cfg(feature = "persistence")]
            deferred_indexing: None,
            async_index_builder: None,
        };
        Self::create_from_config(path, config, None)
    }

    /// Creates a new collection with custom HNSW parameters.
    ///
    /// This is the lowest-level vector collection constructor, giving full
    /// control over the HNSW graph topology (M, `ef_construction`) while
    /// retaining the standard storage pipeline.
    ///
    /// Uses the engine default `pq_rescore_oversampling = 4`. Callers that
    /// need to override the PQ rescore factor should use
    /// [`Collection::create_with_full_config`] instead.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    #[allow(dead_code)] // Reason: kept as a convenience shortcut for lifecycle tests; the
                        // canonical constructor is `create_with_full_config`, which is used by all production
                        // call-sites since Wave 3 Commit 5. Deleting this wrapper would force every test to
                        // restate `Some(4)` for `pq_rescore_oversampling`, spreading the engine default across
                        // the test suite.
    pub fn create_with_hnsw_params(
        path: PathBuf,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        hnsw_params: crate::index::hnsw::HnswParams,
    ) -> Result<Self> {
        Self::create_with_full_config(path, dimension, metric, storage_mode, hnsw_params, Some(4))
    }

    /// Creates a new collection with custom HNSW parameters and an explicit
    /// PQ rescore oversampling factor.
    ///
    /// This is the most expressive vector constructor: callers pass a fully
    /// populated [`HnswParams`] (including `alpha`, `max_elements`, and any
    /// future fields) together with an explicit `pq_rescore_oversampling`
    /// override. It is the single underlying factory called by
    /// [`Collection::create_with_hnsw_params`] (which pins the PQ factor to
    /// its engine default of `Some(4)`).
    ///
    /// Passing `pq_rescore_oversampling = None` keeps the on-disk config
    /// in "no explicit override" mode, which allows later migrations to
    /// recompute the factor from the dataset shape without having to
    /// distinguish a persisted explicit value from a legacy default.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config
    /// cannot be saved.
    pub fn create_with_full_config(
        path: PathBuf,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        hnsw_params: crate::index::hnsw::HnswParams,
        pq_rescore_oversampling: Option<u32>,
    ) -> Result<Self> {
        let config = CollectionConfig {
            name: Self::name_from_path(&path),
            dimension,
            metric,
            point_count: 0,
            schema_version: CURRENT_SCHEMA_VERSION,
            storage_mode,
            metadata_only: false,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling,
            hnsw_params: Some(hnsw_params),
            #[cfg(feature = "persistence")]
            deferred_indexing: None,
            async_index_builder: None,
        };
        Self::create_from_config(path, config, Some(hnsw_params))
    }

    /// Creates a new collection with `AsyncIndexBuilder` configuration.
    ///
    /// When `async_index_builder` is `Some`, `upsert_bulk` uses the optimized
    /// V2 path: `DirectVectorWriter` + `AsyncIndexBuilder` for higher throughput.
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
        let config = CollectionConfig {
            name: Self::name_from_path(&path),
            dimension,
            metric,
            point_count: 0,
            schema_version: CURRENT_SCHEMA_VERSION,
            storage_mode: StorageMode::Full,
            metadata_only: false,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: Some(4),
            hnsw_params: None,
            #[cfg(feature = "persistence")]
            deferred_indexing: None,
            async_index_builder: Some(async_builder_config),
        };
        Self::create_from_config(path, config, None)
    }

    /// Creates a new metadata-only collection (no vectors, no HNSW index).
    ///
    /// Metadata-only collections are optimized for storing reference data,
    /// catalogs, and other non-vector data. They support CRUD operations
    /// and `VelesQL` queries on payload, but NOT vector search.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create_metadata_only(path: PathBuf, name: &str) -> Result<Self> {
        let config = CollectionConfig {
            name: name.to_string(),
            dimension: 0,
            metric: DistanceMetric::Cosine,
            point_count: 0,
            schema_version: CURRENT_SCHEMA_VERSION,
            storage_mode: StorageMode::Full,
            metadata_only: true,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: Some(4),
            hnsw_params: None,
            #[cfg(feature = "persistence")]
            deferred_indexing: None,
            async_index_builder: None,
        };
        Self::create_from_config(path, config, None)
    }

    /// Creates a new graph collection (with optional node embeddings).
    ///
    /// Persists `graph_schema` and `embedding_dimension` in `config.json`.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create_graph_collection(
        path: PathBuf,
        name: &str,
        schema: crate::collection::graph::GraphSchema,
        embedding_dim: Option<usize>,
        metric: DistanceMetric,
    ) -> Result<Self> {
        let config = CollectionConfig {
            name: name.to_string(),
            dimension: embedding_dim.unwrap_or(0),
            metric,
            point_count: 0,
            schema_version: CURRENT_SCHEMA_VERSION,
            storage_mode: StorageMode::Full,
            metadata_only: false,
            graph_schema: Some(schema),
            embedding_dimension: embedding_dim,
            pq_rescore_oversampling: Some(4),
            hnsw_params: None,
            #[cfg(feature = "persistence")]
            deferred_indexing: None,
            async_index_builder: None,
        };
        Self::create_from_config(path, config, None)
    }
}
