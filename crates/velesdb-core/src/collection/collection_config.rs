//! Collection configuration and schema versioning.

use crate::collection::auto_reindex::AutoReindexConfig;
use crate::collection::streaming::AsyncIndexBuilderConfig;
use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswParams;
use crate::quantization::StorageMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::collection::graph::GraphSchema;

/// Current on-disk schema version for `config.json`.
///
/// Increment this constant when the persisted format changes in a way that
/// older VelesDB versions cannot safely read. The `Collection::open()` path
/// rejects any `schema_version > CURRENT_SCHEMA_VERSION` with a clear error.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Returns the default schema version for backward-compatible deserialization.
///
/// Old `config.json` files written before schema versioning was introduced
/// will deserialize with this default, which is equivalent to version 1.
fn default_schema_version() -> u32 {
    1
}

/// Returns `Some(4)` as the default PQ rescore oversampling factor.
/// Returns `Option` because the field type is `Option<u32>` (None = disabled).
#[allow(clippy::unnecessary_wraps)]
fn default_pq_rescore_oversampling() -> Option<u32> {
    Some(4)
}

/// Metadata for a collection.
///
/// `#[non_exhaustive]`: new fields are added over time (schema-versioned and
/// serde-defaulted), so external crates must obtain a `CollectionConfig` via
/// the `VectorCollection::create*` constructors / `Collection::config()` rather
/// than a struct literal — this keeps future field additions non-breaking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CollectionConfig {
    /// Name of the collection.
    pub name: String,

    /// Vector dimension (0 for metadata-only or graph-without-embeddings collections).
    pub dimension: usize,

    /// Distance metric.
    pub metric: DistanceMetric,

    /// Number of points in the collection.
    pub point_count: usize,

    /// On-disk schema version for forward-compatibility detection.
    ///
    /// When a newer VelesDB version writes a `config.json` with a higher
    /// schema version, older versions will refuse to open the collection
    /// rather than silently corrupting data.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `1` (the initial version).
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// Storage mode for vectors (Full, SQ8, Binary).
    #[serde(default)]
    pub storage_mode: StorageMode,

    /// Whether this is a metadata-only collection.
    #[serde(default)]
    pub metadata_only: bool,

    /// Graph schema — `Some` iff this is a graph collection.
    /// Persisted to config.json; `None` for vector and metadata collections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_schema: Option<GraphSchema>,

    /// Embedding dimension for graph node vectors (None = no embeddings).
    /// Only meaningful when `graph_schema` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_dimension: Option<usize>,

    /// PQ rescore oversampling factor. `Some(4)` by default.
    ///
    /// The search pipeline fetches `max(k * factor, k + 32)` candidates from HNSW
    /// and rescores them with full-precision ADC.
    ///
    /// - `None`: disables rescore entirely (expert-only; risks silent recall collapse).
    /// - `Some(0)`: treated as disabled (equivalent to `None`) — the oversampling factor
    ///   of 0 produces a candidates count of 0, which falls back to raw HNSW results.
    /// - `Some(n)` where `n > 0`: enables rescore with `n`-fold oversampling.
    #[serde(default = "default_pq_rescore_oversampling")]
    pub pq_rescore_oversampling: Option<u32>,

    /// Custom HNSW index parameters (M, `ef_construction`, etc.).
    ///
    /// When `Some`, these parameters are used to rebuild the HNSW index on
    /// collection reopen if no persisted index exists yet (`native_meta.bin`
    /// absent). When `None`, the default `HnswParams::auto(dimension)` is used.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hnsw_params: Option<HnswParams>,

    /// Deferred indexing configuration (US-366).
    ///
    /// When `Some` and `enabled`, inserts are buffered in memory and
    /// batch-merged into the HNSW index when the buffer reaches
    /// `merge_threshold`. This decouples write latency from index cost.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `None` (disabled).
    #[cfg(feature = "persistence")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_indexing: Option<crate::collection::streaming::DeferredIndexerConfig>,

    /// Async index builder configuration (Issue #488 — Bulk Insert V2).
    ///
    /// When `Some`, enables the `AsyncIndexBuilder` for deferred HNSW
    /// insertion during bulk import. Buffered vectors are flushed to the
    /// HNSW index via `HnswIndex::insert_batch_parallel`.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `None` (disabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_index_builder: Option<AsyncIndexBuilderConfig>,

    /// Auto-reindex configuration (schema v2 — W2).
    ///
    /// When `Some`, the [`AutoReindexManager`](crate::collection::auto_reindex::AutoReindexManager)
    /// is restored automatically on [`Collection::open`](crate::collection::VectorCollection)
    /// so the policy survives a process restart instead of requiring a manual
    /// re-attach.
    ///
    /// Backward compatible: v1 `config.json` files without this field
    /// deserialize to `None` (no manager attached).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_reindex_config: Option<AutoReindexConfig>,

    /// Streaming ingestion configuration (schema v2 — STREAM-7).
    ///
    /// Describes the persisted shape (channel/batch sizing, flush timing) of
    /// the streaming pipeline. The live `StreamIngester` is still created on
    /// demand via `Collection::enable_streaming`; persisting the config lets a
    /// future open-time hook re-enable streaming without a fresh API call.
    ///
    /// Backward compatible: v1 `config.json` files without this field
    /// deserialize to `None` (streaming not configured).
    #[cfg(feature = "persistence")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_config: Option<crate::collection::streaming::StreamingConfig>,

    /// Names of payload fields carrying a secondary metadata index
    /// (`CREATE INDEX (<field>)`) — the persisted **authority** for which
    /// indexes exist (EPIC-081 phase 3d).
    ///
    /// `create_index` adds a field here and `drop_secondary_index` removes it,
    /// each persisted via `save_config`. On
    /// [`Collection::open`](crate::collection::VectorCollection) every listed
    /// field is rebuilt from the recovered payloads (backfill), so an index
    /// survives a process restart instead of silently vanishing — without
    /// which the ordered-index `ORDER BY` fast path, the bitmap pre-filter,
    /// `EXPLAIN` `IndexLookup`, and the index advisor would all change
    /// behaviour after a restart (results stay correct via the exhaustive
    /// fallback).
    ///
    /// A `BTreeSet` so the on-disk ordering is deterministic. Backward
    /// compatible: configs written before this field deserialize to an empty
    /// set (no indexes restored), and an empty set is not serialized.
    ///
    /// Downgrade caveat: a pre-3d binary opening this config ignores the field
    /// (no `deny_unknown_fields`), but the next `save_config` it performs
    /// re-serializes without it, dropping the authority — a subsequent newer
    /// binary then will not restore those indexes until `CREATE INDEX` is
    /// re-issued. Bounded and fully recoverable (results stay correct via the
    /// exhaustive fallback); no schema-version bump guards it, by design.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub indexed_fields: BTreeSet<String>,
}

#[cfg(test)]
#[path = "rescore_config_tests.rs"]
mod rescore_config_tests;
