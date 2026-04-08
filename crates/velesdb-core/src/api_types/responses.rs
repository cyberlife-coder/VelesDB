//! Canonical response DTOs for the `VelesDB` API.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use super::serde_id;

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
// EXPLAIN Responses
// ============================================================================

/// Response from query EXPLAIN.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainResponse {
    /// The original query.
    pub query: String,
    /// Query type (SELECT, MATCH, etc.).
    pub query_type: String,
    /// Target collection name.
    pub collection: String,
    /// Query plan steps.
    pub plan: Vec<ExplainStep>,
    /// Estimated cost metrics.
    pub estimated_cost: ExplainCost,
    /// Query features detected.
    pub features: ExplainFeatures,
    /// Whether this plan was served from the compiled plan cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub cache_hit: Option<bool>,
    /// How many times this cached plan has been reused.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub plan_reuse_count: Option<u64>,
    /// Estimated cost in milliseconds (for side-by-side comparison with actual).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub estimated_cost_ms: Option<f64>,
    /// Actual execution time in milliseconds (only when `analyze: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub actual_time_ms: Option<f64>,
    /// Actual execution statistics (only when `analyze: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub actual_stats: Option<ActualStatsResponse>,
    /// Per-node execution statistics (only when `analyze: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub node_stats: Option<Vec<NodeStatsResponse>>,
}

/// Actual execution statistics for EXPLAIN ANALYZE responses.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActualStatsResponse {
    /// Actual number of rows returned.
    pub actual_rows: u64,
    /// Actual execution time in milliseconds.
    pub actual_time_ms: f64,
    /// Number of loop iterations.
    pub loops: u64,
    /// Number of nodes visited (for graph traversal).
    pub nodes_visited: u64,
    /// Number of edges traversed.
    pub edges_traversed: u64,
}

#[cfg(feature = "persistence")]
impl From<&crate::velesql::ActualStats> for ActualStatsResponse {
    fn from(s: &crate::velesql::ActualStats) -> Self {
        Self {
            actual_rows: s.actual_rows,
            actual_time_ms: s.actual_time_ms,
            loops: s.loops,
            nodes_visited: s.nodes_visited,
            edges_traversed: s.edges_traversed,
        }
    }
}

/// Per-plan-node **estimated** execution statistics for EXPLAIN ANALYZE responses.
///
/// All values are synthetic heuristics derived from the plan-global
/// `actual_time_ms` — they are **not** individually measured per node.
/// Field names keep the `actual_` prefix for API stability; check the
/// `estimated` flag to distinguish heuristic values from future
/// instrumented measurements (#467).
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NodeStatsResponse {
    /// Node label (e.g. `VectorSearch`, `Filter`, `Limit`).
    pub node_label: String,
    /// Estimated wall-clock time for this node in milliseconds.
    /// Derived from total execution time using normalized weight fractions,
    /// not real per-node instrumentation. Will be replaced by measured
    /// timing (#467).
    pub actual_time_ms: f64,
    /// Estimated rows entering this node (heuristic approximation).
    pub actual_rows_in: u64,
    /// Estimated rows leaving this node (heuristic approximation).
    pub actual_rows_out: u64,
    /// Number of loop iterations (1 for non-looping nodes).
    pub loops: u64,
    /// When `true`, time and row counts are heuristic estimates, not real
    /// per-node measurements. Will become `false` once instrumented timing
    /// lands (#467).
    pub estimated: bool,
}

#[cfg(feature = "persistence")]
impl From<&crate::velesql::NodeStats> for NodeStatsResponse {
    fn from(ns: &crate::velesql::NodeStats) -> Self {
        Self {
            node_label: ns.node_label.clone(),
            actual_time_ms: ns.actual_time_ms,
            actual_rows_in: ns.actual_rows_in,
            actual_rows_out: ns.actual_rows_out,
            loops: ns.loops,
            estimated: ns.estimated,
        }
    }
}

/// A step in the query execution plan.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainStep {
    /// Step number (1-indexed).
    pub step: usize,
    /// Operation type.
    pub operation: String,
    /// Description of what this step does.
    pub description: String,
    /// Estimated rows processed/produced.
    pub estimated_rows: Option<usize>,
    /// How the row estimate was produced (e.g. "histogram", "cardinality", "heuristic").
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub estimation_method: Option<String>,
}

/// Estimated cost metrics for the query.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainCost {
    /// Whether an index can be used.
    pub uses_index: bool,
    /// Index name if used.
    pub index_name: Option<String>,
    /// Estimated selectivity (0.0 - 1.0).
    pub selectivity: f64,
    /// Estimated complexity class.
    pub complexity: String,
}

/// Features detected in the query.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainFeatures {
    /// Has vector search (NEAR clause).
    pub has_vector_search: bool,
    /// Has metadata filter.
    pub has_filter: bool,
    /// Has ORDER BY clause.
    pub has_order_by: bool,
    /// Has GROUP BY clause.
    pub has_group_by: bool,
    /// Has aggregation functions.
    pub has_aggregation: bool,
    /// Has JOIN clause.
    pub has_join: bool,
    /// Has FUSION clause.
    pub has_fusion: bool,
    /// LIMIT value if present.
    pub limit: Option<u64>,
    /// OFFSET value if present.
    pub offset: Option<u64>,
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
    #[serde(default)]
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
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScrollResponse {
    /// Points in this batch (ascending ID order).
    pub points: Vec<ScrollPoint>,
    /// Cursor for the next batch. Null when iteration is complete.
    pub next_cursor: Option<u64>,
}

/// A single point in a scroll response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScrollPoint {
    /// Point ID.
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
