//! Collection lifecycle methods (create, open, flush).

use crate::collection::graph::property_index::{
    CompositeIndexManager, IndexAdvisor, QueryPatternTracker,
};
use crate::collection::graph::{ConcurrentEdgeStore, LabelIndex, PropertyIndex, RangeIndex};
use crate::collection::types::{Collection, CollectionConfig};
use crate::error::Result;
use crate::guardrails::GuardRails;
use crate::index::sparse::SparseInvertedIndex;
use crate::index::{Bm25Index, HnswIndex};
use crate::sparse_index::DEFAULT_SPARSE_INDEX_NAME;
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage};
use crate::velesql::{QueryCache, QueryPlanner};

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
            graph_range_indexes: Arc::new(RwLock::new(HashMap::new())),
            edge_range_indexes: Arc::new(RwLock::new(HashMap::new())),
            composite_index_manager: Arc::new(RwLock::new(CompositeIndexManager::new())),
            query_pattern_tracker: Arc::new(RwLock::new(QueryPatternTracker::new())),
            index_advisor: Arc::new(RwLock::new(IndexAdvisor::new())),
            edge_store: Arc::new(parts.edge_store),
            edge_wal_lock: Arc::new(Mutex::new(())),
            sparse_indexes: Arc::new(RwLock::new(parts.sparse_indexes)),
            secondary_indexes: Arc::new(RwLock::new(HashMap::new())),
            payload_mirror: Arc::new(crate::collection::payload_mirror::PayloadMirror::default()),
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            query_cache: Arc::new(QueryCache::new(256)),
            cached_stats: Arc::new(Mutex::new(None)),
            stats_io_mutex: Arc::new(Mutex::new(())),
            write_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            analyze_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            inserts_since_last_hnsw_save: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            #[cfg(feature = "persistence")]
            stream_ingester: Arc::new(RwLock::new(None)),
            #[cfg(feature = "persistence")]
            delta_buffer: Arc::new(crate::collection::streaming::delta::DeltaBuffer::new()),
            #[cfg(feature = "persistence")]
            deferred_indexer,
            async_index_builder,
            auto_reindex: Arc::new(RwLock::new(None)),
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
        let index = Arc::new(Self::build_hnsw_index(&config, hnsw_params)?);
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

    /// Builds a fresh HNSW index for a collection, honouring the
    /// collection-level storage mode.
    ///
    /// `config.storage_mode` is the source of truth for the index backend:
    /// it overrides whatever `hnsw_params.storage_mode` carries so a
    /// `RaBitQ` collection always gets the binary-traversal backend, even
    /// when the params predate a `TRAIN QUANTIZER 'rabitq'` mode flip.
    fn build_hnsw_index(
        config: &CollectionConfig,
        hnsw_params: Option<crate::index::hnsw::HnswParams>,
    ) -> Result<HnswIndex> {
        let mut params =
            hnsw_params.unwrap_or_else(|| crate::index::hnsw::HnswParams::auto(config.dimension));
        params.storage_mode = config.storage_mode;
        HnswIndex::with_params(config.dimension, config.metric, params)
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

    /// Loads the BM25 index from its snapshot + WAL if present, falling
    /// back to the payload-scan rebuild otherwise.
    ///
    /// Issue #389: O(N) cold-start rebuild → O(1) snapshot load + O(WAL
    /// delta) replay.
    ///
    /// # Contract
    ///
    /// - Snapshot present → load + replay WAL on top.
    /// - Snapshot absent → fall back to the legacy payload rebuild
    ///   (backward-compat for DBs written before this feature).
    /// - Snapshot corrupt → propagate the error (fail-fast, per #618
    ///   learning: silent fallback masks data loss).
    fn load_bm25_index(
        path: &std::path::Path,
        payload_storage: &Arc<RwLock<LogPayloadStorage>>,
    ) -> Result<Arc<Bm25Index>> {
        if let Some(loaded) = crate::index::bm25_persistence::load_snapshot(path)? {
            let index = Arc::new(loaded);
            let wal_path = crate::index::bm25_persistence_wal::wal_path_for_bm25(path);
            let replayed = crate::index::bm25_persistence_wal::wal_replay(&wal_path, &index)?;
            tracing::debug!(
                "BM25 restored from snapshot + {replayed} WAL entries ({} docs)",
                index.len()
            );
            Ok(index)
        } else {
            let index = Arc::new(Bm25Index::new());
            Self::rebuild_bm25_index(payload_storage, &index);
            tracing::debug!(
                "BM25 snapshot absent; rebuilt from payload storage ({} docs)",
                index.len()
            );
            Ok(index)
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
        // Issue #389: try snapshot + WAL first, fall back to payload
        // rebuild if no snapshot exists (backward-compat).
        let text_index = Self::load_bm25_index(&path, &payload_storage)?;

        let property_index = Self::load_property_index(&path);
        let label_index = Self::rebuild_label_index(&payload_storage);
        let range_index = Self::load_range_index(&path);
        let edge_store = Self::load_edge_store(&path);
        let sparse_indexes = Self::load_named_sparse_indexes(&path);

        config.point_count =
            super::recovery::reconcile_point_count(&config, &vector_storage, &payload_storage);

        let index = Self::recover_index_state(&path, &config, &vector_storage, index)?;

        let collection = Self::assemble(CollectionParts {
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
        });

        collection.restore_auto_reindex_from_config();

        #[cfg(feature = "persistence")]
        collection.run_post_open_hooks()?;

        Ok(collection)
    }

    /// Restores the [`AutoReindexManager`](crate::collection::auto_reindex::AutoReindexManager)
    /// from the persisted `auto_reindex_config` (schema v2 — W2).
    ///
    /// No-op when `auto_reindex_config` is `None` (v1 collections or those
    /// created without an auto-reindex policy). Previously the manager had to
    /// be re-attached manually after every open
    /// (see `docs/CORE_WIRING_DEBT.md` entry 2).
    fn restore_auto_reindex_from_config(&self) {
        let cfg = self.config.read().auto_reindex_config.clone();
        if let Some(cfg) = cfg {
            let manager = Arc::new(crate::collection::auto_reindex::AutoReindexManager::new(
                cfg,
            ));
            self.attach_auto_reindex(manager);
        }
    }

    /// Pre-assemble index recovery: quantizer preinstall + 3-pass
    /// reconciliation of the (possibly stale) loaded HNSW index against the
    /// WAL-replayed vector storage.
    ///
    /// The persisted `RaBitQ` quantizer installs BEFORE the reconciliation so
    /// the recovered vectors re-insert through it — otherwise the lazy
    /// training threshold (1000 inserts) would preempt the TRAIN QUANTIZER
    /// artifact with a throwaway quantizer on every reopen of a realistically
    /// sized collection.
    ///
    /// When the reconciliation mutated the index, it is re-saved before the
    /// open completes: the vector WAL was truncated during replay, so without
    /// a fresh save the reconciled delta would be undetectable after the next
    /// crash. Returns the index to assemble (a fresh one when the loaded
    /// index could not be verified — see [`Self::rebuild_if_unverifiable`]).
    fn recover_index_state(
        path: &std::path::Path,
        config: &CollectionConfig,
        vector_storage: &Arc<RwLock<MmapStorage>>,
        index: Arc<HnswIndex>,
    ) -> Result<Arc<HnswIndex>> {
        let wal_ids = vector_storage.write().take_wal_replayed_ids();
        let (index, rebuilt) = Self::rebuild_if_unverifiable(config, index, &wal_ids)?;
        #[cfg(feature = "persistence")]
        super::quantizer_restore::preinstall_persisted_rabitq(path, config.dimension, &index)?;
        let changed =
            super::recovery::run_crash_recovery(config, vector_storage, &index, &wal_ids)?;
        if rebuilt || changed {
            index.save(path)?;
        }
        Ok(index)
    }

    /// Replaces a loaded index that cannot be verified against storage.
    ///
    /// Pass 3 of the open-time reconciliation compares the indexed vectors
    /// against storage for every WAL-touched id. An index loaded without
    /// sidecar vector storage (fast-insert save) has nothing to compare:
    /// when WAL-touched ids overlap its mappings, some entries may be stale
    /// with no way to tell which. Fall back to a fresh empty index — gap
    /// recovery then rebuilds it entirely from storage (simple and safe).
    fn rebuild_if_unverifiable(
        config: &CollectionConfig,
        index: Arc<HnswIndex>,
        wal_ids: &[u64],
    ) -> Result<(Arc<HnswIndex>, bool)> {
        let overlap = wal_ids.iter().any(|id| index.mappings.contains(*id));
        if index.has_vector_storage() || !overlap {
            return Ok((index, false));
        }
        tracing::warn!(
            "loaded HNSW index has no vector storage but {} WAL-touched ids overlap it; \
             rebuilding from vector storage",
            wal_ids.len()
        );
        let fresh = Self::build_hnsw_index(config, config.hnsw_params)?;
        Ok((Arc::new(fresh), true))
    }

    /// Post-open hooks that need a fully assembled collection.
    ///
    /// 1. Edge property indexes: snapshot-loaded edges must re-enter the
    ///    property indexes BEFORE the WAL replays (replay indexes its own
    ///    ADDs — a full pass after it would double-index replayed edges).
    /// 2. Edge crash durability: replay the edge WAL on top of the loaded
    ///    `edge_store` snapshot so edge mutations since the last flush
    ///    survive a crash. No-op when `edges.wal` is absent.
    /// 3. Quantizer restore: reload persisted PQ codebook / `RaBitQ` index
    ///    AFTER crash recovery so every recovered vector is re-encoded.
    ///    O(n) over stored vectors — same cost class as gap recovery.
    #[cfg(feature = "persistence")]
    fn run_post_open_hooks(&self) -> Result<()> {
        self.reindex_edge_properties_from_store();
        self.replay_edge_wal()?;
        self.restore_persisted_quantizers()
    }

    // create_graph_collection is in lifecycle_create.rs

    /// Loads the persisted HNSW index or creates an empty one.
    ///
    /// The presence gate is `native_meta.bin` — the commit point written
    /// LAST by `HnswIndex::save` (generation-stamped, issue #617). When the
    /// load fails or the persisted meta does not match the collection config,
    /// an empty index is built instead and gap recovery rebuilds it from
    /// vector storage (see [`Self::try_load_hnsw`]). When no persisted index
    /// exists and `config.hnsw_params` is set, the persisted custom params
    /// are honoured so they survive collection reopen.
    ///
    /// Both branches honour `config.storage_mode`: the load path upgrades the
    /// backend to `RaBitQ` when the collection mode requires it (and installs
    /// `rabitq.idx` when present — see `HnswIndex::load_with_storage_mode`),
    /// and the create path builds the backend from the collection mode.
    fn load_or_create_hnsw(
        path: &std::path::Path,
        config: &CollectionConfig,
    ) -> Result<Arc<HnswIndex>> {
        if let Some(idx) = Self::try_load_hnsw(path, config) {
            return Ok(Arc::new(idx));
        }
        Ok(Arc::new(Self::build_hnsw_index(
            config,
            config.hnsw_params,
        )?))
    }

    /// Attempts to load the persisted HNSW index, validating it against the
    /// collection config.
    ///
    /// Returns `None` — with a `warn` log — when `native_meta.bin` is absent,
    /// the load fails (corruption, generation mismatch, …), or the persisted
    /// dimension/metric disagree with the collection config. The caller then
    /// falls back to an empty index rebuilt by gap recovery, which is the
    /// pre-existing slow-but-safe behaviour.
    fn try_load_hnsw(path: &std::path::Path, config: &CollectionConfig) -> Option<HnswIndex> {
        if !path.join("native_meta.bin").exists() {
            return None;
        }
        let idx = match HnswIndex::load_with_storage_mode(path, config.storage_mode) {
            Ok(idx) => idx,
            Err(e) => {
                tracing::warn!(
                    "failed to load persisted HNSW index from {path:?}: {e}; \
                     falling back to full rebuild from vector storage"
                );
                return None;
            }
        };
        if idx.dimension() != config.dimension || idx.metric() != config.metric {
            tracing::warn!(
                loaded_dimension = idx.dimension(),
                loaded_metric = ?idx.metric(),
                config_dimension = config.dimension,
                config_metric = ?config.metric,
                "persisted HNSW index does not match the collection config; \
                 falling back to full rebuild from vector storage"
            );
            return None;
        }
        Some(idx)
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
