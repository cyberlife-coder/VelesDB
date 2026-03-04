//! Graph handlers for VelesDB REST API.
//!
//! All graph operations route through `AppState.db.get_graph_collection()`.
//! Graph data persists on disk via `GraphCollection` / `GraphEngine`.
//! [EPIC-016/US-031]

mod handlers;
mod stream;
mod types;

// Re-export public API
pub use handlers::{add_edge, get_edges, get_node_degree, traverse_graph};
pub use stream::stream_traverse;
#[allow(unused_imports)]
pub use types::{
    AddEdgeRequest, DegreeResponse, EdgeQueryParams, EdgeResponse, EdgesResponse, StreamDoneEvent,
    StreamErrorEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams, TraversalResultItem,
    TraversalStats, TraverseRequest, TraverseResponse,
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
}
