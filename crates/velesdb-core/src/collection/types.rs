//! Collection internal types (struct fields, streaming methods).
//!
//! Configuration types (`CollectionConfig`, `CURRENT_SCHEMA_VERSION`) live in
//! the sibling `collection_config` module.

use crate::collection::graph::{
    ConcurrentEdgeStore, GraphSchema, LabelIndex, PropertyIndex, RangeIndex,
};
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
use crate::storage::{LogPayloadStorage, MmapStorage};
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

// === LOCK ORDERING ===
// All code acquiring multiple locks on Collection MUST follow this order.
// Acquiring in any other order risks deadlock under concurrent access.
//
// Canonical order (acquire lower numbers first):
//   1. config
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

/// A collection of vectors with associated metadata.
///
/// Internal executor type — external callers should use `VectorCollection`,
/// `GraphCollection`, or `MetadataCollection` instead.
#[derive(Clone)]
pub(crate) struct Collection {
    /// Path to the collection data.
    pub(super) path: PathBuf,

    /// Collection configuration.
    pub(super) config: Arc<RwLock<CollectionConfig>>,

    /// Vector storage (on-disk, memory-mapped).
    pub(super) vector_storage: Arc<RwLock<MmapStorage>>,

    /// Payload storage (on-disk, log-structured).
    pub(super) payload_storage: Arc<RwLock<LogPayloadStorage>>,

    /// HNSW index for fast approximate nearest neighbor search.
    pub(super) index: Arc<HnswIndex>,

    /// BM25 index for full-text search.
    pub(super) text_index: Arc<Bm25Index>,

    /// SQ8 quantized vectors cache (for SQ8 storage mode).
    pub(super) sq8_cache: Arc<RwLock<HashMap<u64, QuantizedVector>>>,

    /// Binary quantized vectors cache (for Binary storage mode).
    pub(super) binary_cache: Arc<RwLock<HashMap<u64, BinaryQuantizedVector>>>,

    /// PQ quantized vectors cache (for ProductQuantization storage mode).
    pub(super) pq_cache: Arc<RwLock<HashMap<u64, PQVector>>>,

    /// Trained ProductQuantizer (lazy-trained on first inserted vectors).
    pub(super) pq_quantizer: Arc<RwLock<Option<ProductQuantizer>>>,

    /// Buffer of first vectors used to train PQ codebooks.
    /// Stores `(point_id, vector)` so trained quantizers can backfill cache entries.
    pub(super) pq_training_buffer: Arc<RwLock<VecDeque<PqTrainingSample>>>,

    /// Property index for O(1) equality lookups on graph nodes (EPIC-009).
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
    pub(super) range_index: Arc<RwLock<RangeIndex>>,

    /// Concurrent edge store for knowledge graph relationships (EPIC-015).
    ///
    /// Uses sharded internal locking (256 shards) — no outer `RwLock` needed.
    /// Lock order position **8** is now managed internally by `ConcurrentEdgeStore`.
    pub(super) edge_store: Arc<ConcurrentEdgeStore>,

    /// Named sparse inverted indexes for sparse vector search (EPIC-062).
    /// Key is the sparse vector name (e.g., `""` for default, `"title"`, `"body"`).
    pub(super) sparse_indexes: Arc<RwLock<BTreeMap<String, SparseInvertedIndex>>>,

    /// Secondary indexes for metadata payload fields.
    pub(super) secondary_indexes: Arc<RwLock<HashMap<String, SecondaryIndex>>>,

    /// Guard-rails for query execution (EPIC-048).
    pub(crate) guard_rails: Arc<GuardRails>,

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

    /// Monotonic write generation counter (CACHE-01).
    ///
    /// Incremented once per mutation batch (upsert, `upsert_bulk`, `upsert_metadata`, delete).
    /// Used by `CompiledPlanCache` to invalidate cached query plans when collection data changes.
    /// `Arc` because `Collection` is `Clone` and all clones must share the same counter.
    pub(crate) write_generation: Arc<std::sync::atomic::AtomicU64>,

    /// Tracks inserts since the last HNSW index save (Issue #423 Component 3).
    ///
    /// When this counter exceeds `HNSW_SAVE_THRESHOLD`, `flush()` saves the
    /// HNSW graph as a safety measure. `flush_full()` always saves and resets.
    pub(crate) inserts_since_last_hnsw_save: Arc<std::sync::atomic::AtomicU64>,

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
}

impl Collection {
    /// Returns a reference to the named sparse indexes lock (EPIC-062 sparse integration).
    #[allow(dead_code)]
    pub(crate) fn sparse_indexes(&self) -> &Arc<RwLock<BTreeMap<String, SparseInvertedIndex>>> {
        &self.sparse_indexes
    }

    /// Returns the current write generation counter.
    ///
    /// The counter starts at 0 and increments once per mutation batch.
    #[must_use]
    pub(crate) fn write_generation(&self) -> u64 {
        self.write_generation
            .load(std::sync::atomic::Ordering::Relaxed)
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
        let guard = self.stream_ingester.read();
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
        let guard = self.stream_ingester.read();
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
    #[allow(dead_code)] // Reason: Called via VectorCollection/server inner delegation
    pub fn enable_streaming(&self, config: crate::collection::streaming::StreamingConfig) {
        use crate::collection::streaming::StreamIngester;
        let ingester = StreamIngester::new(self.clone(), config);
        *self.stream_ingester.write() = Some(ingester);
    }

    /// Pushes entries into the delta buffer if it is currently active.
    ///
    /// This is a convenience method for callers (e.g., the REST upsert handlers)
    /// that do not have direct access to the delta buffer internals.
    ///
    /// No-op when the delta buffer is inactive.
    #[cfg(feature = "persistence")]
    pub fn push_to_delta_if_active(&self, entries: &[(u64, Vec<f32>)]) {
        if self.delta_buffer.is_active() {
            self.delta_buffer
                .extend(entries.iter().map(|(id, v)| (*id, v.clone())));
        }
    }
}
