//! Graph handlers for VelesDB REST API.
//!
//! Provides endpoints for graph operations including edge queries, traversal, and degree.
//! [EPIC-016/US-031]
//!
//! Note: Graph data is stored in a separate in-memory EdgeStore per collection.
//! This is managed by the GraphService state.

#![allow(dead_code)] // Handlers will be used when integrated into router

mod handlers;
mod service;
mod stream;
mod types;

// Re-export public API
pub use handlers::{add_edge, get_edges, get_node_degree, traverse_graph};
pub use service::GraphService;
pub use stream::stream_traverse;
#[allow(unused_imports)]
pub use types::{
    AddEdgeRequest, DegreeResponse, EdgeQueryParams, EdgeResponse, EdgesResponse, StreamDoneEvent,
    StreamErrorEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams, TraversalResultItem,
    TraversalStats, TraverseRequest, TraverseResponse,
};

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::collection::graph::GraphEdge;

    #[test]
    fn test_graph_service_add_and_get() {
        let service = GraphService::new();
        let edge = GraphEdge::new(1, 100, 200, "KNOWS")
            .expect("valid edge")
            .with_properties(std::collections::HashMap::new());

        service
            .add_edge("test_collection", edge)
            .expect("should add");

        let edges = service
            .get_edges_by_label("test_collection", "KNOWS")
            .expect("should get edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].label(), "KNOWS");
    }

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

    fn create_test_graph() -> GraphService {
        let service = GraphService::new();
        // Graph: 1 --KNOWS--> 2 --KNOWS--> 3 --KNOWS--> 4
        //                     |
        //                     +--WROTE--> 5
        service
            .add_edge(
                "test",
                GraphEdge::new(100, 1, 2, "KNOWS")
                    .unwrap()
                    .with_properties(std::collections::HashMap::new()),
            )
            .unwrap();
        service
            .add_edge(
                "test",
                GraphEdge::new(101, 2, 3, "KNOWS")
                    .unwrap()
                    .with_properties(std::collections::HashMap::new()),
            )
            .unwrap();
        service
            .add_edge(
                "test",
                GraphEdge::new(102, 3, 4, "KNOWS")
                    .unwrap()
                    .with_properties(std::collections::HashMap::new()),
            )
            .unwrap();
        service
            .add_edge(
                "test",
                GraphEdge::new(103, 2, 5, "WROTE")
                    .unwrap()
                    .with_properties(std::collections::HashMap::new()),
            )
            .unwrap();
        service
    }

    #[test]
    fn test_traverse_bfs_basic() {
        let service = create_test_graph();
        let results = service
            .traverse_bfs("test", 1, 3, 100, &[])
            .expect("should traverse");

        assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
        assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 2));
        assert!(results.iter().any(|r| r.target_id == 4 && r.depth == 3));
        assert!(results.iter().any(|r| r.target_id == 5 && r.depth == 2));
    }

    #[test]
    fn test_traverse_bfs_with_limit() {
        let service = create_test_graph();
        let results = service
            .traverse_bfs("test", 1, 5, 2, &[])
            .expect("should traverse");

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_traverse_bfs_with_rel_type_filter() {
        let service = create_test_graph();
        let results = service
            .traverse_bfs("test", 1, 5, 100, &["KNOWS".to_string()])
            .expect("should traverse");

        // Should not find node 5 (WROTE edge)
        assert!(!results.iter().any(|r| r.target_id == 5));
        // Should find nodes via KNOWS
        assert!(results.iter().any(|r| r.target_id == 4));
    }

    #[test]
    fn test_traverse_dfs_basic() {
        let service = create_test_graph();
        let results = service
            .traverse_dfs("test", 1, 3, 100, &[])
            .expect("should traverse");

        assert!(results.iter().any(|r| r.target_id == 2));
        assert!(results.iter().any(|r| r.target_id == 3));
        assert!(results.iter().any(|r| r.target_id == 4));
    }

    #[test]
    fn test_traverse_dfs_with_limit() {
        let service = create_test_graph();
        let results = service
            .traverse_dfs("test", 1, 5, 2, &[])
            .expect("should traverse");

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_get_node_degree() {
        let service = create_test_graph();

        // Node 2 has 1 incoming (from 1) and 2 outgoing (to 3 and 5)
        let (in_deg, out_deg) = service
            .get_node_degree("test", 2)
            .expect("should get degree");
        assert_eq!(in_deg, 1);
        assert_eq!(out_deg, 2);

        // Node 1 has 0 incoming and 1 outgoing
        let (in_deg, out_deg) = service
            .get_node_degree("test", 1)
            .expect("should get degree");
        assert_eq!(in_deg, 0);
        assert_eq!(out_deg, 1);

        // Node 4 has 1 incoming and 0 outgoing (leaf)
        let (in_deg, out_deg) = service
            .get_node_degree("test", 4)
            .expect("should get degree");
        assert_eq!(in_deg, 1);
        assert_eq!(out_deg, 0);
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
