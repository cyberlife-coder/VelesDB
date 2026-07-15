//! Collection internal types (struct fields, streaming methods).
//!
//! Configuration types (`CollectionConfig`, `CURRENT_SCHEMA_VERSION`) live in
//! the sibling `collection_config` module.

use crate::collection::graph::property_index::{
    CompositeIndexManager, CompositeRangeIndex, EdgePropertyIndex, IndexAdvisor,
    QueryPatternTracker,
};
use crate::collection::graph::{
    ConcurrentEdgeStore, GraphSchema, LabelIndex, PropertyIndex, RangeIndex,
};
use crate::collection::order_by_advisor::OrderByIndexAdvisor;
use crate::collection::stats::CollectionStats;
#[cfg(feature = "persistence")]
use crate::collection::streaming::delta::DeltaBuffer;
use crate::collection::streaming::AsyncIndexBuilder;
#[cfg(feature = "persistence")]
use crate::collection::streaming::{BackpressureError, DeferredIndexer, StreamIngester};
use crate::distance::DistanceMetric;
use crate::guardrails::GuardRails;
use crate::index::sparse::SparseInvertedIndex;
use crate::index::{Bm25Index, HnswIndex, SecondaryIndex};
#[cfg(feature = "persistence")]
use crate::point::Point;
use crate::quantization::{
    BinaryQuantizedVector, PQVector, ProductQuantizer, QuantizedVector, StorageMode,
};
use crate::storage::{LogPayloadStorage, MmapStorage, VectorStorage};
use crate::velesql::{QueryCache, QueryPlanner};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) use super::collection_config::{CollectionConfig, CURRENT_SCHEMA_VERSION};

type PqTrainingSample = (u64, Vec<f32>);

/// Type of collection: Vector-based or Metadata-only.
///
/// # Examples
///
/// ```rust,ignore
/// use velesdb_core::{CollectionType, DistanceMetric, StorageMode};
///
/// // Vector collection (standard)
/// let vector_type = CollectionType::Vector {
///     dimension: 768,
///     metric: DistanceMetric::Cosine,
///     storage_mode: StorageMode::Full,
/// };
///
/// // Metadata-only collection (no vectors)
/// let metadata_type = CollectionType::MetadataOnly;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum CollectionType {
    /// Standard vector collection with HNSW index.
    Vector {
        /// Vector dimension (e.g., 768 for BERT embeddings).
        dimension: usize,
        /// Distance metric for similarity calculations.
        metric: DistanceMetric,
        /// Storage mode for vector quantization.
        storage_mode: StorageMode,
    },
    /// Metadata-only collection (no vectors, no HNSW index).
    ///
    /// Ideal for reference tables, catalogs, and metadata storage.
    /// Supports CRUD operations and `VelesQL` queries on payload.
    /// Does NOT support vector search operations.
    MetadataOnly,

    /// Graph collection for knowledge graph storage.
    ///
    /// Supports heterogeneous nodes (with optional embeddings) and typed edges.
    /// Ideal for agentic memory, knowledge graphs, and entity-relationship storage.
    Graph {
        /// Optional vector dimension for node embeddings.
        dimension: Option<usize>,
        /// Distance metric for similarity (if embeddings are used).
        metric: DistanceMetric,
        /// Graph schema (strict or schemaless).
        schema: GraphSchema,
    },
}

impl Default for CollectionType {
    fn default() -> Self {
        Self::Vector {
            dimension: 768,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        }
    }
}

impl CollectionType {
    /// Returns true if this is a metadata-only collection.
    #[must_use]
    pub const fn is_metadata_only(&self) -> bool {
        matches!(self, Self::MetadataOnly)
    }

    /// Returns the dimension if this is a vector collection.
    #[must_use]
    pub fn dimension(&self) -> Option<usize> {
        match self {
            Self::Vector { dimension, .. } => Some(*dimension),
            Self::Graph { dimension, .. } => *dimension,
            Self::MetadataOnly => None,
        }
    }

    /// Returns true if this is a graph collection.
    #[must_use]
    pub const fn is_graph(&self) -> bool {
        matches!(self, Self::Graph { .. })
    }

    /// Returns the graph schema if this is a graph collection.
    #[must_use]
    pub fn graph_schema(&self) -> Option<&GraphSchema> {
        match self {
            Self::Graph { schema, .. } => Some(schema),
            _ => None,
        }
    }
}

/// Runtime guard-rail limits threaded into a `Collection` from the live
/// [`VelesConfig::limits`](crate::config::LimitsConfig).
///
/// These three fields are the subset of `LimitsConfig` enforced at the
/// `Collection` ingest/search boundary (the other two — `max_dimensions`
/// and `max_collections` — are enforced at `Database` collection-creation
/// time). They are **not** persisted to `config.json`: each `Database`
/// re-pushes the live values after every open, so the source of truth stays
/// the runtime `VelesConfig`.
///
/// `Copy` so the field adds no allocation and `Collection: Clone` stays cheap.
/// The default mirrors [`LimitsConfig::default`](crate::config::LimitsConfig)
/// so direct `Collection::create`/`open` callers (and tests) that never set
/// it are permissive by construction.
#[derive(Debug, Clone, Copy)]
// Field names intentionally mirror `LimitsConfig` so the mapping in
// `from_config` is a 1:1 transcription; the shared `max_` prefix is the
// established convention there.
#[allow(clippy::struct_field_names)]
pub(crate) struct RuntimeLimits {
    /// Maximum vectors a single collection may hold.
    pub(crate) max_vectors_per_collection: usize,
    /// Maximum serialized payload size (bytes) for a single point.
    pub(crate) max_payload_size: usize,
    /// Maximum collection size for which Perfect (brute-force) search is allowed.
    pub(crate) max_perfect_mode_vectors: usize,
}

impl RuntimeLimits {
    /// Extracts the three enforced fields from a [`LimitsConfig`](crate::config::LimitsConfig).
    ///
    /// Single mapping point reused by both [`Default`] and the `Database`
    /// registration push, so the field correspondence never drifts.
    #[must_use]
    pub(crate) fn from_config(limits: &crate::config::LimitsConfig) -> Self {
        Self {
            max_vectors_per_collection: limits.max_vectors_per_collection,
            max_payload_size: limits.max_payload_size,
            max_perfect_mode_vectors: limits.max_perfect_mode_vectors,
        }
    }
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self::from_config(&crate::config::LimitsConfig::default())
    }
}

// === LOCK ORDERING ===
// All code acquiring multiple locks on Collection MUST follow this order.
// Acquiring in any other order risks deadlock under concurrent access.
//
// Canonical order (acquire lower numbers first):
//   1. config
//   1b. payload_mirror   (held while acquiring 2 and 3 during the lazy build)
//   2. vector_storage
//   3. payload_storage
//   4. sq8_cache / binary_cache / pq_cache  (any order among themselves)
//   5. pq_quantizer → pq_training_buffer
//   6. secondary_indexes
//   7. property_index / range_index         (any order among themselves)
//   8. (reserved — edge_store now uses internal sharded locking)
//   9. sparse_indexes
//  10. delta_buffer
//  11. deferred_indexer / async_index_builder (internal locks)
//  12. stats_io_mutex                         (disk I/O only, no other lock held)

/// Vector + payload storage, quantization caches and the columnar mirror.
///
/// Concern cluster **storage** of `Collection` (R1.1 of the god-object split,
/// EPIC #1384). Grouping is a pure declaration/access-path change: no lock is
/// added, removed, merged, or re-ordered — the runtime acquisition order is
/// dictated by code, never by field placement. Lock-order positions of the
/// individual fields are preserved verbatim below.
///
/// `Clone` for the same reason `Collection` is: every field is an `Arc`, so a
/// clone shares state and stays cheap.
#[derive(Clone)]
pub(crate) struct StorageState {
    /// Path to the collection data.
    pub(super) path: PathBuf,

    /// Collection configuration.
    ///
    /// Lock order position: **1**.
    pub(super) config: Arc<RwLock<CollectionConfig>>,

    /// Vector storage (on-disk, memory-mapped).
    ///
    /// Lock order position: **2**.
    pub(super) vector_storage: Arc<RwLock<MmapStorage>>,

    /// Payload storage (on-disk, log-structured).
    ///
    /// Lock order position: **3**.
    pub(super) payload_storage: Arc<RwLock<LogPayloadStorage>>,

    /// HNSW index for fast approximate nearest neighbor search.
    pub(super) index: Arc<HnswIndex>,

    /// BM25 index for full-text search.
    pub(super) text_index: Arc<Bm25Index>,

    /// SQ8 quantized vectors cache (for SQ8 storage mode).
    ///
    /// Lock order position: **4**.
    pub(super) sq8_cache: Arc<RwLock<HashMap<u64, QuantizedVector>>>,

    /// Binary quantized vectors cache (for Binary storage mode).
    ///
    /// Lock order position: **4**.
    pub(super) binary_cache: Arc<RwLock<HashMap<u64, BinaryQuantizedVector>>>,

    /// PQ quantized vectors cache (for ProductQuantization storage mode).
    ///
    /// Lock order position: **4**.
    pub(super) pq_cache: Arc<RwLock<HashMap<u64, PQVector>>>,

    /// Trained ProductQuantizer (lazy-trained on first inserted vectors).
    ///
    /// Lock order position: **5** (`pq_quantizer` → `pq_training_buffer`).
    pub(super) pq_quantizer: Arc<RwLock<Option<ProductQuantizer>>>,

    /// Buffer of first vectors used to train PQ codebooks.
    /// Stores `(point_id, vector)` so trained quantizers can backfill cache entries.
    ///
    /// Lock order position: **5** (`pq_quantizer` → `pq_training_buffer`).
    pub(super) pq_training_buffer: Arc<RwLock<VecDeque<PqTrainingSample>>>,

    /// Columnar mirror of top-level scalar payload fields (`ColumnStore`).
    ///
    /// Lazily built when full-scan debt warrants it; consulted by
    /// `dispatch_metadata_filter` before the JSON scan fallback.
    /// Lock order position: **1b** — held while acquiring `vector_storage`
    /// (2) and `payload_storage` (3) during the lazy build; mutation hooks
    /// and queries acquire it with no other collection lock held.
    pub(crate) payload_mirror: Arc<crate::collection::payload_mirror::PayloadMirror>,
}

/// Graph node/edge indexes, advisors and the edge store.
///
/// Concern cluster **graph** of `Collection` (R1.1, EPIC #1384). See
/// [`StorageState`] for the lock-order-preservation rationale.
#[derive(Clone)]
pub(crate) struct GraphState {
    /// Property index for O(1) equality lookups on graph nodes (EPIC-009).
    ///
    /// Lock order position: **7**.
    pub(super) property_index: Arc<RwLock<PropertyIndex>>,

    /// Label index for O(1) label-based node lookups (Issue #486).
    ///
    /// Maps label names to `RoaringBitmap` of node IDs, enabling
    /// `find_start_nodes()` to skip the O(N) full scan when a MATCH
    /// pattern specifies node labels like `(n:Person)`.
    ///
    /// Lock order position: **7** (same as `property_index` / `range_index`).
    pub(super) label_index: Arc<RwLock<LabelIndex>>,

    /// Range index for O(log n) range queries on graph nodes (EPIC-009).
    ///
    /// Lock order position: **7**.
    pub(super) range_index: Arc<RwLock<RangeIndex>>,

    /// Graph node property range indexes keyed by `"label.property"` (EPIC-047).
    ///
    /// Populated automatically when nodes are stored via `store_node_payload`.
    /// Lock order position: **7** (same tier as `property_index` / `range_index`).
    pub(crate) graph_range_indexes: Arc<RwLock<HashMap<String, CompositeRangeIndex>>>,

    /// Edge property indexes keyed by `"rel_type.property"` (EPIC-047).
    ///
    /// Populated automatically when edges with properties are added.
    /// Lock order position: **7** (same tier as `property_index` / `range_index`).
    pub(crate) edge_range_indexes: Arc<RwLock<HashMap<String, EdgePropertyIndex>>>,

    /// Composite index manager for multi-property lookups (EPIC-047).
    ///
    /// Lock order position: **7** (same tier as `property_index` / `range_index`).
    pub(crate) composite_index_manager: Arc<RwLock<CompositeIndexManager>>,

    /// Query pattern tracker for auto-index suggestion (EPIC-047).
    ///
    /// Lock order position: **7** (same tier as `property_index` / `range_index`).
    pub(crate) query_pattern_tracker: Arc<RwLock<QueryPatternTracker>>,

    /// Index advisor that suggests indexes based on tracked patterns (EPIC-047).
    ///
    /// Lock order position: **7** (same tier as `property_index` / `range_index`).
    pub(crate) index_advisor: Arc<RwLock<IndexAdvisor>>,

    /// Scalar `ORDER BY <field>` index advisor (EPIC-081 phase 3a).
    ///
    /// Records eligible `ORDER BY` queries that fell back to the exhaustive
    /// sort because the sort field lacks a fully-covering secondary index, so
    /// an operator can be advised to create one. Recommendation-only — never
    /// mutates an index or a query result.
    ///
    /// Lock order position: **7** (same tier as `index_advisor`).
    pub(crate) order_by_advisor: Arc<RwLock<OrderByIndexAdvisor>>,

    /// Concurrent edge store for knowledge graph relationships (EPIC-015).
    ///
    /// Uses sharded internal locking (256 shards) — no outer `RwLock` needed.
    /// Lock order position **8** is now managed internally by `ConcurrentEdgeStore`.
    pub(super) edge_store: Arc<ConcurrentEdgeStore>,

    /// Serializes edge-WAL append + edge-store apply pairs so the WAL order
    /// always equals the apply order (replay resolves id collisions exactly
    /// like live execution) and concurrent appends cannot interleave one
    /// entry's bytes.
    ///
    /// Lock order position: **7c** — acquired with no other collection lock
    /// held; the edge store's internal `edge_ids → shards` chain is acquired
    /// while holding it.
    pub(super) edge_wal_lock: Arc<Mutex<()>>,
}

/// Secondary/sparse payload indexes and the query-execution engine state.
///
/// Concern cluster **query** of `Collection` (R1.1, EPIC #1384). See
/// [`StorageState`] for the lock-order-preservation rationale.
#[derive(Clone)]
pub(crate) struct QueryState {
    /// Named sparse inverted indexes for sparse vector search (EPIC-062).
    /// Key is the sparse vector name (e.g., `""` for default, `"title"`, `"body"`).
    ///
    /// Lock order position: **9**.
    pub(super) sparse_indexes: Arc<RwLock<BTreeMap<String, SparseInvertedIndex>>>,

    /// Secondary indexes for metadata payload fields.
    ///
    /// Lock order position: **6**.
    pub(super) secondary_indexes: Arc<RwLock<HashMap<String, SecondaryIndex>>>,

    /// Query planner for cost-based optimization (EPIC-046).
    pub(crate) query_planner: Arc<QueryPlanner>,

    /// Query parse cache for amortizing repeated query parsing (P1-A).
    pub(crate) query_cache: Arc<QueryCache>,

    /// Cached CBO statistics with TTL (avoids O(n) scan per query).
    pub(crate) cached_stats: Arc<Mutex<Option<(CollectionStats, std::time::Instant)>>>,

    /// Guards read → modify → write cycles on `collection.stats.json`.
    ///
    /// Lock order position: **12** (after `deferred_indexer`/`async_index_builder`
    /// at 11). Protects only disk I/O — no other lock is held while this one is
    /// held, so it cannot participate in a deadlock chain.
    pub(super) stats_io_mutex: Arc<Mutex<()>>,
}

/// Monotonic generation counters that gate compiled-plan cache invalidation.
///
/// Concern cluster **generations** of `Collection` (R1.1, EPIC #1384). These
/// are lock-free atomics; grouping them changes no synchronization.
#[derive(Clone)]
pub(crate) struct GenerationCounters {
    /// Monotonic write generation counter (CACHE-01).
    ///
    /// Incremented once per mutation batch (upsert, `upsert_bulk`, `upsert_metadata`, delete).
    /// Used by `CompiledPlanCache` to invalidate cached query plans when collection data changes.
    /// `Arc` because `Collection` is `Clone` and all clones must share the same counter.
    pub(crate) write_generation: Arc<std::sync::atomic::AtomicU64>,

    /// Monotonic analyze generation counter (issue #608).
    ///
    /// Incremented every time `ANALYZE` produces fresh `CollectionStats`.
    /// Threaded into the compiled plan cache key so that running `ANALYZE`
    /// alone (without any subsequent mutation) invalidates plans whose cost
    /// estimates were derived from pre-analyze heuristics.
    ///
    /// Stored behind `Arc` for the same reason as `write_generation`.
    pub(crate) analyze_generation: Arc<std::sync::atomic::AtomicU64>,

    /// Tracks inserts since the last HNSW index save (Issue #423 Component 3).
    ///
    /// When this counter exceeds `HNSW_SAVE_THRESHOLD`, `flush()` saves the
    /// HNSW graph as a safety measure. `flush_full()` always saves and resets.
    pub(crate) inserts_since_last_hnsw_save: Arc<std::sync::atomic::AtomicU64>,
}

/// Streaming ingestion, delta buffering and deferred/async/auto indexing.
///
/// Concern cluster **streaming** of `Collection` (R1.1, EPIC #1384). Three of
/// the five fields are `#[cfg(feature = "persistence")]`; the cfg attributes
/// stay on the individual fields so the struct compiles (and derives `Clone`)
/// under both `--features persistence` and `--no-default-features`. See
/// [`StorageState`] for the lock-order-preservation rationale.
#[derive(Clone)]
pub(crate) struct StreamingState {
    /// Streaming ingestion handle (STREAM-01).
    ///
    /// `None` when streaming is not configured. Wrapped in `RwLock` so that
    /// the ingester can be lazily initialized or swapped at runtime.
    ///
    /// Future: wire StreamIngester creation into collection open/config (STREAM-01)
    ///
    /// `enable_streaming()` initialises this field at runtime. A future pass should
    /// persist `StreamingConfig` in `CollectionConfig` and restore it on `open`.
    #[cfg(feature = "persistence")]
    pub(super) stream_ingester: Arc<RwLock<Option<StreamIngester>>>,

    /// Delta buffer for vectors pending HNSW index insertion (STREAM-02).
    ///
    /// Lock order position: **10** (after `sparse_indexes` at 9).
    #[cfg(feature = "persistence")]
    pub(crate) delta_buffer: Arc<DeltaBuffer>,

    /// Deferred indexer for high-throughput sequential inserts (US-366).
    ///
    /// `None` when deferred indexing is not configured. When `Some`, inserts
    /// are buffered and batch-merged into the HNSW index at threshold.
    ///
    /// Lock order position: **11** (after `delta_buffer` at 10).
    #[cfg(feature = "persistence")]
    pub(crate) deferred_indexer: Option<Arc<DeferredIndexer>>,

    /// Async index builder for bulk insert V2 (Issue #488).
    ///
    /// `None` when not configured. When `Some`, buffers vectors during
    /// bulk import and flushes them to the HNSW index via
    /// `HnswIndex::insert_batch_parallel`.
    ///
    /// Lock order position: **11** (same tier as `deferred_indexer`).
    pub(crate) async_index_builder: Option<Arc<AsyncIndexBuilder>>,

    /// Runtime-only auto-reindex manager (Wave 3 Commit 9).
    ///
    /// `None` by default. Attached via
    /// [`VectorCollection::attach_auto_reindex`] after the collection is
    /// opened. **Not persisted** to `config.json` — each caller must
    /// re-attach after every [`Database::open`](crate::Database::open) to
    /// avoid the `Duration` serde round-trip problem and the associated
    /// schema version bump.
    ///
    /// When attached, the bulk upsert hot path (see `crud_bulk.rs`) calls
    /// [`AutoReindexManager::should_reindex`](crate::collection::auto_reindex::AutoReindexManager::should_reindex)
    /// after a successful batch. A `true` result is surfaced via
    /// `tracing::info!` — automatic reconstruction is out of scope for
    /// Wave 3 and is left to the caller or a background task.
    ///
    /// Lock order position: **11** (same tier as `deferred_indexer` /
    /// `async_index_builder`).
    pub(crate) auto_reindex:
        Arc<RwLock<Option<Arc<crate::collection::auto_reindex::AutoReindexManager>>>>,
}

/// Query-execution guard-rails and the runtime ingest/search limits.
///
/// Concern cluster **runtime** of `Collection` (R1.1, EPIC #1384).
#[derive(Clone)]
pub(crate) struct RuntimeGuards {
    /// Guard-rails for query execution (EPIC-048).
    pub(crate) guard_rails: Arc<GuardRails>,

    /// Runtime ingest/search limits pushed from the live
    /// [`VelesConfig::limits`](crate::config::LimitsConfig) at `Database`
    /// registration time (parity item E).
    ///
    /// Defaults to the permissive [`RuntimeLimits::default`] so direct
    /// `Collection::create`/`open` callers are unaffected; the `Database`
    /// registration paths overwrite it via [`Collection::set_runtime_limits`].
    /// `Arc<RwLock<_>>` so every `Collection` clone shares the same value and
    /// the setter can run after the registry has cloned the collection.
    /// **Not persisted** — re-pushed on every open.
    pub(crate) runtime_limits: Arc<RwLock<RuntimeLimits>>,
}

/// A collection of vectors with associated metadata.
///
/// Internal executor type — external callers should use `VectorCollection`,
/// `GraphCollection`, or `MetadataCollection` instead.
///
/// The ~39 shared fields are grouped into six concern sub-structs
/// ([`StorageState`], [`GraphState`], [`QueryState`], [`GenerationCounters`],
/// [`StreamingState`], [`RuntimeGuards`]) as the foundation of the god-object
/// split (R1.1 of EPIC #1384). This is a pure structural regrouping: the lock
/// ordering documented above is unchanged (no lock added, removed, merged, or
/// re-ordered), every field keeps its original visibility and `cfg`, and the
/// public API is untouched.
#[derive(Clone)]
pub(crate) struct Collection {
    /// Vector + payload storage, quantization caches and columnar mirror.
    pub(crate) storage: StorageState,

    /// Graph node/edge indexes, advisors and the edge store.
    pub(crate) graph: GraphState,

    /// Secondary/sparse payload indexes and query-execution engine state.
    pub(crate) query: QueryState,

    /// Monotonic generation counters for plan-cache invalidation.
    pub(crate) generations: GenerationCounters,

    /// Streaming ingestion, delta buffering and deferred/async/auto indexing.
    pub(crate) streaming: StreamingState,

    /// Query-execution guard-rails and runtime ingest/search limits.
    pub(crate) runtime: RuntimeGuards,
}

impl Collection {
    /// Returns a reference to the named sparse indexes lock (EPIC-062 sparse integration).
    #[allow(dead_code)] // Reason: Used in tests for sparse index verification
    pub(crate) fn sparse_indexes(&self) -> &Arc<RwLock<BTreeMap<String, SparseInvertedIndex>>> {
        &self.query.sparse_indexes
    }

    /// Overwrites the runtime ingest/search limits (parity item E).
    ///
    /// Called by the `Database` registration paths to push the live
    /// [`VelesConfig::limits`](crate::config::LimitsConfig) into the
    /// collection. The value is **not** persisted — each open re-pushes it.
    pub(crate) fn set_runtime_limits(&self, limits: RuntimeLimits) {
        *self.runtime.runtime_limits.write() = limits;
    }

    /// Returns the current runtime limits snapshot (`Copy`, no lock retained).
    pub(crate) fn runtime_limits(&self) -> RuntimeLimits {
        *self.runtime.runtime_limits.read()
    }

    /// Enforces the runtime ingest limits at the cold upsert boundary
    /// (parity item E): the O(1) `max_vectors_per_collection` cap once for
    /// the whole batch, then `max_payload_size` per point.
    ///
    /// Shared by [`Self::upsert`](crate::collection::Collection) and
    /// `upsert_bulk_inner` so both ingest paths apply identical limits with
    /// no duplicated logic. Runs before any storage lock or WAL write, so a
    /// violation rejects the batch without leaving partial state.
    ///
    /// # Cap is a conservative pre-count
    ///
    /// `max_vectors_per_collection` is checked as `len() + points.len()`,
    /// treating every incoming point as net-new. Because upsert dedups by id,
    /// re-supplying ids already present (a pure in-place update) does **not**
    /// grow the stored count, yet still counts toward the projection here.
    /// A collection exactly at the cap may therefore reject an update batch
    /// that would have left the count unchanged. This O(1) approximation is
    /// intentional: counting true net-new ids would require a storage read
    /// pass on the hot ingest boundary. Raise the cap to update at the limit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::GuardRail`](crate::error::Error::GuardRail) when the
    /// batch would push the collection past `max_vectors_per_collection`, or
    /// when any point's serialized payload exceeds `max_payload_size`.
    pub(crate) fn enforce_upsert_limits(
        &self,
        points: &[crate::point::Point],
    ) -> crate::error::Result<()> {
        let limits = self.runtime_limits();
        self.enforce_vector_count(points.len(), limits.max_vectors_per_collection)?;
        for point in points {
            if let Some(payload) = point.payload.as_ref() {
                Self::enforce_payload_value_size(point.id, payload, limits.max_payload_size)?;
            }
        }
        Ok(())
    }

    /// Projects the post-batch collection size against the vector cap.
    ///
    /// Shared by the `Point`-based [`Self::enforce_upsert_limits`] and the
    /// slice-based raw bulk path so both apply the identical conservative
    /// pre-count (see the doc note on `enforce_upsert_limits`).
    pub(crate) fn enforce_vector_count(
        &self,
        incoming: usize,
        cap: usize,
    ) -> crate::error::Result<()> {
        let projected = self.len().saturating_add(incoming);
        if projected > cap {
            return Err(crate::error::Error::GuardRail(format!(
                "upsert would raise collection size to {projected}, exceeding \
                 max_vectors_per_collection cap of {cap}; raise \
                 `limits.max_vectors_per_collection` in VelesConfig"
            )));
        }
        Ok(())
    }

    /// Rejects a JSON payload whose serialized size exceeds the cap.
    ///
    /// The single shared payload-size gate, reused by every ingest path
    /// (`Point` upsert, raw bulk, and graph node writes). Measures the
    /// serialized length with a bounded counting writer that stops as soon
    /// as the running total passes `cap`, so it never materializes a throwaway
    /// `Vec` and never serializes more than `cap + 1` bytes. Payloads that
    /// fail to serialize (the JSON value is in-memory and infallible in
    /// practice) are accepted.
    pub(crate) fn enforce_payload_value_size(
        id: u64,
        payload: &serde_json::Value,
        cap: usize,
    ) -> crate::error::Result<()> {
        let mut counter = crate::collection::payload_size::BoundedCounter::new(cap);
        if serde_json::to_writer(&mut counter, payload).is_err() && !counter.exceeded() {
            // A real I/O error from the counter only ever signals "over cap"
            // (see `BoundedCounter`); any other serde error is treated as
            // unserializable and accepted, matching prior behavior.
            return Ok(());
        }
        if counter.exceeded() {
            return Err(crate::error::Error::GuardRail(format!(
                "point {id} payload exceeds max_payload_size cap of {cap} bytes; \
                 raise `limits.max_payload_size` in VelesConfig"
            )));
        }
        Ok(())
    }

    /// Returns the current write generation counter.
    ///
    /// The counter starts at 0 and increments once per mutation batch.
    #[must_use]
    pub(crate) fn write_generation(&self) -> u64 {
        self.generations
            .write_generation
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns the current analyze generation counter (issue #608).
    ///
    /// Starts at 0 and increments each time `ANALYZE` is run against the
    /// collection. Threaded through the compiled plan cache key so that the
    /// cache invalidates when calibrated stats change, even when no data
    /// mutation has bumped `write_generation`.
    #[must_use]
    pub(crate) fn analyze_generation(&self) -> u64 {
        self.generations
            .analyze_generation
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Bumps the analyze generation counter (issue #608).
    ///
    /// Called by `Database::analyze_collection` after fresh stats are
    /// persisted. Uses `Relaxed` ordering because the cache key is rebuilt
    /// on every query dispatch; observing a slightly stale counter on a
    /// concurrent reader at most causes one extra cache miss (self-healing).
    pub(crate) fn bump_analyze_generation(&self) {
        self.generations
            .analyze_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Extracts all string values from a JSON payload for text indexing.
    pub(crate) fn extract_text_from_payload(payload: &serde_json::Value) -> String {
        crate::collection::text_utils::extract_text(payload)
    }

    /// Sends a point into the streaming ingestion channel.
    ///
    /// Returns `BackpressureError::NotConfigured` if streaming is not active
    /// on this collection.
    ///
    /// # Errors
    ///
    /// Returns [`BackpressureError`] on buffer-full or not-configured.
    #[cfg(feature = "persistence")]
    pub fn stream_insert(&self, point: Point) -> Result<(), BackpressureError> {
        let guard = self.streaming.stream_ingester.read();
        match guard.as_ref() {
            Some(ingester) => ingester.try_send(point),
            None => Err(BackpressureError::NotConfigured),
        }
    }

    /// Sends a batch of points into the streaming ingestion channel.
    ///
    /// Acquires the ingester read-lock once for the entire batch, eliminating
    /// per-point lock overhead. Returns the number of points successfully
    /// queued. If the channel fills mid-batch, returns
    /// [`BackpressureError::BufferFull`] (points already sent are still queued).
    ///
    /// # Errors
    ///
    /// Returns [`BackpressureError`] on buffer-full, drain-dead, or not-configured.
    #[cfg(feature = "persistence")]
    pub fn stream_insert_batch(&self, points: Vec<Point>) -> Result<usize, BackpressureError> {
        let guard = self.streaming.stream_ingester.read();
        match guard.as_ref() {
            Some(ingester) => ingester.try_send_batch(points),
            None => Err(BackpressureError::NotConfigured),
        }
    }

    /// Enables streaming ingestion on this collection.
    ///
    /// Creates a [`StreamIngester`] with the given `config` and stores it in
    /// the `stream_ingester` field. Points can then be submitted via
    /// [`stream_insert`](Self::stream_insert).
    ///
    /// Calling this when streaming is already active replaces the existing
    /// ingester (the old drain task is aborted via `Drop`).
    ///
    /// Future: auto-enable from persisted StreamingConfig on open (STREAM-01)
    #[cfg(feature = "persistence")]
    pub fn enable_streaming(&self, config: crate::collection::streaming::StreamingConfig) {
        use crate::collection::streaming::StreamIngester;
        let ingester = StreamIngester::new(self.clone(), config);
        *self.streaming.stream_ingester.write() = Some(ingester);
    }

    /// Pushes entries into the delta buffer if it is currently active.
    ///
    /// This is a convenience method for callers (e.g., the REST upsert handlers)
    /// that do not have direct access to the delta buffer internals.
    ///
    /// No-op when the delta buffer is inactive.
    #[cfg(feature = "persistence")]
    pub fn push_to_delta_if_active(&self, entries: &[(u64, Vec<f32>)]) {
        if self.streaming.delta_buffer.is_active() {
            self.streaming
                .delta_buffer
                .extend(entries.iter().map(|(id, v)| (*id, v.clone())));
        }
    }

    /// Attaches an [`AutoReindexManager`](crate::collection::auto_reindex::AutoReindexManager)
    /// and records its config for persistence (schema v2 — W2).
    ///
    /// Replaces any previously attached manager. The manager is consulted by
    /// the bulk upsert hot path after every successful batch and can be
    /// queried externally via [`Self::auto_reindex_manager`] or
    /// [`Self::check_auto_reindex_divergence`].
    ///
    /// The manager's config is mirrored into [`CollectionConfig::auto_reindex_config`]
    /// so the next `save_config` persists it; the manager is then restored
    /// automatically on the following [`Collection::open`] without a manual
    /// re-attach.
    pub(crate) fn attach_auto_reindex(
        &self,
        manager: Arc<crate::collection::auto_reindex::AutoReindexManager>,
    ) {
        // Mirror the policy into the persisted config first (config lock
        // released before the auto_reindex lock at position 11 is taken).
        self.storage.config.write().auto_reindex_config = Some(manager.config());
        *self.streaming.auto_reindex.write() = Some(manager);
    }

    /// Detaches the currently attached auto-reindex manager, if any.
    ///
    /// Subsequent bulk upserts will no longer consult the manager. Returns
    /// the previously attached manager so callers can drop or reuse it. Also
    /// clears the persisted [`CollectionConfig::auto_reindex_config`] so a
    /// subsequent `save_config` does not re-restore it on the next open.
    pub(crate) fn detach_auto_reindex(
        &self,
    ) -> Option<Arc<crate::collection::auto_reindex::AutoReindexManager>> {
        self.storage.config.write().auto_reindex_config = None;
        self.streaming.auto_reindex.write().take()
    }

    /// Returns a clone of the currently attached auto-reindex manager, if any.
    ///
    /// External consumers use this to inspect the manager state, register
    /// their own event callbacks, or trigger a manual reindex.
    #[must_use]
    pub(crate) fn auto_reindex_manager(
        &self,
    ) -> Option<Arc<crate::collection::auto_reindex::AutoReindexManager>> {
        self.streaming.auto_reindex.read().as_ref().map(Arc::clone)
    }

    /// Returns a [`DivergenceCheck`](crate::collection::auto_reindex::DivergenceCheck)
    /// from the attached manager, or `None` if no manager is attached.
    ///
    /// Uses the collection's current persisted HNSW params, the live vector
    /// count, and the configured dimension. Callers that want to force a
    /// particular parameter set should use the manager directly via
    /// [`Self::auto_reindex_manager`].
    ///
    /// This method is a read-only query — it does not trigger any state
    /// transition on the manager.
    #[must_use]
    pub(crate) fn check_auto_reindex_divergence(
        &self,
    ) -> Option<crate::collection::auto_reindex::DivergenceCheck> {
        let manager = self.auto_reindex_manager()?;
        let (params, size, dimension) = self.auto_reindex_inputs();
        Some(manager.check_divergence(&params, size, dimension))
    }

    /// Notifies the attached auto-reindex manager after a successful bulk
    /// upsert, surfacing a `tracing::info!` event when the manager reports
    /// that a reindex would be beneficial.
    ///
    /// Silently no-ops when no manager is attached. Does not block the
    /// hot path — the only cost is three `parking_lot::RwLock::read()`
    /// calls when a manager is attached, and zero syscalls when it is not.
    ///
    /// This method intentionally does NOT trigger automatic reindex
    /// reconstruction: the runtime-only attachment model leaves that
    /// decision to the caller. External consumers can wire their own
    /// reindex pipeline on top of [`Self::auto_reindex_manager`].
    pub(crate) fn notify_auto_reindex_after_bulk(&self) {
        let Some(manager) = self.auto_reindex_manager() else {
            return;
        };
        let (params, size, dimension) = self.auto_reindex_inputs();
        if manager.should_reindex(&params, size, dimension) {
            tracing::info!(
                collection = %self.storage.config.read().name,
                current_size = size,
                dimension = dimension,
                "auto-reindex manager reports divergence — reindex recommended"
            );
        }
    }

    /// Gathers the three inputs `AutoReindexManager` needs from the
    /// collection: current HNSW params (persisted in config, falling back
    /// to the engine default when unset), live vector count, and the
    /// configured vector dimension.
    ///
    /// Extracted as a helper so [`Self::check_auto_reindex_divergence`]
    /// and [`Self::notify_auto_reindex_after_bulk`] share the same source
    /// of truth instead of drifting.
    fn auto_reindex_inputs(&self) -> (crate::index::hnsw::HnswParams, usize, usize) {
        let config = self.storage.config.read();
        let params = config.hnsw_params.unwrap_or_default();
        let dimension = config.dimension;
        drop(config);
        let size = self.storage.vector_storage.read().len();
        (params, size, dimension)
    }
}
