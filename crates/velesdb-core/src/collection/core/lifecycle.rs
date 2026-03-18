//! Collection lifecycle methods (create, open, flush).

use crate::collection::graph::{EdgeStore, PropertyIndex, RangeIndex};
use crate::collection::types::{Collection, CollectionConfig, CollectionType};
use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::guardrails::GuardRails;
use crate::index::{Bm25Index, HnswIndex};
use crate::quantization::StorageMode;
use crate::sparse_index::DEFAULT_SPARSE_INDEX_NAME;
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage, VectorStorage};
use crate::validation::validate_dimension;
use crate::velesql::{QueryCache, QueryPlanner};

use crate::index::sparse::SparseInvertedIndex;

use std::collections::{BTreeMap, HashMap, VecDeque};

use parking_lot::{Mutex, RwLock};
use std::path::PathBuf;
use std::sync::Arc;

/// Pre-built components needed to assemble a [`Collection`].
///
/// Used by [`Collection::assemble`] as the single point of truth for the
/// struct literal, eliminating duplication across the five public constructors.
struct CollectionParts {
    path: PathBuf,
    config: CollectionConfig,
    vector_storage: Arc<RwLock<MmapStorage>>,
    payload_storage: Arc<RwLock<LogPayloadStorage>>,
    index: Arc<HnswIndex>,
    text_index: Arc<Bm25Index>,
    property_index: PropertyIndex,
    range_index: RangeIndex,
    edge_store: EdgeStore,
    sparse_indexes: BTreeMap<String, SparseInvertedIndex>,
}

impl CollectionParts {
    /// Returns a new `CollectionParts` with empty graph and sparse indexes.
    ///
    /// The six storage/index fields must be supplied by the caller; only the
    /// four optional index fields (`property_index`, `range_index`,
    /// `edge_store`, `sparse_indexes`) default to empty.
    fn new_with_empty_indexes(
        path: PathBuf,
        config: CollectionConfig,
        vector_storage: Arc<RwLock<MmapStorage>>,
        payload_storage: Arc<RwLock<LogPayloadStorage>>,
        index: Arc<HnswIndex>,
        text_index: Arc<Bm25Index>,
    ) -> Self {
        Self {
            path,
            config,
            vector_storage,
            payload_storage,
            index,
            text_index,
            property_index: PropertyIndex::new(),
            range_index: RangeIndex::new(),
            edge_store: EdgeStore::new(),
            sparse_indexes: BTreeMap::new(),
        }
    }
}

impl Collection {
    /// Assembles a `Collection` from pre-built components and default caches.
    ///
    /// This is the single point of truth for the `Self { .. }` struct literal,
    /// eliminating duplication across the five public constructors.
    fn assemble(parts: CollectionParts) -> Self {
        Self {
            path: parts.path,
            config: Arc::new(RwLock::new(parts.config)),
            vector_storage: parts.vector_storage,
            payload_storage: parts.payload_storage,
            index: parts.index,
            text_index: parts.text_index,
            sq8_cache: Arc::new(RwLock::new(HashMap::new())),
            binary_cache: Arc::new(RwLock::new(HashMap::new())),
            pq_cache: Arc::new(RwLock::new(HashMap::new())),
            pq_quantizer: Arc::new(RwLock::new(None)),
            pq_training_buffer: Arc::new(RwLock::new(VecDeque::new())),
            property_index: Arc::new(RwLock::new(parts.property_index)),
            range_index: Arc::new(RwLock::new(parts.range_index)),
            edge_store: Arc::new(RwLock::new(parts.edge_store)),
            sparse_indexes: Arc::new(RwLock::new(parts.sparse_indexes)),
            secondary_indexes: Arc::new(RwLock::new(HashMap::new())),
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            query_cache: Arc::new(QueryCache::new(256)),
            cached_stats: Arc::new(Mutex::new(None)),
            write_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            #[cfg(feature = "persistence")]
            stream_ingester: Arc::new(RwLock::new(None)),
            #[cfg(feature = "persistence")]
            delta_buffer: Arc::new(crate::collection::streaming::delta::DeltaBuffer::new()),
        }
    }

    /// Initialises persistent storages and indexes for a new collection.
    ///
    /// Returns a complete `CollectionParts` with empty graph/sparse indexes,
    /// ready to be passed to [`Self::assemble`].
    fn init_collection_parts(
        path: PathBuf,
        config: CollectionConfig,
        hnsw_params: Option<crate::index::hnsw::HnswParams>,
    ) -> Result<CollectionParts> {
        let vector_storage = Arc::new(RwLock::new(MmapStorage::new(&path, config.dimension)?));
        let payload_storage = Arc::new(RwLock::new(LogPayloadStorage::new(&path)?));
        let index = if let Some(params) = hnsw_params {
            Arc::new(HnswIndex::with_params(
                config.dimension,
                config.metric,
                params,
            )?)
        } else {
            Arc::new(HnswIndex::new(config.dimension, config.metric)?)
        };
        let text_index = Arc::new(Bm25Index::new());
        Ok(CollectionParts::new_with_empty_indexes(
            path,
            config,
            vector_storage,
            payload_storage,
            index,
            text_index,
        ))
    }

    /// Rebuilds the BM25 full-text index from persisted payloads.
    fn rebuild_bm25_index(
        payload_storage: &Arc<RwLock<LogPayloadStorage>>,
        text_index: &Arc<Bm25Index>,
    ) {
        let storage = payload_storage.read();
        let ids = storage.ids();
        for id in ids {
            if let Ok(Some(payload)) = storage.retrieve(id) {
                let text = Self::extract_text_from_payload(&payload);
                if !text.is_empty() {
                    text_index.add_document(id, &text);
                }
            }
        }
    }

    /// Creates a new collection at the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
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
    fn create_from_config(
        path: PathBuf,
        config: CollectionConfig,
        hnsw_params: Option<crate::index::hnsw::HnswParams>,
    ) -> Result<Self> {
        let skip_dimension_check = config.metadata_only
            || (config.graph_schema.is_some() && config.embedding_dimension.is_none());
        if skip_dimension_check {
            // dimension=0 is valid for metadata-only and graph-without-embedding
        } else {
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
            storage_mode,
            metadata_only: false,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: Some(4),
            hnsw_params: None,
        };
        Self::create_from_config(path, config, None)
    }

    /// Creates a new collection with custom HNSW parameters.
    ///
    /// This is the lowest-level vector collection constructor, giving full
    /// control over the HNSW graph topology (M, `ef_construction`) while
    /// retaining the standard storage pipeline.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the collection directory
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `storage_mode` - Vector storage mode (Full, SQ8, Binary)
    /// * `hnsw_params` - Custom HNSW index parameters
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create_with_hnsw_params(
        path: PathBuf,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        hnsw_params: crate::index::hnsw::HnswParams,
    ) -> Result<Self> {
        let config = CollectionConfig {
            name: Self::name_from_path(&path),
            dimension,
            metric,
            point_count: 0,
            storage_mode,
            metadata_only: false,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: Some(4),
            hnsw_params: Some(hnsw_params),
        };
        Self::create_from_config(path, config, Some(hnsw_params))
    }

    /// Creates a new collection with a specific type (Vector or `MetadataOnly`).
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the collection directory
    /// * `name` - Name of the collection
    /// * `collection_type` - Type of collection to create
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create_typed(
        path: PathBuf,
        name: &str,
        collection_type: &CollectionType,
    ) -> Result<Self> {
        match collection_type {
            CollectionType::Vector {
                dimension,
                metric,
                storage_mode,
            } => Self::create_with_options(path, *dimension, *metric, *storage_mode),
            CollectionType::MetadataOnly => Self::create_metadata_only(path, name),
            CollectionType::Graph { .. } => {
                // Graph collections will be implemented in EPIC-004
                // For now, return an error indicating this is not yet supported
                Err(crate::Error::GraphNotSupported(
                    "Graph collection creation not yet implemented".to_string(),
                ))
            }
        }
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
            dimension: 0,                   // No vector dimension
            metric: DistanceMetric::Cosine, // Default, not used
            point_count: 0,
            storage_mode: StorageMode::Full, // Default, not used
            metadata_only: true,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: Some(4),
            hnsw_params: None,
        };
        Self::create_from_config(path, config, None)
    }

    /// Returns true if this is a metadata-only collection.
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.config.read().metadata_only
    }

    /// Opens an existing collection from the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed.
    ///
    /// # INVARIANT(CACHE-01): write_generation starts at 0 on open
    ///
    /// Every call to `Collection::open` initialises `write_generation` to 0.
    /// This is **safe** for cache correctness because:
    ///
    /// 1. The plan cache is **not persisted** across process restarts — it is
    ///    always empty when the database opens. There are therefore no stale
    ///    cached plans that could be incorrectly served.
    ///
    /// 2. `Database::load_collections` bumps `schema_version` after loading
    ///    at least one collection (C-3). Any plan key built before the load
    ///    would carry the pre-load `schema_version` and would miss the cache
    ///    even if the `write_generation` happened to match.
    ///
    /// 3. Within a single process lifetime the `write_generation` is only ever
    ///    incremented (never reset), so a cache key built with generation N
    ///    will never be reused once the generation advances past N.
    pub fn open(path: PathBuf) -> Result<Self> {
        let config_path = path.join("config.json");
        let config_data = std::fs::read_to_string(&config_path)?;
        let config: CollectionConfig =
            serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))?;

        // Open persistent storages
        let vector_storage = Arc::new(RwLock::new(MmapStorage::new(&path, config.dimension)?));
        let payload_storage = Arc::new(RwLock::new(LogPayloadStorage::new(&path)?));

        // Load HNSW index if it exists, otherwise create new (empty).
        // When hnsw.bin is absent and config.hnsw_params is set, honour the
        // persisted custom params so they survive collection reopen.
        let index = Self::load_or_create_hnsw(&path, &config)?;
        let text_index = Arc::new(Bm25Index::new());

        // Rebuild BM25 index from persisted payloads
        Self::rebuild_bm25_index(&payload_storage, &text_index);

        // Load persisted graph/sparse indexes (EPIC-009, EPIC-062)
        let property_index = Self::load_property_index(&path);
        let range_index = Self::load_range_index(&path);
        let edge_store = Self::load_edge_store(&path);
        let sparse_indexes = Self::load_named_sparse_indexes(&path);

        Ok(Self::assemble(CollectionParts {
            path,
            config,
            vector_storage,
            payload_storage,
            index,
            text_index,
            property_index,
            range_index,
            edge_store,
            sparse_indexes,
        }))
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
            storage_mode: StorageMode::Full,
            metadata_only: false,
            graph_schema: Some(schema),
            embedding_dimension: embedding_dim,
            pq_rescore_oversampling: Some(4),
            hnsw_params: None,
        };
        // NOTE: create_from_config validates dimension only when > 0,
        // so embedding_dim=None (dimension=0) skips validation correctly.
        Self::create_from_config(path, config, None)
    }

    /// Loads the HNSW index from `hnsw.bin` or creates an empty one.
    ///
    /// When `hnsw.bin` is absent and `config.hnsw_params` is set, the
    /// persisted custom params are honoured so they survive collection reopen.
    fn load_or_create_hnsw(
        path: &std::path::Path,
        config: &CollectionConfig,
    ) -> Result<Arc<HnswIndex>> {
        if path.join("hnsw.bin").exists() {
            let idx = HnswIndex::load(path, config.dimension, config.metric)?;
            Ok(Arc::new(idx))
        } else if let Some(params) = config.hnsw_params {
            Ok(Arc::new(HnswIndex::with_params(
                config.dimension,
                config.metric,
                params,
            )?))
        } else {
            Ok(Arc::new(HnswIndex::new(config.dimension, config.metric)?))
        }
    }

    /// Loads all named sparse indexes from disk.
    ///
    /// Scans for `sparse.meta` (default name `""`) and `sparse-{name}.meta` files.
    /// Returns a `BTreeMap` keyed by sparse vector name.
    ///
    /// # Concurrency safety of `read_dir`
    ///
    /// The `read_dir` scan below is safe from race conditions for two reasons:
    ///
    /// 1. **Single-threaded open**: `Collection::open` (and therefore this
    ///    function) is always called from `Database::open`, which runs
    ///    single-threaded during startup. No concurrent writers exist at this
    ///    point.
    ///
    /// 2. **Atomic rename in compaction**: `compact_with_prefix` writes new
    ///    data to `{prefix}.*.tmp` staging files and only promotes them to
    ///    their final names via an atomic `rename(2)`. A `read_dir` scan
    ///    therefore never observes a partially-written `sparse-*.meta` file;
    ///    it either sees the complete previous version or the complete new
    ///    version — never a torn write.
    fn load_named_sparse_indexes(
        path: &std::path::Path,
    ) -> BTreeMap<String, crate::index::sparse::SparseInvertedIndex> {
        let mut indexes = BTreeMap::new();

        // Load default (unprefixed) sparse index: sparse.meta / sparse.wal
        match crate::index::sparse::persistence::load_from_disk(path) {
            Ok(Some(idx)) => {
                indexes.insert(DEFAULT_SPARSE_INDEX_NAME.to_string(), idx);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    "Failed to load default sparse index from {:?}: {}. Skipping.",
                    path,
                    e
                );
            }
        }

        // Scan for named sparse indexes: sparse-{name}.meta files.
        // The `.meta` suffix is the sentinel for a fully compacted (committed)
        // index file; stale `.tmp` artefacts from interrupted compactions are
        // ignored because they do not match the `strip_suffix(".meta")` filter.
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if let Some(sparse_name) = name_str
                    .strip_prefix("sparse-")
                    .and_then(|s| s.strip_suffix(".meta"))
                {
                    let sparse_name = sparse_name.to_string();
                    match crate::index::sparse::persistence::load_named_from_disk(
                        path,
                        &sparse_name,
                    ) {
                        Ok(Some(idx)) => {
                            indexes.insert(sparse_name, idx);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load sparse index '{}' from {:?}: {}. Skipping.",
                                sparse_name,
                                path,
                                e
                            );
                        }
                    }
                }
            }
        }

        indexes
    }

    /// Loads a persisted index from disk, falling back to a default on missing
    /// file or deserialization error.
    ///
    /// This is the single implementation for the load-or-default pattern shared
    /// by `PropertyIndex`, `RangeIndex`, and `EdgeStore`.
    fn load_or_default<T>(
        path: &std::path::Path,
        file_name: &str,
        load_fn: impl FnOnce(&std::path::Path) -> std::io::Result<T>,
        default: impl FnOnce() -> T,
    ) -> T {
        let full_path = path.join(file_name);
        if full_path.exists() {
            match load_fn(&full_path) {
                Ok(val) => return val,
                Err(e) => tracing::warn!(
                    "Failed to load {} from {:?}: {}. Starting with empty default.",
                    file_name,
                    full_path,
                    e
                ),
            }
        }
        default()
    }

    fn load_edge_store(path: &std::path::Path) -> EdgeStore {
        Self::load_or_default(
            path,
            "edge_store.bin",
            EdgeStore::load_from_file,
            EdgeStore::new,
        )
    }

    fn load_property_index(path: &std::path::Path) -> PropertyIndex {
        Self::load_or_default(
            path,
            "property_index.bin",
            PropertyIndex::load_from_file,
            PropertyIndex::new,
        )
    }

    fn load_range_index(path: &std::path::Path) -> RangeIndex {
        Self::load_or_default(
            path,
            "range_index.bin",
            RangeIndex::load_from_file,
            RangeIndex::new,
        )
    }

    /// Returns a reference to the collection's guard rails.
    #[must_use]
    pub fn guard_rails(&self) -> &std::sync::Arc<crate::guardrails::GuardRails> {
        &self.guard_rails
    }

    /// Returns the collection configuration.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        self.config.read().clone()
    }

    /// Saves the collection configuration and index to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn flush(&self) -> Result<()> {
        self.save_config()?;
        self.vector_storage.write().flush()?;
        self.payload_storage.write().flush()?;
        self.index.save(&self.path)?;
        self.flush_secondary_indexes()?;
        self.flush_sparse_indexes()
    }

    /// Persists property index, range index, and edge store (EPIC-009 US-005).
    fn flush_secondary_indexes(&self) -> Result<()> {
        let property_index_path = self.path.join("property_index.bin");
        self.property_index
            .read()
            .save_to_file(&property_index_path)?;

        let range_index_path = self.path.join("range_index.bin");
        self.range_index.read().save_to_file(&range_index_path)?;

        // Save EdgeStore for graph collections (BUG-1: was never persisted)
        if self.config.read().graph_schema.is_some() {
            let edge_store_path = self.path.join("edge_store.bin");
            self.edge_store.read().save_to_file(&edge_store_path)?;
        }

        Ok(())
    }

    /// Compacts all named sparse indexes to disk (EPIC-062 / SPARSE-04).
    fn flush_sparse_indexes(&self) -> Result<()> {
        let indexes = self.sparse_indexes.read();
        for (name, idx) in indexes.iter() {
            crate::index::sparse::persistence::compact_named(&self.path, name, idx)?;
        }
        Ok(())
    }

    /// Returns a reference to the collection's data path.
    #[must_use]
    pub(crate) fn data_path(&self) -> &std::path::Path {
        &self.path
    }

    /// Returns a write guard on the collection config for mutation.
    pub(crate) fn config_write(
        &self,
    ) -> parking_lot::RwLockWriteGuard<'_, crate::collection::types::CollectionConfig> {
        self.config.write()
    }

    /// Returns a write guard on the PQ quantizer slot.
    pub(crate) fn pq_quantizer_write(
        &self,
    ) -> parking_lot::RwLockWriteGuard<'_, Option<crate::quantization::ProductQuantizer>> {
        self.pq_quantizer.write()
    }

    /// Returns a read guard on the PQ quantizer slot.
    pub(crate) fn pq_quantizer_read(
        &self,
    ) -> parking_lot::RwLockReadGuard<'_, Option<crate::quantization::ProductQuantizer>> {
        self.pq_quantizer.read()
    }

    /// Saves the collection configuration to disk.
    ///
    /// Uses atomic write-tmp-fsync-rename to prevent torn writes on crash.
    pub(crate) fn save_config(&self) -> Result<()> {
        use std::io::Write;

        let config = self.config.read();
        let config_path = self.path.join("config.json");
        let tmp_path = self.path.join("config.json.tmp");
        let config_data = serde_json::to_string_pretty(&*config)
            .map_err(|e| Error::Serialization(e.to_string()))?;

        let file = std::fs::File::create(&tmp_path)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(config_data.as_bytes())?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        std::fs::rename(&tmp_path, &config_path)?;
        Ok(())
    }
}
