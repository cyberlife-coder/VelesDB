//! Request/Response DTOs for Tauri commands.
//!
//! These types are used for communication between the frontend and backend
//! via Tauri's IPC system.

use serde::{Deserialize, Serialize};

// ============================================================================
// Request DTOs
// ============================================================================

/// Request to create a new collection.
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
}

/// Request to create a metadata-only collection.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMetadataCollectionRequest {
    /// Collection name.
    pub name: String,
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
    /// Fusion parameters (e.g., {"k": 60} for RRF, {"avgWeight": 0.6, ...} for weighted).
    #[serde(default)]
    pub fusion_params: Option<serde_json::Value>,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

// ============================================================================
// Response DTOs
// ============================================================================

/// Response for collection info.
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

/// Search result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Point ID.
    pub id: u64,
    /// Similarity/distance score.
    pub score: f32,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Multi-model query result (EPIC-031 US-012).
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

/// Response for `VelesQL` query operations.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResponse {
    /// Query results in multi-model format.
    pub results: Vec<HybridResult>,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
}

/// Point output for get operations.
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

/// Response for search operations.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    /// Search results.
    pub results: Vec<SearchResult>,
    /// Query time in milliseconds.
    pub timing_ms: f64,
}

// ============================================================================
// Default value functions
// ============================================================================

#[must_use]
pub fn default_metric() -> String {
    "cosine".to_string()
}

#[must_use]
pub fn default_storage_mode() -> String {
    "full".to_string()
}

#[must_use]
pub const fn default_top_k() -> usize {
    10
}

#[must_use]
pub const fn default_vector_weight() -> f32 {
    0.5
}

#[must_use]
pub fn default_fusion() -> String {
    "rrf".to_string()
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

/// Request to record an episode.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicRecordRequest {
    /// Episode description/content.
    pub content: String,
    /// Embedding vector for the episode.
    pub embedding: Vec<f32>,
    /// Optional context metadata.
    #[serde(default)]
    pub context: Option<serde_json::Value>,
}

/// Request to query recent episodes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicRecentRequest {
    /// Number of recent episodes to return.
    #[serde(default = "default_top_k")]
    pub limit: usize,
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
    pub timestamp: u64,
    /// Optional context.
    pub context: Option<serde_json::Value>,
}

/// Default dimension for agent memory (384 for typical sentence transformers).
#[must_use]
pub const fn default_dimension() -> usize {
    384
}
