//! Graph types for VelesDB REST API.
//!
//! Contains request/response types for graph operations.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// A single traversal result item.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TraversalResultItem {
    /// Target node ID reached.
    pub target_id: u64,
    /// Depth of traversal (number of hops from source).
    pub depth: u32,
    /// Path taken (list of edge IDs).
    pub path: Vec<u64>,
}

/// Query parameters for edge operations.
#[derive(Debug, Deserialize, IntoParams)]
pub struct EdgeQueryParams {
    /// Filter edges by label (e.g., "KNOWS", "FOLLOWS").
    #[param(example = "KNOWS")]
    pub label: Option<String>,
}

/// Request for graph traversal.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TraverseRequest {
    /// Source node ID to start traversal from.
    pub source: u64,
    /// Traversal strategy: "bfs" or "dfs".
    #[serde(default = "default_strategy")]
    pub strategy: String,
    /// Maximum traversal depth.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Optional cursor for pagination (not implemented yet).
    pub cursor: Option<String>,
    /// Filter by relationship types (empty = all types).
    #[serde(default)]
    pub rel_types: Vec<String>,
}

fn default_strategy() -> String {
    "bfs".to_string()
}

fn default_max_depth() -> u32 {
    3
}

fn default_limit() -> usize {
    100
}

/// Response from graph traversal.
#[derive(Debug, Serialize, ToSchema)]
pub struct TraverseResponse {
    /// List of traversal results.
    pub results: Vec<TraversalResultItem>,
    /// Cursor for next page (if applicable).
    pub next_cursor: Option<String>,
    /// Whether more results are available.
    pub has_more: bool,
    /// Traversal statistics.
    pub stats: TraversalStats,
}

/// Statistics from traversal operation.
#[derive(Debug, Serialize, ToSchema)]
pub struct TraversalStats {
    /// Number of nodes visited.
    pub visited: usize,
    /// Maximum depth reached.
    pub depth_reached: u32,
}

/// Response for node degree query.
#[derive(Debug, Serialize, ToSchema)]
pub struct DegreeResponse {
    /// Number of incoming edges.
    pub in_degree: usize,
    /// Number of outgoing edges.
    pub out_degree: usize,
}

/// Response containing edges.
#[derive(Debug, Serialize, ToSchema)]
pub struct EdgesResponse {
    /// List of edges.
    pub edges: Vec<EdgeResponse>,
    /// Total count of edges returned.
    pub count: usize,
}

/// A single edge in the response.
#[derive(Debug, Serialize, ToSchema)]
pub struct EdgeResponse {
    /// Edge ID.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Edge label (relationship type).
    pub label: String,
    /// Edge properties.
    pub properties: serde_json::Value,
}

/// Request to add an edge to the graph.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddEdgeRequest {
    /// Edge ID.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Edge label (relationship type).
    pub label: String,
    /// Edge properties.
    #[serde(default)]
    pub properties: serde_json::Value,
}

// ============================================================================
// Edge Count, Node List, Node Payload, Parallel Traversal, Graph Search
// ============================================================================

/// Response for edge count query.
#[derive(Debug, Serialize, ToSchema)]
pub struct EdgeCountResponse {
    /// Total number of edges in the graph.
    pub count: usize,
}

/// Query parameters for node-scoped edge queries.
#[derive(Debug, Deserialize, IntoParams)]
pub struct NodeEdgeQueryParams {
    /// Filter by direction: "in", "out", or "both".
    #[serde(default = "default_direction")]
    #[param(example = "out")]
    pub direction: String,
    /// Filter edges by label.
    #[param(example = "KNOWS")]
    pub label: Option<String>,
}

fn default_direction() -> String {
    "out".to_string()
}

/// Response containing all node IDs in the graph.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodeListResponse {
    /// List of node IDs.
    pub node_ids: Vec<u64>,
    /// Total count of nodes.
    pub count: usize,
}

/// Request to upsert a node payload.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertNodePayloadRequest {
    /// JSON payload to store on the node.
    pub payload: serde_json::Value,
}

/// Response for a node payload retrieval.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodePayloadResponse {
    /// Node ID.
    pub node_id: u64,
    /// Stored payload (null if none).
    pub payload: Option<serde_json::Value>,
}

/// Request for parallel multi-source BFS traversal.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ParallelTraverseRequest {
    /// Source node IDs to start traversal from.
    pub sources: Vec<u64>,
    /// Maximum traversal depth.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Maximum number of results per source.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Filter by relationship types (empty = all types).
    #[serde(default)]
    pub rel_types: Vec<String>,
}

/// Request for graph embedding search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GraphSearchRequest {
    /// Query vector for similarity search.
    pub vector: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_graph_search_k")]
    pub top_k: usize,
}

fn default_graph_search_k() -> usize {
    10
}

/// Response for graph embedding search.
#[derive(Debug, Serialize, ToSchema)]
pub struct GraphSearchResponse {
    /// Search results with node ID and similarity score.
    pub results: Vec<GraphSearchResultItem>,
}

/// A single graph search result.
#[derive(Debug, Serialize, ToSchema)]
pub struct GraphSearchResultItem {
    /// Node ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Node payload (if any).
    pub payload: Option<serde_json::Value>,
}

// ============================================================================
// SSE Streaming Types (EPIC-058 US-003)
// ============================================================================

/// Query parameters for streaming graph traversal.
#[derive(Debug, Deserialize, IntoParams)]
pub struct StreamTraverseParams {
    /// Source node ID to start traversal from.
    #[param(example = 123)]
    pub start_node: u64,
    /// Traversal algorithm: "bfs" or "dfs".
    #[serde(default = "default_algorithm")]
    #[param(example = "bfs")]
    pub algorithm: String,
    /// Maximum traversal depth.
    #[serde(default = "default_stream_max_depth")]
    #[param(example = 5)]
    pub max_depth: u32,
    /// Maximum number of results to stream.
    #[serde(default = "default_stream_limit")]
    #[param(example = 1000)]
    pub limit: usize,
    /// Filter by relationship types (comma-separated).
    #[serde(default)]
    #[param(example = "KNOWS,FOLLOWS")]
    pub relationship_types: Option<String>,
}

fn default_algorithm() -> String {
    "bfs".to_string()
}

fn default_stream_max_depth() -> u32 {
    5
}

fn default_stream_limit() -> usize {
    1000
}

/// SSE event: A node reached during traversal.
#[derive(Debug, Serialize, ToSchema)]
pub struct StreamNodeEvent {
    /// Target node ID.
    pub id: u64,
    /// Depth from source.
    pub depth: u32,
    /// Path of edge IDs taken to reach this node.
    pub path: Vec<u64>,
}

/// SSE event: Periodic statistics update.
#[derive(Debug, Serialize, ToSchema)]
pub struct StreamStatsEvent {
    /// Number of nodes visited so far.
    pub nodes_visited: usize,
    /// Elapsed time in milliseconds.
    pub elapsed_ms: u64,
}

/// SSE event: Traversal completed.
#[derive(Debug, Serialize, ToSchema)]
pub struct StreamDoneEvent {
    /// Total nodes returned.
    pub total_nodes: usize,
    /// Maximum depth reached.
    pub max_depth_reached: u32,
    /// Total elapsed time in milliseconds.
    pub elapsed_ms: u64,
}

/// SSE event: Error occurred.
#[derive(Debug, Serialize, ToSchema)]
pub struct StreamErrorEvent {
    /// Error message.
    pub error: String,
}
