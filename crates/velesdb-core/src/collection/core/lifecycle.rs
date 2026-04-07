//! Collection lifecycle methods (create, open, flush).

use crate::collection::graph::{ConcurrentEdgeStore, LabelIndex, PropertyIndex, RangeIndex};
use crate::collection::types::{Collection, CollectionConfig};
use crate::error::Result;
use crate::guardrails::GuardRails;
use crate::index::{Bm25Index, HnswIndex};
use crate::sparse_index::DEFAULT_SPARSE_INDEX_NAME;
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage};
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
pub(super) struct CollectionParts {
    pub(super) path: PathBuf,
    pub(super) config: CollectionConfig,
    pub(super) vector_storage: Arc<RwLock<MmapStorage>>,
    pub(super) payload_storage: Arc<RwLock<LogPayloadStorage>>,
    pub(super) index: Arc<HnswIndex>,
    pub(super) text_index: Arc<Bm25Index>,
    pub(super) property_index: PropertyIndex,
    pub(super) label_index: LabelIndex,
    pub(super) range_index: RangeIndex,
    pub(super) edge_store: ConcurrentEdgeStore,
    pub(super) sparse_indexes: BTreeMap<String, SparseInvertedIndex>,
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
            label_index: LabelIndex::new(),
            range_index: RangeIndex::new(),
            edge_store: ConcurrentEdgeStore::new(),
            sparse_indexes: BTreeMap::new(),
        }
    }
}

impl Collection {
    /// Assembles a `Collection` from pre-built components and default caches.
    ///
    /// This is the single point of truth for the `Self { .. }` struct literal,
    /// eliminating duplication across the five public constructors.
    pub(super) fn assemble(parts: CollectionParts) -> Self {
        #[cfg(feature = "persistence")]
        let deferred_indexer = Self::build_deferred_indexer(&parts.config);

        let async_index_builder = Self::build_async_index_builder(&parts.config);

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
            label_index: Arc::new(RwLock::new(parts.label_index)),
            range_index: Arc::new(RwLock::new(parts.range_index)),
            edge_store: Arc::new(parts.edge_store),
            sparse_indexes: Arc::new(RwLock::new(parts.sparse_indexes)),
            secondary_indexes: Arc::new(RwLock::new(HashMap::new())),
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            query_cache: Arc::new(QueryCache::new(256)),
            cached_stats: Arc::new(Mutex::new(None)),
            stats_io_mutex: Arc::new(Mutex::new(())),
            write_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            inserts_since_last_hnsw_save: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            #[cfg(feature = "persistence")]
            stream_ingester: Arc::new(RwLock::new(None)),
            #[cfg(feature = "persistence")]
            delta_buffer: Arc::new(crate::collection::streaming::delta::DeltaBuffer::new()),
            #[cfg(feature = "persistence")]
            deferred_indexer,
            async_index_builder,
        }
    }

    /// Builds the optional `DeferredIndexer` from config.
    ///
    /// Returns `Some(Arc<DeferredIndexer>)` when `deferred_indexing` is
    /// configured and enabled; `None` otherwise.
    #[cfg(feature = "persistence")]
    fn build_deferred_indexer(
        config: &CollectionConfig,
    ) -> Option<Arc<crate::collection::streaming::DeferredIndexer>> {
        config
            .deferred_indexing
            .as_ref()
            .filter(|cfg| cfg.enabled)
            .map(|cfg| {
                Arc::new(crate::collection::streaming::DeferredIndexer::new(
                    cfg.clone(),
                ))
            })
    }

    /// Builds the optional `AsyncIndexBuilder` from config.
    ///
    /// Returns `Some(Arc<AsyncIndexBuilder>)` when `async_index_builder` is
    /// configured; `None` otherwise.
    fn build_async_index_builder(
        config: &CollectionConfig,
    ) -> Option<Arc<crate::collection::streaming::AsyncIndexBuilder>> {
        config.async_index_builder.as_ref().map(|cfg| {
            Arc::new(crate::collection::streaming::AsyncIndexBuilder::new(
                cfg.clone(),
            ))
        })
    }

    /// Initialises persistent storages and indexes for a new collection.
    ///
    /// Returns a complete `CollectionParts` with empty graph/sparse indexes,
    /// ready to be passed to [`Self::assemble`].
    pub(super) fn init_collection_parts(
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

    // Create constructors are in lifecycle_create.rs

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
        let mut config = super::recovery::load_config(&path)?;

        let vector_storage = Arc::new(RwLock::new(MmapStorage::new(&path, config.dimension)?));
        let payload_storage = Arc::new(RwLock::new(LogPayloadStorage::new(&path)?));
        let index = Self::load_or_create_hnsw(&path, &config)?;
        let text_index = Arc::new(Bm25Index::new());

        Self::rebuild_bm25_index(&payload_storage, &text_index);

        let property_index = Self::load_property_index(&path);
        let label_index = Self::rebuild_label_index(&payload_storage);
        let range_index = Self::load_range_index(&path);
        let edge_store = Self::load_edge_store(&path);
        let sparse_indexes = Self::load_named_sparse_indexes(&path);

        config.point_count =
            super::recovery::reconcile_point_count(&config, &vector_storage, &payload_storage);

        super::recovery::run_crash_recovery(&config, &vector_storage, &index)?;

        Ok(Self::assemble(CollectionParts {
            path,
            config,
            vector_storage,
            payload_storage,
            index,
            text_index,
            property_index,
            label_index,
            range_index,
            edge_store,
            sparse_indexes,
        }))
    }

    // create_graph_collection is in lifecycle_create.rs

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

    fn load_edge_store(path: &std::path::Path) -> ConcurrentEdgeStore {
        Self::load_or_default(
            path,
            "edge_store.bin",
            ConcurrentEdgeStore::load_from_file,
            ConcurrentEdgeStore::new,
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

    /// Rebuilds the label index from payload storage on collection open.
    ///
    /// Scans all stored payloads and extracts `_labels` arrays to populate
    /// the in-memory `LabelIndex`. This is cheap for typical graph workloads
    /// (label arrays are small) and avoids requiring a separate persistence
    /// file for the label index.
    fn rebuild_label_index(payload_storage: &Arc<RwLock<LogPayloadStorage>>) -> LabelIndex {
        let storage = payload_storage.read();
        let mut index = LabelIndex::new();
        for id in storage.ids() {
            if let Ok(Some(payload)) = storage.retrieve(id) {
                index.index_from_payload(id, &payload);
            }
        }
        index
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
}
