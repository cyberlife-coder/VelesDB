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
    pub ids: Vec<u64>,
}

/// Request to delete points by IDs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePointsRequest {
    /// Collection name.
    pub collection: String,
    /// Point IDs to delete.
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
    /// Number of sub-quantizers.
    #[serde(default)]
    pub m: Option<usize>,
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
    pub node_id: u64,
    /// Vector similarity score (if applicable).
    pub vector_score: Option<f32>,
    /// Graph relevance score (if applicable).
    pub graph_score: Option<f32>,
    /// Combined fused score.
    pub fused_score: f32,
    /// Variable bindings/payload.
    pub bindings: Option<serde_json::Value>,
    /// Column data from JOIN (if applicable).
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

// ============================================================================
// Default value functions (Tauri-specific)
// ============================================================================

#[must_use]
pub fn default_fusion() -> String {
    "rrf".to_string()
}

/// Default dimension for agent memory (384 for typical sentence transformers).
#[must_use]
pub const fn default_dimension() -> usize {
    384
}

// ============================================================================
// AgentMemory DTOs (EPIC-016 US-003)
// ============================================================================

/// Request to store knowledge in semantic memory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticStoreRequest {
    /// Unique ID for this knowledge fact.
    pub id: u64,
    /// Text content of the knowledge.
    pub content: String,
    /// Embedding vector for the content.
    pub embedding: Vec<f32>,
}

/// Request to query semantic memory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticQueryRequest {
    /// Query embedding vector.
    pub embedding: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

/// Result from semantic memory query.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticQueryResult {
    /// Knowledge fact ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Knowledge content text.
    pub content: String,
}

/// Request to record an episode in episodic memory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicRecordRequest {
    /// Episode event ID.
    pub event_id: u64,
    /// Episode description/content.
    pub content: String,
    /// Timestamp (epoch seconds).
    pub timestamp: i64,
    /// Embedding vector for the episode.
    pub embedding: Vec<f32>,
}

/// Request to query recent episodes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicRecentRequest {
    /// Number of recent episodes to return.
    #[serde(default = "default_top_k")]
    pub limit: usize,
    /// Only return episodes since this timestamp (epoch seconds).
    #[serde(default)]
    pub since_timestamp: Option<i64>,
}

/// Result from episodic memory query.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicResult {
    /// Episode ID.
    pub id: u64,
    /// Episode content.
    pub content: String,
    /// Timestamp (epoch seconds).
    pub timestamp: i64,
}

// ============================================================================
// ProceduralMemory DTOs
// ============================================================================

/// Default confidence for procedural learning.
#[must_use]
pub const fn default_confidence() -> f32 {
    1.0
}

/// Request to learn a procedure.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralLearnRequest {
    /// Procedure ID.
    pub procedure_id: u64,
    /// Procedure name.
    pub name: String,
    /// Steps to perform.
    pub steps: Vec<String>,
    /// Embedding vector for the procedure.
    pub embedding: Vec<f32>,
    /// Confidence level (0.0-1.0). Default: 1.0.
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

/// Request to recall procedures by similarity.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralRecallRequest {
    /// Query embedding vector.
    pub embedding: Vec<f32>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Minimum confidence threshold. Default: 0.0 (no filter).
    #[serde(default)]
    pub min_confidence: f32,
}

/// Result from procedural memory recall.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralMatchResult {
    /// Procedure ID.
    pub id: u64,
    /// Procedure name.
    pub name: String,
    /// Steps.
    pub steps: Vec<String>,
    /// Confidence score.
    pub confidence: f32,
    /// Similarity score from vector search.
    pub score: f32,
}

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
    /// Cursor from a previous scroll (omit for the first batch).
    #[serde(default)]
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
    /// Cursor for the next batch (absent when no more points).
    #[serde(skip_serializing_if = "Option::is_none")]
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
    /// Node label (or field name for secondary indexes).
    pub label: String,
    /// Property name.
    pub property: String,
    /// Index type (hash, range, or secondary).
    pub index_type: String,
    /// Number of unique values indexed.
    pub cardinality: usize,
    /// Memory usage in bytes.
    pub memory_bytes: usize,
}
