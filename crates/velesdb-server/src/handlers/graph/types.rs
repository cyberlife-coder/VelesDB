//! Graph types for VelesDB REST API.
//!
//! Contains request/response types for graph operations.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use velesdb_core::api_types::serde_id;

/// A single traversal result item.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TraversalResultItem {
    /// Target node ID reached.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub target_id: u64,
    /// Depth of traversal (number of hops from source).
    pub depth: u32,
    /// Path taken (list of edge IDs).
    #[serde(serialize_with = "serde_id::serialize_ids_as_strings")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::ids_array_schema))]
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
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_input_schema))]
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
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub id: u64,
    /// Source node ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub source: u64,
    /// Target node ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
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
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_input_schema))]
    pub id: u64,
    /// Source node ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_input_schema))]
    pub source: u64,
    /// Target node ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_input_schema))]
    pub target: u64,
    /// Edge label (relationship type).
    pub label: String,
    /// Edge properties.
    #[serde(default)]
    pub properties: serde_json::Value,
}

/// Request to add multiple edges in one batched operation.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddEdgesBatchRequest {
    /// Edges to insert.
    pub edges: Vec<AddEdgeRequest>,
}

/// Response for a batched edge insertion.
#[derive(Debug, Serialize, ToSchema)]
pub struct AddEdgesBatchResponse {
    /// Number of edges inserted.
    pub added: usize,
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
    /// List of node IDs (serialized as strings to preserve u64 precision in JS).
    #[serde(serialize_with = "serde_id::serialize_ids_as_strings")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::ids_array_schema))]
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
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub node_id: u64,
    /// Stored payload (null if none).
    pub payload: Option<serde_json::Value>,
}

/// Request for parallel multi-source BFS traversal.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ParallelTraverseRequest {
    /// Source node IDs to start traversal from (accepts strings or numbers so
    /// JS clients can send precision-safe u64 IDs above `Number.MAX_SAFE_INTEGER`).
    #[serde(deserialize_with = "serde_id::deserialize_ids_from_string_or_number")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::ids_array_schema))]
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
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
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
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
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
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub id: u64,
    /// Depth from source.
    pub depth: u32,
    /// Path of edge IDs taken to reach this node.
    #[serde(serialize_with = "serde_id::serialize_ids_as_strings")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::ids_array_schema))]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Path edge IDs above 2^53 must serialize as a JSON string array.
    #[test]
    fn test_traversal_path_serialized_as_strings() {
        let above_safe = (1_u64 << 53) + 1; // 9_007_199_254_740_993
        let item = TraversalResultItem {
            target_id: 2,
            depth: 1,
            path: vec![above_safe],
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["path"], serde_json::json!(["9007199254740993"]));
    }

    /// Streamed node path edge IDs above 2^53 must serialize as strings.
    #[test]
    fn test_stream_node_event_path_serialized_as_strings() {
        let above_safe = (1_u64 << 53) + 1;
        let event = StreamNodeEvent {
            id: 1,
            depth: 1,
            path: vec![above_safe],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["path"], serde_json::json!(["9007199254740993"]));
    }

    /// Node-list ids above 2^53 must serialize as a JSON string array.
    #[test]
    fn test_node_list_ids_serialized_as_strings() {
        let above_safe = (1_u64 << 53) + 1;
        let response = NodeListResponse {
            node_ids: vec![1, above_safe],
            count: 2,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(
            json["node_ids"],
            serde_json::json!(["1", "9007199254740993"])
        );
    }

    /// Parallel-traverse sources must deserialize from BOTH strings and numbers.
    #[test]
    fn test_parallel_sources_accepts_strings_and_numbers() {
        let from_strings: ParallelTraverseRequest =
            serde_json::from_value(serde_json::json!({ "sources": ["9007199254740993", "2"] }))
                .expect("string sources must deserialize");
        assert_eq!(from_strings.sources, vec![(1_u64 << 53) + 1, 2]);

        let from_numbers: ParallelTraverseRequest =
            serde_json::from_value(serde_json::json!({ "sources": [3, 4] }))
                .expect("numeric sources must still deserialize");
        assert_eq!(from_numbers.sources, vec![3, 4]);
    }
}
