//! Knowledge Graph request/response DTOs extracted from `types.rs` (EPIC-015 US-001).
//!
//! Contains graph-related types: edges, traversal, node degree, and parallel traversal.

use serde::{Deserialize, Serialize};

// ============================================================================
// Default value functions (graph-specific)
// ============================================================================

pub(crate) fn default_max_depth() -> u32 {
    3
}

pub(crate) fn default_traverse_limit() -> usize {
    100
}

pub(crate) fn default_algorithm() -> String {
    "bfs".to_string()
}

// ============================================================================
// Knowledge Graph Types
// ============================================================================

/// Request to add an edge to the knowledge graph.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddEdgeRequest {
    /// Collection name.
    pub collection: String,
    /// Edge ID.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Edge label (relationship type).
    pub label: String,
    /// Optional edge properties.
    #[serde(default)]
    pub properties: Option<serde_json::Value>,
}

/// Request to get edges from the knowledge graph.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetEdgesRequest {
    /// Collection name.
    pub collection: String,
    /// Optional label filter.
    pub label: Option<String>,
    /// Optional source node filter.
    pub source: Option<u64>,
    /// Optional target node filter.
    pub target: Option<u64>,
}

/// Request to traverse the knowledge graph.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraverseGraphRequest {
    /// Collection name.
    pub collection: String,
    /// Starting node ID.
    pub source: u64,
    /// Maximum traversal depth.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Optional relationship type filter.
    pub rel_types: Option<Vec<String>>,
    /// Maximum number of results.
    #[serde(default = "default_traverse_limit")]
    pub limit: usize,
    /// Traversal algorithm: "bfs" or "dfs".
    #[serde(default = "default_algorithm")]
    pub algorithm: String,
}

/// Request to get node degree.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetNodeDegreeRequest {
    /// Collection name.
    pub collection: String,
    /// Node ID.
    pub node_id: u64,
}

/// Edge output for API responses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeOutput {
    /// Edge ID.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Edge label.
    pub label: String,
    /// Edge properties.
    pub properties: serde_json::Value,
}

/// Traversal result output.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraversalOutput {
    /// Target node ID reached.
    pub target_id: u64,
    /// Depth of traversal.
    pub depth: u32,
    /// Path taken (node IDs).
    pub path: Vec<u64>,
}

/// Node degree output.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDegreeOutput {
    /// Node ID.
    pub node_id: u64,
    /// Number of incoming edges.
    pub in_degree: usize,
    /// Number of outgoing edges.
    pub out_degree: usize,
}

/// Request for multi-source parallel BFS traversal.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraverseGraphParallelRequest {
    /// Collection name.
    pub collection: String,
    /// Source node IDs to start traversal from.
    pub sources: Vec<u64>,
    /// Maximum traversal depth.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Maximum number of results.
    #[serde(default = "default_traverse_limit")]
    pub limit: usize,
    /// Optional relationship types to follow.
    pub rel_types: Option<Vec<String>>,
}
