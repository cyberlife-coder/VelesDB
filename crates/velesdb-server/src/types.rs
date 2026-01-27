//! Request/Response types for VelesDB REST API.
//!
//! This module contains all the data transfer objects used by the API handlers.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ============================================================================
// Collection Types
// ============================================================================

/// Request to create a new collection.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCollectionRequest {
    /// Collection name.
    #[schema(example = "documents")]
    pub name: String,
    /// Vector dimension (required for vector collections, ignored for metadata_only).
    #[schema(example = 768)]
    pub dimension: Option<usize>,
    /// Distance metric (cosine, euclidean, dot, hamming, jaccard).
    #[serde(default = "default_metric")]
    #[schema(example = "cosine")]
    pub metric: String,
    /// Storage mode (full, sq8, binary). Defaults to full.
    #[serde(default = "default_storage_mode")]
    #[schema(example = "full")]
    pub storage_mode: String,
    /// Collection type: "vector" (default) or "metadata_only".
    #[serde(default = "default_collection_type")]
    #[schema(example = "vector")]
    pub collection_type: String,
}

fn default_collection_type() -> String {
    "vector".to_string()
}

fn default_metric() -> String {
    "cosine".to_string()
}

fn default_storage_mode() -> String {
    "full".to_string()
}

/// Response with collection information.
#[derive(Debug, Serialize, ToSchema)]
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

// ============================================================================
// Point Types
// ============================================================================

/// Request to upsert points.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertPointsRequest {
    /// Points to upsert.
    pub points: Vec<PointRequest>,
}

/// A point in an upsert request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PointRequest {
    /// Point ID.
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Optional payload.
    pub payload: Option<serde_json::Value>,
}

// ============================================================================
// Search Types
// ============================================================================

/// Request for vector search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchRequest {
    /// Query vector.
    pub vector: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Search mode preset: fast, balanced, accurate, perfect.
    /// Overrides ef_search with predefined values.
    #[serde(default)]
    #[schema(example = "balanced")]
    pub mode: Option<String>,
    /// HNSW ef_search parameter (higher = better recall, slower).
    /// Overrides mode if both are specified.
    #[serde(default)]
    #[schema(example = 128)]
    pub ef_search: Option<usize>,
    /// Query timeout in milliseconds.
    #[serde(default)]
    #[schema(example = 30000)]
    pub timeout_ms: Option<u64>,
    /// Optional metadata filter to apply to results (JSON object with condition).
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for batch vector search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchSearchRequest {
    /// List of search requests.
    pub searches: Vec<SearchRequest>,
}

fn default_top_k() -> usize {
    10
}

/// Convert mode string to ef_search value.
#[must_use]
pub fn mode_to_ef_search(mode: &str) -> Option<usize> {
    match mode.to_lowercase().as_str() {
        "fast" => Some(64),
        "balanced" => Some(128),
        "accurate" => Some(256),
        "perfect" => Some(usize::MAX),
        _ => None,
    }
}

/// Response from vector search.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    /// Search results.
    pub results: Vec<SearchResultResponse>,
}

/// Response from batch search.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchSearchResponse {
    /// Results for each search query.
    pub results: Vec<SearchResponse>,
    /// Total time in milliseconds.
    pub timing_ms: f64,
}

/// A single search result.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResultResponse {
    /// Point ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Error message.
    pub error: String,
}

/// Request for BM25 text search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TextSearchRequest {
    /// Text query for full-text search.
    #[schema(example = "rust programming")]
    pub query: String,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[schema(example = 10)]
    pub top_k: usize,
    /// Optional metadata filter to apply to results (JSON object with condition).
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for hybrid search (vector + text).
#[derive(Debug, Deserialize, ToSchema)]
pub struct HybridSearchRequest {
    /// Query vector for similarity search.
    pub vector: Vec<f32>,
    /// Text query for BM25 search.
    #[schema(example = "rust programming")]
    pub query: String,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[schema(example = 10)]
    pub top_k: usize,
    /// Weight for vector similarity (0.0-1.0). Text weight = 1 - vector_weight.
    #[serde(default = "default_vector_weight")]
    #[schema(example = 0.5)]
    pub vector_weight: f32,
    /// Optional metadata filter to apply to results (JSON object with condition).
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

fn default_vector_weight() -> f32 {
    0.5
}

/// Request for multi-query vector search with fusion.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MultiQuerySearchRequest {
    /// List of query vectors.
    pub vectors: Vec<Vec<f32>>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[schema(example = 10)]
    pub top_k: usize,
    /// Fusion strategy: "average", "maximum", "rrf", "weighted".
    #[serde(default = "default_fusion_strategy")]
    #[schema(example = "rrf")]
    pub strategy: String,
    /// RRF k parameter (only used when strategy = "rrf").
    #[serde(default = "default_rrf_k")]
    #[schema(example = 60)]
    pub rrf_k: u32,
    /// Weighted fusion: weight for average score component (default 0.5).
    #[serde(default = "default_avg_weight")]
    #[schema(example = 0.5)]
    pub avg_weight: f32,
    /// Weighted fusion: weight for max score component (default 0.3).
    #[serde(default = "default_max_weight")]
    #[schema(example = 0.3)]
    pub max_weight: f32,
    /// Weighted fusion: weight for hit count component (default 0.2).
    #[serde(default = "default_hit_weight")]
    #[schema(example = 0.2)]
    pub hit_weight: f32,
    /// Optional metadata filter to apply to results.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

fn default_fusion_strategy() -> String {
    "rrf".to_string()
}

fn default_rrf_k() -> u32 {
    60
}

fn default_avg_weight() -> f32 {
    0.5
}

fn default_max_weight() -> f32 {
    0.3
}

fn default_hit_weight() -> f32 {
    0.2
}

// ============================================================================
// Query Types
// ============================================================================

/// Request for `VelesQL` query execution.
#[derive(Debug, Deserialize, ToSchema)]
pub struct QueryRequest {
    /// The `VelesQL` query string.
    pub query: String,
    /// Named parameters for the query.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
}

/// Response from VelesQL query execution.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueryResponse {
    /// Query results.
    pub results: Vec<SearchResultResponse>,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
    /// Number of rows returned.
    pub rows_returned: usize,
}

/// Response from VelesQL aggregation query execution (BUG-1 FIX).
#[derive(Debug, Serialize, ToSchema)]
pub struct AggregationResponse {
    /// Aggregation results (JSON object or array for GROUP BY).
    pub result: serde_json::Value,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
}

/// VelesQL query error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueryErrorResponse {
    /// Error details.
    pub error: QueryErrorDetail,
}

// ============================================================================
// EXPLAIN Types (EPIC-058 US-002)
// ============================================================================

/// Request for query EXPLAIN.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExplainRequest {
    /// The `VelesQL` query string to explain.
    #[schema(example = "SELECT * FROM docs WHERE category = 'tech' AND vector NEAR $v LIMIT 10")]
    pub query: String,
    /// Named parameters for the query (optional, used for validation).
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
}

/// Response from query EXPLAIN.
#[derive(Debug, Serialize, ToSchema)]
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
}

/// A step in the query execution plan.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExplainStep {
    /// Step number (1-indexed).
    pub step: usize,
    /// Operation type (e.g., "VectorSearch", "Filter", "Sort").
    pub operation: String,
    /// Description of what this step does.
    pub description: String,
    /// Estimated rows processed/produced.
    pub estimated_rows: Option<usize>,
}

/// Estimated cost metrics for the query.
#[derive(Debug, Serialize, ToSchema)]
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
#[derive(Debug, Serialize, ToSchema)]
pub struct ExplainFeatures {
    /// Has vector search (NEAR clause).
    pub has_vector_search: bool,
    /// Has metadata filter (WHERE without NEAR).
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

/// VelesQL query error detail.
#[derive(Debug, Serialize, ToSchema)]
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

// ============================================================================
// Index Management Types (EPIC-009)
// ============================================================================

/// Request to create a property index.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateIndexRequest {
    /// Node label to index (e.g., "Person").
    #[schema(example = "Person")]
    pub label: String,
    /// Property name to index (e.g., "email").
    #[schema(example = "email")]
    pub property: String,
    /// Index type: "hash" (equality O(1)) or "range" (range queries O(log n)).
    #[serde(default = "default_index_type")]
    #[schema(example = "hash")]
    pub index_type: String,
}

fn default_index_type() -> String {
    "hash".to_string()
}

/// Response with index information.
#[derive(Debug, Serialize, ToSchema)]
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
#[derive(Debug, Serialize, ToSchema)]
pub struct ListIndexesResponse {
    /// List of indexes.
    pub indexes: Vec<IndexResponse>,
    /// Total number of indexes.
    pub total: usize,
}
