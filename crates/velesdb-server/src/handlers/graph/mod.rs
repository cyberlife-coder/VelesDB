//! Graph handlers for VelesDB REST API.
//!
//! All graph operations route through `AppState.db.get_graph_collection()`.
//! Graph data persists on disk via `GraphCollection` / `GraphEngine`.
//! [EPIC-016/US-031]

pub mod handlers;
pub mod handlers_extended;
pub mod stream;
pub mod types;

// Re-export public API — original handlers
pub use handlers::{add_edge, get_edges, get_node_degree, traverse_graph};
// Re-export public API — extended handlers (parity)
pub use handlers_extended::{
    get_edge_count, get_node_edges, get_node_payload, graph_search, list_nodes, remove_edge,
    traverse_parallel, upsert_node_payload,
};
pub use stream::stream_traverse;
#[allow(unused_imports)]
pub use types::{
    AddEdgeRequest, DegreeResponse, EdgeCountResponse, EdgeQueryParams, EdgeResponse,
    EdgesResponse, GraphSearchRequest, GraphSearchResponse, GraphSearchResultItem,
    NodeEdgeQueryParams, NodeListResponse, NodePayloadResponse, ParallelTraverseRequest,
    StreamDoneEvent, StreamErrorEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams,
    TraversalResultItem, TraversalStats, TraverseRequest, TraverseResponse,
    UpsertNodePayloadRequest,
};

#[cfg(test)]
mod tests {
    use super::types::*;
    use tempfile::tempdir;
    use velesdb_core::collection::graph::{GraphEdge, GraphSchema, TraversalConfig};
    use velesdb_core::GraphCollection;

    /// Creates an in-memory `GraphCollection` for testing (no Database needed).
    fn make_graph() -> (GraphCollection, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let coll = GraphCollection::create(
            dir.path().to_path_buf(),
            "test",
            None,
            velesdb_core::DistanceMetric::Cosine,
            GraphSchema::schemaless(),
        )
        .expect("create graph collection");
        (coll, dir)
    }

    fn add_test_edges(coll: &GraphCollection) {
        // Graph: 1 --KNOWS--> 2 --KNOWS--> 3 --KNOWS--> 4
        //                     |
        //                     +--WROTE--> 5
        for (id, src, tgt, lbl) in [
            (100, 1, 2, "KNOWS"),
            (101, 2, 3, "KNOWS"),
            (102, 3, 4, "KNOWS"),
            (103, 2, 5, "WROTE"),
        ] {
            coll.add_edge(GraphEdge::new(id, src, tgt, lbl).unwrap())
                .unwrap();
        }
    }

    #[test]
    fn test_graph_collection_add_and_get_edges() {
        let (coll, _dir) = make_graph();
        coll.add_edge(GraphEdge::new(1, 100, 200, "KNOWS").unwrap())
            .unwrap();
        let edges = coll.get_edges(Some("KNOWS"));
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

    #[test]
    fn test_traverse_bfs_basic() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let config = TraversalConfig::with_range(1, 3).with_limit(100);
        let results = coll.traverse_bfs(1, &config);
        assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
        assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 2));
        assert!(results.iter().any(|r| r.target_id == 4 && r.depth == 3));
        assert!(results.iter().any(|r| r.target_id == 5 && r.depth == 2));
    }

    #[test]
    fn test_traverse_bfs_with_limit() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let config = TraversalConfig::with_range(1, 5).with_limit(2);
        let results = coll.traverse_bfs(1, &config);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_traverse_bfs_with_rel_type_filter() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let config = TraversalConfig::with_range(1, 5)
            .with_limit(100)
            .with_rel_types(vec!["KNOWS".to_string()]);
        let results = coll.traverse_bfs(1, &config);
        assert!(!results.iter().any(|r| r.target_id == 5));
        assert!(results.iter().any(|r| r.target_id == 4));
    }

    #[test]
    fn test_traverse_dfs_basic() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let config = TraversalConfig::with_range(1, 3).with_limit(100);
        let results = coll.traverse_dfs(1, &config);
        assert!(results.iter().any(|r| r.target_id == 2));
        assert!(results.iter().any(|r| r.target_id == 3));
        assert!(results.iter().any(|r| r.target_id == 4));
    }

    #[test]
    fn test_traverse_dfs_with_limit() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let config = TraversalConfig::with_range(1, 5).with_limit(2);
        let results = coll.traverse_dfs(1, &config);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_get_node_degree() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let (in_deg, out_deg) = coll.node_degree(2);
        assert_eq!(in_deg, 1);
        assert_eq!(out_deg, 2);
        let (in_deg, out_deg) = coll.node_degree(1);
        assert_eq!(in_deg, 0);
        assert_eq!(out_deg, 1);
        let (in_deg, out_deg) = coll.node_degree(4);
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

    // =========================================================================
    // New handler tests (Phase 1+2 parity)
    // =========================================================================

    #[test]
    fn test_remove_edge() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        assert_eq!(coll.edge_count(), 4);
        assert!(coll.remove_edge(100));
        assert_eq!(coll.edge_count(), 3);
        assert!(!coll.remove_edge(999)); // non-existent
    }

    #[test]
    fn test_edge_count() {
        let (coll, _dir) = make_graph();
        assert_eq!(coll.edge_count(), 0);
        add_test_edges(&coll);
        assert_eq!(coll.edge_count(), 4);
    }

    #[test]
    fn test_all_node_ids() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        // all_node_ids returns IDs from edge store (source + target)
        let ids = coll.all_node_ids();
        // With edges: 1->2, 2->3, 3->4, 2->5, we should have nodes 1..5
        // Note: all_node_ids delegates to inner.all_ids() which returns
        // payload-stored IDs. Nodes referenced only by edges may not appear.
        // Store payloads to make them visible.
        coll.upsert_node_payload(1, &serde_json::json!({})).unwrap();
        coll.upsert_node_payload(2, &serde_json::json!({})).unwrap();
        let ids = coll.all_node_ids();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn test_node_edges_outgoing() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let out = coll.get_outgoing(2);
        assert_eq!(out.len(), 2); // 2->3 KNOWS, 2->5 WROTE
    }

    #[test]
    fn test_node_edges_incoming() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let inc = coll.get_incoming(3);
        assert_eq!(inc.len(), 1); // 2->3
        assert_eq!(inc[0].source(), 2);
    }

    #[test]
    fn test_node_payload_roundtrip() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let payload = serde_json::json!({"name": "Alice", "age": 30});
        coll.upsert_node_payload(1, &payload).unwrap();
        let retrieved = coll.get_node_payload(1).unwrap();
        assert_eq!(retrieved, Some(payload));
    }

    #[test]
    fn test_node_payload_missing() {
        let (coll, _dir) = make_graph();
        let retrieved = coll.get_node_payload(999).unwrap();
        assert_eq!(retrieved, None);
    }

    #[test]
    fn test_traverse_bfs_parallel() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let config = TraversalConfig::with_range(1, 2).with_limit(100);
        let results = coll.traverse_bfs_parallel(&[1, 3], &config);
        // From 1: reaches 2 (depth 1), 3,5 (depth 2)
        // From 3: reaches 4 (depth 1)
        assert!(results.iter().any(|r| r.target_id == 2));
        assert!(results.iter().any(|r| r.target_id == 4));
    }

    #[test]
    fn test_traverse_bfs_parallel_deduplicates() {
        let (coll, _dir) = make_graph();
        add_test_edges(&coll);
        let config = TraversalConfig::with_range(1, 3).with_limit(100);
        let results = coll.traverse_bfs_parallel(&[1, 2], &config);
        // Node 3 reachable from both 1 and 2 — should appear only once
        let count_3 = results.iter().filter(|r| r.target_id == 3).count();
        assert_eq!(count_3, 1);
    }

    #[test]
    fn test_edge_count_response_serialize() {
        let response = EdgeCountResponse { count: 42 };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("42"));
    }

    #[test]
    fn test_node_list_response_serialize() {
        let response = NodeListResponse {
            node_ids: vec![1, 2, 3],
            count: 3,
        };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("node_ids"));
    }

    #[test]
    fn test_node_payload_response_serialize() {
        let response = NodePayloadResponse {
            node_id: 1,
            payload: Some(serde_json::json!({"name": "Alice"})),
        };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("Alice"));
    }

    #[test]
    fn test_graph_search_response_serialize() {
        let response = GraphSearchResponse {
            results: vec![GraphSearchResultItem {
                id: 1,
                score: 0.95,
                payload: None,
            }],
        };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("0.95"));
    }
}
