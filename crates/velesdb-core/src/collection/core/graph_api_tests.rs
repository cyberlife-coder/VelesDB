//! Tests for graph_api.rs (EPIC-015 US-001, EPIC-041 coverage).

#[cfg(test)]
mod tests {
    use crate::collection::graph::{GraphEdge, TraversalConfig};
    use crate::collection::types::Collection;
    use crate::DistanceMetric;
    use tempfile::TempDir;

    fn create_test_collection() -> (Collection, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let collection =
            Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine)
                .expect("Failed to create collection");
        (collection, temp_dir)
    }

    fn make_edge(id: u64, source: u64, target: u64, label: &str) -> GraphEdge {
        GraphEdge::new(id, source, target, label).expect("edge should be valid")
    }

    // =========================================================================
    // Edge CRUD
    // =========================================================================

    #[test]
    fn test_add_edge_success() {
        let (collection, _temp) = create_test_collection();
        let edge = make_edge(1, 100, 200, "KNOWS");
        assert!(collection.add_edge(edge).is_ok());
    }

    #[test]
    fn test_add_duplicate_edge_fails() {
        let (collection, _temp) = create_test_collection();
        collection
            .add_edge(make_edge(1, 100, 200, "KNOWS"))
            .unwrap();
        let result = collection.add_edge(make_edge(1, 100, 200, "KNOWS"));
        assert!(result.is_err(), "duplicate edge ID should return error");
    }

    #[test]
    fn test_edge_count_empty() {
        let (collection, _temp) = create_test_collection();
        assert_eq!(collection.edge_count(), 0);
    }

    #[test]
    fn test_edge_count_after_adding() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "KNOWS")).unwrap();
        collection.add_edge(make_edge(2, 2, 3, "KNOWS")).unwrap();
        assert_eq!(collection.edge_count(), 2);
    }

    #[test]
    fn test_remove_edge_existing() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "KNOWS")).unwrap();
        assert!(collection.remove_edge(1), "should return true when removed");
        assert_eq!(collection.edge_count(), 0);
    }

    #[test]
    fn test_remove_edge_nonexistent() {
        let (collection, _temp) = create_test_collection();
        assert!(
            !collection.remove_edge(999),
            "should return false when not found"
        );
    }

    // =========================================================================
    // Edge queries
    // =========================================================================

    #[test]
    fn test_get_all_edges_empty() {
        let (collection, _temp) = create_test_collection();
        assert!(collection.get_all_edges().is_empty());
    }

    #[test]
    fn test_get_all_edges_returns_all() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "KNOWS")).unwrap();
        collection.add_edge(make_edge(2, 2, 3, "LIKES")).unwrap();
        let edges = collection.get_all_edges();
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_get_edges_by_label_matching() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "KNOWS")).unwrap();
        collection.add_edge(make_edge(2, 1, 3, "KNOWS")).unwrap();
        collection.add_edge(make_edge(3, 1, 4, "LIKES")).unwrap();

        let knows = collection.get_edges_by_label("KNOWS");
        assert_eq!(knows.len(), 2);
        assert!(knows.iter().all(|e| e.label() == "KNOWS"));
    }

    #[test]
    fn test_get_edges_by_label_no_match() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "KNOWS")).unwrap();
        let result = collection.get_edges_by_label("NONEXISTENT");
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_outgoing_edges() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 10, 20, "KNOWS")).unwrap();
        collection.add_edge(make_edge(2, 10, 30, "LIKES")).unwrap();
        collection.add_edge(make_edge(3, 20, 30, "KNOWS")).unwrap();

        let outgoing = collection.get_outgoing_edges(10);
        assert_eq!(outgoing.len(), 2);
        assert!(outgoing.iter().all(|e| e.source() == 10));
    }

    #[test]
    fn test_get_outgoing_edges_empty_for_unknown_node() {
        let (collection, _temp) = create_test_collection();
        assert!(collection.get_outgoing_edges(999).is_empty());
    }

    #[test]
    fn test_get_incoming_edges() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 10, 30, "KNOWS")).unwrap();
        collection.add_edge(make_edge(2, 20, 30, "LIKES")).unwrap();
        collection.add_edge(make_edge(3, 10, 20, "KNOWS")).unwrap();

        let incoming = collection.get_incoming_edges(30);
        assert_eq!(incoming.len(), 2);
        assert!(incoming.iter().all(|e| e.target() == 30));
    }

    #[test]
    fn test_get_incoming_edges_empty_for_unknown_node() {
        let (collection, _temp) = create_test_collection();
        assert!(collection.get_incoming_edges(999).is_empty());
    }

    // =========================================================================
    // Node degree
    // =========================================================================

    #[test]
    fn test_get_node_degree_zero() {
        let (collection, _temp) = create_test_collection();
        let (in_deg, out_deg) = collection.get_node_degree(1);
        assert_eq!(in_deg, 0);
        assert_eq!(out_deg, 0);
    }

    #[test]
    fn test_get_node_degree_out_only() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "KNOWS")).unwrap();
        collection.add_edge(make_edge(2, 1, 3, "KNOWS")).unwrap();
        let (in_deg, out_deg) = collection.get_node_degree(1);
        assert_eq!(in_deg, 0);
        assert_eq!(out_deg, 2);
    }

    #[test]
    fn test_get_node_degree_in_only() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 2, 5, "KNOWS")).unwrap();
        collection.add_edge(make_edge(2, 3, 5, "LIKES")).unwrap();
        let (in_deg, out_deg) = collection.get_node_degree(5);
        assert_eq!(in_deg, 2);
        assert_eq!(out_deg, 0);
    }

    #[test]
    fn test_get_node_degree_both() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 10, 20, "A")).unwrap();
        collection.add_edge(make_edge(2, 30, 20, "B")).unwrap();
        collection.add_edge(make_edge(3, 20, 40, "C")).unwrap();
        let (in_deg, out_deg) = collection.get_node_degree(20);
        assert_eq!(in_deg, 2);
        assert_eq!(out_deg, 1);
    }

    // =========================================================================
    // BFS traversal
    // =========================================================================

    fn build_chain(collection: &Collection) {
        // 1 -> 2 -> 3 -> 4
        collection.add_edge(make_edge(1, 1, 2, "NEXT")).unwrap();
        collection.add_edge(make_edge(2, 2, 3, "NEXT")).unwrap();
        collection.add_edge(make_edge(3, 3, 4, "NEXT")).unwrap();
    }

    #[test]
    fn test_traverse_bfs_basic() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let results = collection.traverse_bfs(1, 3, None, 100).unwrap();
        assert_eq!(results.len(), 3, "should reach nodes 2, 3, 4");
    }

    #[test]
    fn test_traverse_bfs_depth_limit() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let results = collection.traverse_bfs(1, 1, None, 100).unwrap();
        assert_eq!(results.len(), 1, "depth=1 should only reach node 2");
        assert_eq!(results[0].target_id, 2);
        assert_eq!(results[0].depth, 1);
    }

    #[test]
    fn test_traverse_bfs_label_filter() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "NEXT")).unwrap();
        collection.add_edge(make_edge(2, 1, 3, "OTHER")).unwrap();

        let results = collection.traverse_bfs(1, 3, Some(&["NEXT"]), 100).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, 2);
    }

    #[test]
    fn test_traverse_bfs_limit() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let results = collection.traverse_bfs(1, 10, None, 2).unwrap();
        assert!(results.len() <= 2, "limit should be respected");
    }

    #[test]
    fn test_traverse_bfs_empty_graph() {
        let (collection, _temp) = create_test_collection();
        let results = collection.traverse_bfs(1, 5, None, 100).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_traverse_bfs_no_cycles() {
        let (collection, _temp) = create_test_collection();
        // Cycle: 1 -> 2 -> 3 -> 1
        collection.add_edge(make_edge(1, 1, 2, "A")).unwrap();
        collection.add_edge(make_edge(2, 2, 3, "A")).unwrap();
        collection.add_edge(make_edge(3, 3, 1, "A")).unwrap();

        let results = collection.traverse_bfs(1, 10, None, 100).unwrap();
        // Should visit 2 and 3, but not revisit 1
        assert_eq!(results.len(), 2);
    }

    // =========================================================================
    // DFS traversal
    // =========================================================================

    #[test]
    fn test_traverse_dfs_basic() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let results = collection.traverse_dfs(1, 3, None, 100).unwrap();
        assert_eq!(results.len(), 3, "should reach nodes 2, 3, 4");
    }

    #[test]
    fn test_traverse_dfs_depth_limit() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let results = collection.traverse_dfs(1, 1, None, 100).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_traverse_dfs_label_filter() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "NEXT")).unwrap();
        collection.add_edge(make_edge(2, 1, 3, "OTHER")).unwrap();

        let results = collection.traverse_dfs(1, 3, Some(&["NEXT"]), 100).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, 2);
    }

    #[test]
    fn test_traverse_dfs_empty_graph() {
        let (collection, _temp) = create_test_collection();
        let results = collection.traverse_dfs(1, 5, None, 100).unwrap();
        assert!(results.is_empty());
    }

    // =========================================================================
    // TraversalConfig API
    // =========================================================================

    #[test]
    fn test_traverse_bfs_config_basic() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let config = TraversalConfig {
            max_depth: 3,
            min_depth: 0,
            rel_types: vec![],
            limit: 100,
        };
        let results = collection.traverse_bfs_config(1, &config);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_traverse_bfs_config_min_depth() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let config = TraversalConfig {
            max_depth: 3,
            min_depth: 2,
            rel_types: vec![],
            limit: 100,
        };
        let results = collection.traverse_bfs_config(1, &config);
        // min_depth=2 so only nodes at depth >= 2 are returned
        assert!(results.iter().all(|r| r.depth >= 2));
    }

    #[test]
    fn test_traverse_dfs_config_basic() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let config = TraversalConfig {
            max_depth: 3,
            min_depth: 0,
            rel_types: vec![],
            limit: 100,
        };
        let results = collection.traverse_dfs_config(1, &config);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_traverse_dfs_config_rel_filter() {
        let (collection, _temp) = create_test_collection();
        collection.add_edge(make_edge(1, 1, 2, "NEXT")).unwrap();
        collection.add_edge(make_edge(2, 1, 3, "OTHER")).unwrap();

        let config = TraversalConfig {
            max_depth: 2,
            min_depth: 0,
            rel_types: vec!["NEXT".to_string()],
            limit: 100,
        };
        let results = collection.traverse_dfs_config(1, &config);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, 2);
    }

    // =========================================================================
    // Graph schema / metadata
    // =========================================================================

    #[test]
    fn test_is_graph_false_for_plain_collection() {
        let (collection, _temp) = create_test_collection();
        assert!(!collection.is_graph());
    }

    #[test]
    fn test_has_embeddings_false_for_plain_collection() {
        let (collection, _temp) = create_test_collection();
        assert!(!collection.has_embeddings());
    }

    #[test]
    fn test_graph_schema_none_for_plain_collection() {
        let (collection, _temp) = create_test_collection();
        assert!(collection.graph_schema().is_none());
    }
}
