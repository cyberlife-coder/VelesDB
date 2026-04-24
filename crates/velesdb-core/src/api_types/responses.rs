//! Canonical response DTOs for the `VelesDB` API.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use super::serde_id;

// Re-export EXPLAIN-related types for backward compatibility.
pub use super::responses_explain::*;

// ============================================================================
// Collection Responses
// ============================================================================

/// Response with collection information.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CollectionResponse {
    /// Collection name.
    pub name: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Distance metric.
    pub metric: String,
    /// Number of points in the collection.
    pub point_count: usize,
    /// Storage mode (full, sq8, binary).
    pub storage_mode: String,
}

/// Response with detailed collection configuration.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CollectionConfigResponse {
    /// Collection name.
    pub name: String,
    /// Vector dimension (0 for metadata-only collections).
    pub dimension: usize,
    /// Distance metric.
    pub metric: String,
    /// Storage mode (full, sq8, binary, pq).
    pub storage_mode: String,
    /// Number of points in the collection.
    pub point_count: usize,
    /// Whether this is a metadata-only collection.
    pub metadata_only: bool,
    /// Graph schema (if this is a graph collection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_schema: Option<serde_json::Value>,
    /// Embedding dimension for graph node vectors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_dimension: Option<usize>,
    /// On-disk schema version. Increments when the persisted
    /// `config.json` format changes in a way older `VelesDB` versions
    /// cannot safely read.
    #[serde(default)]
    pub schema_version: u32,
    /// PQ rescore oversampling factor. See `CollectionConfig` for
    /// the semantics of `None`, `Some(0)`, and `Some(n > 0)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pq_rescore_oversampling: Option<u32>,
    /// Persisted HNSW parameters (M, `ef_construction`, etc.) when
    /// customised at create time. `None` means the defaults inferred
    /// from the collection dimension are in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hnsw_params: Option<serde_json::Value>,
    /// Deferred indexing configuration (`US-366`) — `None` when the
    /// feature is disabled for this collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_indexing: Option<serde_json::Value>,
    /// Async index builder configuration (Issue `#488`) — `None` when
    /// the feature is disabled for this collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_index_builder: Option<serde_json::Value>,
}

// ============================================================================
// Search Responses
// ============================================================================

/// A single search result.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchResultResponse {
    /// Point ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Response from vector search.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchResponse {
    /// Search results.
    pub results: Vec<SearchResultResponse>,
}

/// Response from batch search.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BatchSearchResponse {
    /// Results for each search query.
    pub results: Vec<SearchResponse>,
    /// Total time in milliseconds.
    pub timing_ms: f64,
}

/// Response from IDs-only search (no payload hydration).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchIdsResponse {
    /// Search results with IDs and scores only.
    pub results: Vec<IdScoreResult>,
}

/// A single ID+score result from IDs-only search.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct IdScoreResult {
    /// Point ID.
    #[serde(
        serialize_with = "serde_id::serialize_id_as_string",
        deserialize_with = "serde_id::deserialize_id_from_string_or_number"
    )]
    pub id: u64,
    /// Similarity score.
    pub score: f32,
}

/// Error response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ErrorResponse {
    /// Error message.
    pub error: String,
    /// Structured error code (e.g., `"VELES-004"` for dimension mismatch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

// ============================================================================
// Query Responses
// ============================================================================

/// Query type for unified /query endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum QueryType {
    /// Vector similarity search.
    Search,
    /// Aggregation query (GROUP BY, COUNT, etc.).
    Aggregation,
    /// Simple SELECT returning rows.
    Rows,
    /// Graph pattern matching (MATCH clause).
    Graph,
    /// DDL statement (CREATE/DROP COLLECTION) — `VelesQL` v3.3.
    Ddl,
    /// DML mutation (INSERT/UPDATE/DELETE) — `VelesQL` v3.3.
    Dml,
}

/// Response from `VelesQL` query execution.
///
/// Results are projected rows: the shape depends on the SELECT clause.
/// - `SELECT *` → `{id, field1, field2, ...}` (no vector)
/// - `SELECT col1, col2` → `{col1, col2}`
/// - `SELECT similarity() AS score, title` → `{score, title}`
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct QueryResponse {
    /// Projected result rows. Shape depends on SELECT clause.
    pub results: Vec<serde_json::Value>,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
    /// Query execution time in whole milliseconds (compat helper).
    pub took_ms: u64,
    /// Number of rows returned.
    pub rows_returned: usize,
    /// Query response metadata.
    pub meta: QueryResponseMeta,
}

/// Metadata section for `VelesQL` query responses.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct QueryResponseMeta {
    /// `VelesQL` contract version used by this response.
    pub velesql_contract_version: String,
    /// Number of rows in `results`.
    pub count: usize,
}

/// Unified response from /query endpoint.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UnifiedQueryResponse {
    /// Type of query executed.
    #[serde(rename = "type")]
    pub query_type: QueryType,
    /// Number of results.
    pub count: usize,
    /// Execution time in milliseconds.
    pub timing_ms: f64,
    /// Results (structure depends on `query_type`).
    pub results: serde_json::Value,
    /// Optional warnings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Response from `VelesQL` aggregation query execution.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AggregationResponse {
    /// Aggregation results.
    pub result: serde_json::Value,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
    /// Query response metadata.
    pub meta: QueryResponseMeta,
}

/// `VelesQL` query error response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct QueryErrorResponse {
    /// Error details.
    pub error: QueryErrorDetail,
}

/// `VelesQL` query error detail.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct QueryErrorDetail {
    /// Error type.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Error message.
    pub message: String,
    /// Position in query where error occurred.
    pub position: usize,
    /// Fragment of query around error.
    pub query: String,
}

/// Standardized `VelesQL` semantic/runtime error detail.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VelesqlErrorDetail {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Actionable hint for developers.
    pub hint: String,
    /// Optional additional details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Standardized `VelesQL` semantic/runtime error response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VelesqlErrorResponse {
    /// Error details.
    pub error: VelesqlErrorDetail,
}

// ============================================================================
// Index Responses
// ============================================================================

/// Response with index information.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct IndexResponse {
    /// Node label.
    pub label: String,
    /// Property name.
    pub property: String,
    /// Index type (hash or range).
    pub index_type: String,
    /// Number of unique values indexed.
    pub cardinality: usize,
    /// Memory usage in bytes.
    pub memory_bytes: usize,
}

/// Response listing all indexes.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListIndexesResponse {
    /// List of indexes.
    pub indexes: Vec<IndexResponse>,
    /// Total number of indexes.
    pub total: usize,
}

// ============================================================================
// Scroll Responses
// ============================================================================

/// Request body for the scroll endpoint.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScrollRequest {
    /// Resume after this point ID (exclusive). Omit to start from beginning.
    #[serde(
        default,
        deserialize_with = "serde_id::deserialize_option_id_from_string_or_number"
    )]
    pub cursor: Option<u64>,
    /// Maximum points per batch (1–10 000, default 100).
    #[serde(default = "default_scroll_batch_size")]
    pub batch_size: u32,
    /// Optional filter expression.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Default batch size for scroll requests.
fn default_scroll_batch_size() -> u32 {
    100
}

/// Response from the scroll endpoint.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScrollResponse {
    /// Points in this batch (ascending ID order).
    pub points: Vec<ScrollPoint>,
    /// Cursor for the next batch. Null when iteration is complete.
    #[serde(
        serialize_with = "serde_id::serialize_option_id_as_string",
        deserialize_with = "serde_id::deserialize_option_id_from_string_or_number"
    )]
    pub next_cursor: Option<u64>,
}

/// A single point in a scroll response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScrollPoint {
    /// Point ID.
    #[serde(
        serialize_with = "serde_id::serialize_id_as_string",
        deserialize_with = "serde_id::deserialize_id_from_string_or_number"
    )]
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Optional payload.
    pub payload: Option<serde_json::Value>,
}

// ============================================================================
// Collection Statistics Responses
// ============================================================================

/// Response with collection statistics from ANALYZE.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CollectionStatsResponse {
    /// Total number of points in the collection.
    #[cfg_attr(feature = "openapi", schema(example = 50000))]
    pub total_points: u64,
    /// Total collection size in bytes.
    #[cfg_attr(feature = "openapi", schema(example = 102_400_000))]
    pub total_size_bytes: u64,
    /// Number of active rows.
    #[cfg_attr(feature = "openapi", schema(example = 49500))]
    pub row_count: u64,
    /// Number of deleted/tombstoned rows.
    #[cfg_attr(feature = "openapi", schema(example = 500))]
    pub deleted_count: u64,
    /// Average row size in bytes.
    #[cfg_attr(feature = "openapi", schema(example = 2048))]
    pub avg_row_size_bytes: u64,
    /// Total payload storage footprint in bytes.
    #[cfg_attr(feature = "openapi", schema(example = 5_120_000))]
    pub payload_size_bytes: u64,
    /// Timestamp of last ANALYZE (epoch milliseconds), or null if never analyzed.
    pub last_analyzed_epoch_ms: Option<u64>,
    /// Per-column statistics.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub column_stats: std::collections::HashMap<String, ColumnStatsResponse>,
    /// Per-index statistics.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub index_stats: std::collections::HashMap<String, IndexStatsResponse>,
}

/// Per-column statistics in a collection stats response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ColumnStatsResponse {
    /// Column name.
    pub name: String,
    /// Number of null values.
    pub null_count: u64,
    /// Number of distinct values (cardinality).
    pub distinct_count: u64,
    /// Minimum value (serialized string), if available.
    pub min_value: Option<String>,
    /// Maximum value (serialized string), if available.
    pub max_value: Option<String>,
    /// Average value size in bytes.
    pub avg_size_bytes: u64,
    /// Number of histogram buckets, or null if no histogram.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub histogram_buckets: Option<usize>,
    /// Whether the histogram is stale, or null if no histogram.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub histogram_stale: Option<bool>,
}

/// Per-index statistics in a collection stats response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct IndexStatsResponse {
    /// Index name.
    pub name: String,
    /// Index type (e.g., "HNSW", "`PropertyIndex`").
    pub index_type: String,
    /// Number of entries in the index.
    pub entry_count: u64,
    /// Index depth (for tree-based indexes).
    pub depth: u32,
    /// Index size in bytes.
    pub size_bytes: u64,
}

// ============================================================================
// GuardRails Responses
// ============================================================================

/// Response with current guard-rails configuration.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GuardRailsConfigResponse {
    /// Maximum graph traversal depth.
    pub max_depth: u32,
    /// Maximum intermediate cardinality.
    pub max_cardinality: usize,
    /// Memory limit per query in bytes.
    pub memory_limit_bytes: usize,
    /// Query timeout in milliseconds.
    pub timeout_ms: u64,
    /// Rate limit: max queries per second per client.
    pub rate_limit_qps: u32,
    /// Circuit breaker: failure threshold.
    pub circuit_failure_threshold: u32,
    /// Circuit breaker: recovery time in seconds.
    pub circuit_recovery_seconds: u64,
}
