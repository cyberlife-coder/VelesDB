//! Request/Response types for VelesDB REST API.
//!
//! This module contains all the data transfer objects used by the API handlers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Canonical VelesQL contract version for REST responses.
pub const VELESQL_CONTRACT_VERSION: &str = "2.1.0";

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
    /// HNSW M parameter (number of bi-directional links per node).
    /// Higher values improve recall but increase memory and insert time.
    /// When omitted, auto-tuned based on dimension.
    #[serde(default)]
    #[schema(example = 32, nullable)]
    pub hnsw_m: Option<usize>,
    /// HNSW ef_construction parameter (candidate list size during build).
    /// Higher values improve recall but slow down indexing.
    /// When omitted, auto-tuned based on dimension.
    #[serde(default)]
    #[schema(example = 400, nullable)]
    pub hnsw_ef_construction: Option<usize>,
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
// Sparse Vector Types
// ============================================================================

/// Input format for sparse vectors, supporting two JSON representations:
///
/// - **Parallel arrays**: `{"indices": [42, 1337], "values": [0.5, 1.2]}`
/// - **Dict format** (Qdrant-compatible): `{"42": 0.5, "1337": 1.2}`
///
/// Both formats are accepted transparently via `#[serde(untagged)]`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum SparseVectorInput {
    /// Canonical parallel-array format with explicit indices and values.
    Parallel {
        /// Term/dimension indices (u32).
        indices: Vec<u32>,
        /// Corresponding weights.
        values: Vec<f32>,
    },
    /// Dictionary format mapping string term IDs to weights.
    Dict(BTreeMap<String, f32>),
}

impl SparseVectorInput {
    /// Converts this input into a core `SparseVector`, validating at the API boundary.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error string on:
    /// - Mismatched `indices`/`values` lengths (parallel format)
    /// - Non-finite values (NaN, Inf)
    /// - Dict keys that cannot be parsed as `u32`
    pub fn into_sparse_vector(self) -> Result<velesdb_core::index::sparse::SparseVector, String> {
        match self {
            Self::Parallel { indices, values } => {
                if indices.len() != values.len() {
                    return Err(format!(
                        "Sparse vector indices/values length mismatch: {} indices vs {} values",
                        indices.len(),
                        values.len()
                    ));
                }
                for (i, &v) in values.iter().enumerate() {
                    if !v.is_finite() {
                        return Err(format!(
                            "Sparse vector value at index {} is not finite: {v}",
                            i
                        ));
                    }
                }
                let pairs: Vec<(u32, f32)> = indices.into_iter().zip(values).collect();
                Ok(velesdb_core::index::sparse::SparseVector::new(pairs))
            }
            Self::Dict(map) => {
                let mut pairs = Vec::with_capacity(map.len());
                for (key, value) in map {
                    if !value.is_finite() {
                        return Err(format!(
                            "Sparse vector value for key '{key}' is not finite: {value}"
                        ));
                    }
                    let idx: u32 = key.parse().map_err(|_| {
                        format!("Sparse vector key '{key}' is not a valid u32 term ID")
                    })?;
                    pairs.push((idx, value));
                }
                Ok(velesdb_core::index::sparse::SparseVector::new(pairs))
            }
        }
    }
}

/// Fusion configuration for hybrid dense+sparse search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FusionRequest {
    /// Fusion strategy: "rrf" or "rsf".
    #[schema(example = "rrf")]
    pub strategy: String,
    /// RRF k parameter (only for strategy = "rrf", default 60).
    #[serde(default)]
    pub k: Option<u32>,
    /// Dense weight (only for strategy = "rsf", default 0.5).
    #[serde(default)]
    pub dense_w: Option<f32>,
    /// Sparse weight (only for strategy = "rsf", default 0.5).
    #[serde(default)]
    pub sparse_w: Option<f32>,
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
    /// Single sparse vector (convenience, stored under default name `""`).
    #[serde(default)]
    pub sparse_vector: Option<SparseVectorInput>,
    /// Named sparse vectors map (e.g., `{"title": {...}, "body": {...}}`).
    #[serde(default)]
    pub sparse_vectors: Option<BTreeMap<String, SparseVectorInput>>,
}

/// Request body for the streaming insert endpoint.
///
/// Accepts a single point to be pushed into the bounded ingestion channel.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamInsertRequest {
    /// Point ID.
    pub id: u64,
    /// Dense vector data.
    pub vector: Vec<f32>,
    /// Optional JSON payload (metadata).
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

// ============================================================================
// Search Types
// ============================================================================

/// Request for vector search.
///
/// Supports three modes based on which fields are provided:
/// - **Dense only**: `vector` only (existing behavior)
/// - **Sparse only**: `sparse_vector` only
/// - **Hybrid**: both `vector` and `sparse_vector` (fused via RRF/RSF)
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchRequest {
    /// Query vector for dense search (optional if sparse-only).
    #[serde(default)]
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
    /// Single sparse query vector (for sparse or hybrid search).
    #[serde(default)]
    pub sparse_vector: Option<SparseVectorInput>,
    /// Named sparse query vectors map.
    #[serde(default)]
    pub sparse_vectors: Option<BTreeMap<String, SparseVectorInput>>,
    /// Which named sparse index to search (default: `""`).
    #[serde(default)]
    pub sparse_index: Option<String>,
    /// Fusion configuration for hybrid search (default: RRF k=60).
    #[serde(default)]
    pub fusion: Option<FusionRequest>,
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

/// Response from IDs-only search (no payload hydration).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchIdsResponse {
    /// Search results with IDs and scores only.
    pub results: Vec<IdScoreResult>,
}

/// A single ID+score result from IDs-only search.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IdScoreResult {
    /// Point ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
}

/// Response with detailed collection configuration.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
    /// Optional collection name.
    /// Required for top-level MATCH queries executed via `/query`.
    #[serde(default)]
    pub collection: Option<String>,
}

/// Query type for unified /query endpoint (EPIC-052 US-006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum QueryType {
    /// Vector similarity search (contains similarity() or NEAR).
    Search,
    /// Aggregation query (GROUP BY, COUNT, SUM, etc.).
    Aggregation,
    /// Simple SELECT returning rows.
    Rows,
    /// Graph pattern matching (MATCH clause).
    Graph,
}

/// Response from VelesQL query execution.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueryResponse {
    /// Query results.
    pub results: Vec<SearchResultResponse>,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
    /// Query execution time in whole milliseconds (compat helper for API clients).
    pub took_ms: u64,
    /// Number of rows returned.
    pub rows_returned: usize,
    /// Query response metadata (contracted fields for SDK parity).
    pub meta: QueryResponseMeta,
}

/// Metadata section for VelesQL query responses.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueryResponseMeta {
    /// VelesQL contract version used by this response.
    pub velesql_contract_version: String,
    /// Number of rows in `results`.
    pub count: usize,
}

/// Unified response from /query endpoint (EPIC-052 US-006).
#[derive(Debug, Serialize, ToSchema)]
pub struct UnifiedQueryResponse {
    /// Type of query executed.
    #[serde(rename = "type")]
    pub query_type: QueryType,
    /// Number of results.
    pub count: usize,
    /// Execution time in milliseconds.
    pub timing_ms: f64,
    /// Results (structure depends on query_type).
    pub results: serde_json::Value,
    /// Optional warnings (e.g., truncated, deprecated).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Response from VelesQL aggregation query execution (BUG-1 FIX).
#[derive(Debug, Serialize, ToSchema)]
pub struct AggregationResponse {
    /// Aggregation results (JSON object or array for GROUP BY).
    pub result: serde_json::Value,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
    /// Query response metadata (contract version + count).
    pub meta: QueryResponseMeta,
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
    /// Whether this plan was served from the compiled plan cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable)]
    pub cache_hit: Option<bool>,
    /// How many times this cached plan has been reused.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable)]
    pub plan_reuse_count: Option<u64>,
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

/// Standardized VelesQL semantic/runtime error payload.
#[derive(Debug, Serialize, ToSchema)]
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

/// Standardized VelesQL semantic/runtime error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct VelesqlErrorResponse {
    /// Error details.
    pub error: VelesqlErrorDetail,
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

// ============================================================================
// Collection Statistics Types (EPIC-048)
// ============================================================================

/// Response with collection statistics from ANALYZE.
#[derive(Debug, Serialize, ToSchema)]
pub struct CollectionStatsResponse {
    /// Total number of points in the collection.
    #[schema(example = 50000)]
    pub total_points: u64,
    /// Total collection size in bytes.
    #[schema(example = 102400000)]
    pub total_size_bytes: u64,
    /// Number of active rows.
    #[schema(example = 49500)]
    pub row_count: u64,
    /// Number of deleted/tombstoned rows.
    #[schema(example = 500)]
    pub deleted_count: u64,
    /// Average row size in bytes.
    #[schema(example = 2048)]
    pub avg_row_size_bytes: u64,
    /// Total payload storage footprint in bytes.
    #[schema(example = 5120000)]
    pub payload_size_bytes: u64,
    /// Timestamp of last ANALYZE (epoch milliseconds), or null if never analyzed.
    pub last_analyzed_epoch_ms: Option<u64>,
    /// Per-column statistics.
    #[schema(value_type = Object)]
    pub column_stats: std::collections::HashMap<String, ColumnStatsResponse>,
    /// Per-index statistics.
    #[schema(value_type = Object)]
    pub index_stats: std::collections::HashMap<String, IndexStatsResponse>,
}

/// Per-column statistics in a collection stats response.
#[derive(Debug, Serialize, ToSchema)]
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
}

/// Per-index statistics in a collection stats response.
#[derive(Debug, Serialize, ToSchema)]
pub struct IndexStatsResponse {
    /// Index name.
    pub name: String,
    /// Index type (e.g., "HNSW", "PropertyIndex").
    pub index_type: String,
    /// Number of entries in the index.
    pub entry_count: u64,
    /// Index depth (for tree-based indexes).
    pub depth: u32,
    /// Index size in bytes.
    pub size_bytes: u64,
}

// ============================================================================
// GuardRails Types (EPIC-048)
// ============================================================================

/// Request to configure query guard-rails.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GuardRailsConfigRequest {
    /// Maximum graph traversal depth.
    #[schema(example = 10)]
    pub max_depth: Option<u32>,
    /// Maximum intermediate cardinality.
    #[schema(example = 100000)]
    pub max_cardinality: Option<usize>,
    /// Memory limit per query in bytes.
    #[schema(example = 104857600)]
    pub memory_limit_bytes: Option<usize>,
    /// Query timeout in milliseconds (0 = no timeout).
    #[schema(example = 30000)]
    pub timeout_ms: Option<u64>,
    /// Rate limit: max queries per second per client.
    #[schema(example = 100)]
    pub rate_limit_qps: Option<u32>,
    /// Circuit breaker: failure threshold before tripping.
    #[schema(example = 5)]
    pub circuit_failure_threshold: Option<u32>,
    /// Circuit breaker: recovery time in seconds.
    #[schema(example = 30)]
    pub circuit_recovery_seconds: Option<u64>,
}

/// Response with current guard-rails configuration.
#[derive(Debug, Serialize, ToSchema)]
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
