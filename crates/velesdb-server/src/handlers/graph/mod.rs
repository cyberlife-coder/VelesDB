//! Graph handlers for VelesDB REST API.
//!
//! All graph operations delegate to `Collection` methods from `velesdb-core`.
//! Graph data lives in the Collection's EdgeStore (in-memory; disk persistence planned).

mod handlers;
mod stream;
mod types;

// Re-export public API
pub use handlers::{add_edge, get_edges, get_node_degree, traverse_graph};
pub use stream::stream_traverse;
// Reason: types used only internally (AddEdgeRequest, EdgeQueryParams, etc.) stay private.
// Only types needed by lib.rs consumers are re-exported here.
pub use types::{
    DegreeResponse, StreamDoneEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams,
    TraversalResultItem, TraversalStats, TraverseRequest, TraverseResponse,
};

#[cfg(test)]
mod tests {
    use super::types::*;

    #[test]
    fn test_edges_response_serialize() {
        let response = EdgesResponse {
            edges: vec![EdgeResponse {
                id: 1,
                source: 100,
                target: 200,
                label: "KNOWS".to_string(),
                properties: serde_json::json!({}),
            }],
            count: 1,
        };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("KNOWS"));
    }

    #[test]
    fn test_traverse_response_serialize() {
        let response = TraverseResponse {
            results: vec![TraversalResultItem {
                target_id: 2,
                depth: 1,
                path: vec![100],
            }],
            next_cursor: None,
            has_more: false,
            stats: TraversalStats {
                visited: 1,
                depth_reached: 1,
            },
        };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("target_id"));
        assert!(json.contains("depth_reached"));
    }

    #[test]
    fn test_degree_response_serialize() {
        let response = DegreeResponse {
            in_degree: 5,
            out_degree: 10,
        };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("in_degree"));
        assert!(json.contains("out_degree"));
    }
}
