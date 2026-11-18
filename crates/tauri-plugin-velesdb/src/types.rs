//! Request/Response DTOs for Tauri IPC commands.
//!
//! # Why separate types from `velesdb-server`?
//!
//! Tauri commands are invoked from JavaScript via IPC, which imposes two
//! constraints that differ from the REST server:
//!
//! 1. **`camelCase` serialization** — All types use `#[serde(rename_all = "camelCase")]`
//!    so that JavaScript callers receive idiomatic field names (`topK`, `storageMode`, etc.).
//!    The server uses `snake_case` (REST convention).
//!
//! 2. **`collection` field on requests** — In the REST API the collection name comes from
//!    the URL path (`/collections/{name}/search`). In Tauri IPC there is no URL, so every
//!    request carries a `collection: String` field.
//!
//! ## What is shared with core
//!
//! - **Default value functions** (`default_metric`, `default_top_k`, etc.) are re-exported
//!   from [`velesdb_core::api_types`] to avoid duplication.
//! - **`SearchResult`** re-uses the canonical [`velesdb_core::api_types::SearchResultResponse`]
//!   via a type alias — its fields (`id`, `score`, `payload`) are single-word and therefore
//!   identical under both `camelCase` and `snake_case` serialization.
//!
//! ## What stays Tauri-specific
//!
//! - All **request types** (they carry `collection` + use `camelCase` deserialization).
//! - **`CollectionInfo`** — uses `count` instead of core's `point_count`, and `storage_mode`
//!   is serialized as `storageMode` for JS.
//! - **`HybridResult`** / **`QueryResponse`** — Tauri-specific multi-model query format.
//! - **`PointOutput`** — no direct core response equivalent.
//! - **Graph types** (`EdgeOutput`, `TraversalOutput`, etc.) — Tauri-specific wrappers.

use serde::{Deserialize, Serialize};
use velesdb_core::api_types::serde_id;

// Re-export shared defaults from core for use in serde attributes.
pub use velesdb_core::api_types::{
    default_metric, default_storage_mode, default_top_k, default_vector_weight,
};

// ============================================================================
// Request DTOs — Tauri-IPC-specific
//
// These types MUST remain in this crate because they:
// - Use `#[serde(rename_all = "camelCase")]` for JavaScript callers
// - Include a `collection: String` field (no URL path in IPC)
// - Have different field shapes than the REST API equivalents in core
// ============================================================================

/// Request to create a new collection (Tauri IPC).
///
/// Supports optional advanced HNSW tuning parameters (`hnswM`,
/// `hnswEfConstruction`, `hnswAlpha`, `hnswMaxElements`) and PQ rescore
/// oversampling (`pqRescoreOversampling`). When all advanced fields are
/// omitted the collection uses dimension-based auto-tuned defaults.
///
/// Uses `camelCase` deserialization for JavaScript callers.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCollectionRequest {
    /// Collection name.
    pub name: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Distance metric: "cosine", "euclidean", "dot", "hamming", "jaccard".
    #[serde(default = "default_metric")]
    pub metric: String,
    /// Storage mode: "full", "sq8", "binary".
    #[serde(default = "default_storage_mode")]
    pub storage_mode: String,
    /// HNSW M parameter (max connections per node). Auto-tuned if omitted.
    #[serde(default)]
    pub hnsw_m: Option<usize>,
    /// HNSW `ef_construction` parameter. Auto-tuned if omitted.
    #[serde(default)]
    pub hnsw_ef_construction: Option<usize>,
    /// HNSW alpha for VAMANA neighbor diversification. Default: 1.2.
    #[serde(default)]
    pub hnsw_alpha: Option<f32>,
    /// HNSW initial max elements capacity. Auto-tuned if omitted.
    #[serde(default)]
    pub hnsw_max_elements: Option<usize>,
    /// PQ rescore oversampling factor. Default: 4.
    #[serde(default)]
    pub pq_rescore_oversampling: Option<u32>,
}

/// Request to create a metadata-only collection.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMetadataCollectionRequest {
    /// Collection name.
    pub name: String,
}

/// Request to create a graph collection with optional schema (Tauri IPC).
///
/// Supports two modes:
/// - **Schemaless** (default): pass `graphSchema: { "schemaless": true }` or omit it entirely.
/// - **Strict**: define `node_types` and `edge_types` in the `graphSchema` JSON object.
///
/// When `dimension` is set, node embeddings are enabled with the specified metric.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGraphCollectionRequest {
    /// Collection name.
    pub name: String,
    /// Optional vector dimension for node embeddings. If omitted, graph has no embeddings.
    #[serde(default)]
    pub dimension: Option<usize>,
    /// Distance metric (when dimension is set). Default: "cosine".
    #[serde(default = "default_metric")]
    pub metric: String,
    /// Graph schema definition as JSON.
    /// Pass `{ "schemaless": true }` for schemaless mode (default),
    /// or define `node_types` / `edge_types` for strict mode.
    #[serde(default)]
    pub graph_schema: Option<serde_json::Value>,
}

/// A metadata-only point to insert (no vector).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPointInput {
    /// Point ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Payload (JSON object).
    pub payload: serde_json::Value,
}

/// Request to upsert metadata-only points.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertMetadataRequest {
    /// Collection name.
    pub collection: String,
    /// Metadata points to upsert.
    pub points: Vec<MetadataPointInput>,
}

/// A point to insert/update.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointInput {
    /// Point ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Optional payload (JSON object).
    pub payload: Option<serde_json::Value>,
}

/// Request to upsert points.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertRequest {
    /// Collection name.
    pub collection: String,
    /// Points to upsert.
    pub points: Vec<PointInput>,
}

/// Request to get points by IDs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPointsRequest {
    /// Collection name.
    pub collection: String,
    /// Point IDs to retrieve.
    #[serde(deserialize_with = "serde_id::deserialize_ids_from_string_or_number")]
    pub ids: Vec<u64>,
}

/// Request to delete points by IDs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePointsRequest {
    /// Collection name.
    pub collection: String,
    /// Point IDs to delete.
    #[serde(deserialize_with = "serde_id::deserialize_ids_from_string_or_number")]
    pub ids: Vec<u64>,
}

/// Request to search vectors.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    /// Collection name.
    pub collection: String,
    /// Query vector.
    pub vector: Vec<f32>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
    /// Search quality mode: "fast", "balanced", "accurate", "perfect", "auto",
    /// "custom:\<ef\>", "adaptive:\<min\>:\<max\>".
    #[serde(default)]
    pub quality: Option<String>,
}

/// Individual search request within a batch.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndividualSearchRequest {
    /// Query vector.
    pub vector: Vec<f32>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
    /// Search quality mode: "fast", "balanced", "accurate", "perfect", "auto",
    /// "custom:\<ef\>", "adaptive:\<min\>:\<max\>".
    #[serde(default)]
    pub quality: Option<String>,
}

/// Request for batch search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSearchRequest {
    /// Collection name.
    pub collection: String,
    /// List of search queries.
    pub searches: Vec<IndividualSearchRequest>,
}

/// Request for text search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSearchRequest {
    /// Collection name.
    pub collection: String,
    /// Text query.
    pub query: String,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for hybrid search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchRequest {
    /// Collection name.
    pub collection: String,
    /// Query vector.
    pub vector: Vec<f32>,
    /// Text query.
    pub query: String,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Weight for vector results (0.0-1.0).
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for `VelesQL` query.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    /// `VelesQL` query string.
    pub query: String,
    /// Query parameters.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
    /// Collection a statement without `FROM` (a top-level `MATCH`, an
    /// aggregate) runs on, as the REST server's `/query` body field: it is
    /// passed as `params._collection` unless that is already set, and a
    /// statement with a `FROM` reads its `FROM`.
    #[serde(default)]
    pub collection: Option<String>,
}

/// Request for multi-query fusion search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiQuerySearchRequest {
    /// Collection name.
    pub collection: String,
    /// List of query vectors.
    pub vectors: Vec<Vec<f32>>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Fusion strategy: "rrf", "average", "maximum", "weighted".
    #[serde(default = "default_fusion")]
    pub fusion: String,
    /// Fusion parameters (e.g., {"k": 60} for RRF).
    #[serde(default)]
    pub fusion_params: Option<serde_json::Value>,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for sparse vector search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SparseSearchRequest {
    /// Collection name.
    pub collection: String,
    /// Sparse vector as `{ "dim_index": weight, ... }`.
    pub sparse_vector: std::collections::HashMap<String, f32>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Optional sparse index name.
    #[serde(default)]
    pub index_name: Option<String>,
}

/// Request for hybrid dense+sparse search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSparseSearchRequest {
    /// Collection name.
    pub collection: String,
    /// Dense query vector.
    pub vector: Vec<f32>,
    /// Sparse vector as `{ "dim_index": weight, ... }`.
    pub sparse_vector: std::collections::HashMap<String, f32>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

/// A point input with optional sparse vector.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SparsePointInput {
    /// Point ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Dense vector data.
    pub vector: Vec<f32>,
    /// Optional payload (JSON object).
    pub payload: Option<serde_json::Value>,
    /// Optional sparse vector.
    #[serde(default)]
    pub sparse_vector: Option<std::collections::HashMap<String, f32>>,
}

/// Request to upsert points with optional sparse vectors.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SparseUpsertRequest {
    /// Collection name.
    pub collection: String,
    /// Points to upsert.
    pub points: Vec<SparsePointInput>,
}

/// Request to train a Product Quantizer.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainPqRequest {
    /// Collection name.
    pub collection: String,
    /// Number of sub-quantizers (the dimension must be divisible by it).
    ///
    /// Required, as in the `TRAIN QUANTIZER ... WITH (m = ...)` statement the
    /// REST server runs: core's TRAIN reads an absent `m` as `0` and refuses
    /// it (VELES-028), so an optional `m` could only ever fail.
    pub m: usize,
    /// Number of centroids per sub-quantizer.
    #[serde(default)]
    pub k: Option<usize>,
    /// Whether to use Optimized Product Quantization.
    #[serde(default)]
    pub opq: Option<bool>,
}

/// Request to stream-insert points.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInsertRequest {
    /// Collection name.
    pub collection: String,
    /// Points to stream-insert.
    pub points: Vec<PointInput>,
}

/// Request to enable streaming ingestion on a collection.
///
/// Each field is optional; omitted fields fall back to the engine default
/// (`buffer_size=10000`, `batch_size=128`, `flush_interval_ms=50`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnableStreamingRequest {
    /// Collection name.
    pub collection: String,
    /// Capacity of the bounded ingestion channel (backpressure threshold).
    #[serde(default)]
    pub buffer_size: Option<usize>,
    /// Number of points that trigger an immediate micro-batch flush.
    #[serde(default)]
    pub batch_size: Option<usize>,
    /// Maximum time (ms) before a partial batch is flushed.
    #[serde(default)]
    pub flush_interval_ms: Option<u64>,
}

// ============================================================================
// Response DTOs
//
// Response types that differ from core only in serialization convention.
// Where field names are single-word (camelCase == snake_case), we re-use
// the canonical core type directly.
// ============================================================================

/// Search result — re-uses the canonical core type.
///
/// All fields (`id`, `score`, `payload`) are single-word, so `camelCase` and
/// `snake_case` serialization produce identical JSON. No wrapper needed.
pub type SearchResult = velesdb_core::api_types::SearchResultResponse;

/// Response for collection info (Tauri IPC).
///
/// Differs from [`velesdb_core::api_types::CollectionResponse`]:
/// - Uses `count` instead of `point_count`
/// - Serializes as `camelCase` (`storageMode` vs `storage_mode`)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfo {
    /// Collection name.
    pub name: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Distance metric.
    pub metric: String,
    /// Number of points.
    pub count: usize,
    /// Storage mode.
    pub storage_mode: String,
}

/// Multi-model query result (Tauri IPC).
///
/// No core equivalent — this format is specific to the Tauri `query` command
/// which fuses vector, graph, and column results into a single shape.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridResult {
    /// Node/point ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub node_id: u64,
    /// Vector similarity score (if applicable).
    pub vector_score: Option<f32>,
    /// Graph relevance score (if applicable).
    pub graph_score: Option<f32>,
    /// Combined fused score.
    pub fused_score: f32,
    /// Variable bindings/payload.
    pub bindings: Option<serde_json::Value>,
    /// Result of an aggregate `SELECT ... FROM` (scalar aggregates or a
    /// `GROUP BY` without `NEAR`), on the single row such a query returns: an
    /// object without `GROUP BY`, an array of one object per group with it.
    /// `null` on every other row.
    pub column_data: Option<serde_json::Value>,
}

/// Response for `VelesQL` query operations (Tauri IPC).
///
/// Differs from [`velesdb_core::api_types::QueryResponse`] which has additional
/// fields (`took_ms`, `rows_returned`, `meta`). The Tauri version is simpler,
/// returning `HybridResult` items.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResponse {
    /// Query results in multi-model format.
    pub results: Vec<HybridResult>,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
}

/// Point output for get operations (Tauri IPC).
///
/// No direct core response equivalent. The core `Point` struct is the internal
/// representation; this DTO projects only the fields needed by the JS frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointOutput {
    /// Point ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Response for search operations (Tauri IPC).
///
/// Differs from [`velesdb_core::api_types::SearchResponse`]: includes `timing_ms`
/// (the core version does not) and uses `camelCase` serialization.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    /// Search results.
    pub results: Vec<SearchResult>,
    /// Query time in milliseconds.
    pub timing_ms: f64,
}

/// A single ID + score result returned by the `search_ids` command.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdScoreOutput {
    /// Matched point ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    /// Relevance score.
    pub score: f32,
}

/// Runtime query guardrail limits for a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailLimits {
    /// Maximum graph traversal depth.
    pub max_depth: u32,
    /// Maximum intermediate cardinality.
    pub max_cardinality: u64,
    /// Memory limit per query in bytes.
    pub memory_limit_bytes: u64,
    /// Query timeout in milliseconds (0 disables the timeout).
    pub timeout_ms: u64,
    /// Rate limit: max queries per second per client.
    pub rate_limit_qps: u32,
    /// Circuit breaker: failure threshold before tripping.
    pub circuit_failure_threshold: u32,
    /// Circuit breaker: recovery time in seconds.
    pub circuit_recovery_seconds: u64,
}

impl From<velesdb_core::guardrails::QueryLimits> for GuardrailLimits {
    fn from(v: velesdb_core::guardrails::QueryLimits) -> Self {
        Self {
            max_depth: v.max_depth,
            max_cardinality: u64::try_from(v.max_cardinality).unwrap_or(u64::MAX),
            memory_limit_bytes: u64::try_from(v.memory_limit_bytes).unwrap_or(u64::MAX),
            timeout_ms: v.timeout_ms,
            rate_limit_qps: v.rate_limit_qps,
            circuit_failure_threshold: v.circuit_failure_threshold,
            circuit_recovery_seconds: v.circuit_recovery_seconds,
        }
    }
}

impl From<GuardrailLimits> for velesdb_core::guardrails::QueryLimits {
    fn from(v: GuardrailLimits) -> Self {
        Self {
            max_depth: v.max_depth,
            max_cardinality: usize::try_from(v.max_cardinality).unwrap_or(usize::MAX),
            memory_limit_bytes: usize::try_from(v.memory_limit_bytes).unwrap_or(usize::MAX),
            timeout_ms: v.timeout_ms,
            rate_limit_qps: v.rate_limit_qps,
            circuit_failure_threshold: v.circuit_failure_threshold,
            circuit_recovery_seconds: v.circuit_recovery_seconds,
        }
    }
}

// ============================================================================
// Default value functions (Tauri-specific)
// ============================================================================

#[must_use]
pub fn default_fusion() -> String {
    "rrf".to_string()
}

// ============================================================================
// AgentMemory DTOs (EPIC-016 US-003) — moved to types_memory.rs
// ============================================================================

pub use crate::types_memory::*;

// ============================================================================
// Knowledge Graph Types (EPIC-015 US-001) — moved to types_graph.rs
// ============================================================================
pub use crate::types_graph::*;

// ============================================================================
// Scroll DTOs
// ============================================================================

/// Default batch size for scroll operations.
#[must_use]
pub const fn default_batch_size() -> usize {
    100
}

/// Request to scroll through collection points.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollRequest {
    /// Collection name.
    pub collection: String,
    /// Cursor from a previous scroll (omit for the first batch): a decimal
    /// string, as `nextCursor` is sent, or a number.
    #[serde(
        default,
        deserialize_with = "serde_id::deserialize_option_id_from_string_or_number"
    )]
    pub cursor: Option<u64>,
    /// Number of points per batch. Default: 100.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Response from a scroll operation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollResponse {
    /// Points in this batch.
    pub points: Vec<PointOutput>,
    /// Cursor for the next batch, as a decimal string (absent when no more
    /// points).
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serde_id::serialize_option_id_as_string"
    )]
    pub next_cursor: Option<u64>,
}

// ============================================================================
// Secondary Index DTOs
// ============================================================================

/// Request to create a secondary index on a metadata field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexRequest {
    /// Collection name.
    pub collection: String,
    /// Metadata field name to index.
    pub field_name: String,
}

/// Request to drop a secondary index.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropIndexRequest {
    /// Collection name.
    pub collection: String,
    /// Metadata field name whose index to drop.
    pub field_name: String,
}

/// Request to list secondary indexes on a collection.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListIndexesRequest {
    /// Collection name.
    pub collection: String,
}

/// Output for a secondary index entry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfoOutput {
    /// Node label (`secondary` for a secondary payload index).
    pub label: String,
    /// Property name (the payload field for a secondary index).
    pub property: String,
    /// Index type: `hash` (graph property), `range` (graph range) or `btree`
    /// (secondary payload index).
    pub index_type: String,
    /// Number of distinct values indexed.
    pub cardinality: usize,
    /// Estimated heap size of the index, in bytes.
    pub memory_bytes: usize,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
