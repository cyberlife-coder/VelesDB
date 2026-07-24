//! Tests for graph_api.rs (EPIC-015 US-001, EPIC-041 coverage).

#[cfg(test)]
mod tests {
    use crate::collection::graph::{GraphEdge, TraversalConfig};
    use crate::collection::types::Collection;
    use crate::DistanceMetric;
    use std::time::{Duration, Instant};
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

    /// Test helper: stores empty payloads for both edge endpoints, then adds
    /// the edge. Schemaless `add_edge` requires endpoints to already exist
    /// (#1442), so tests that only care about edge/traversal mechanics use
    /// this instead of spelling out `store_node_payload` per endpoint.
    fn add_edge_with_nodes(collection: &Collection, edge: GraphEdge) -> crate::error::Result<()> {
        for node_id in [edge.source(), edge.target()] {
            collection.store_node_payload(node_id, &serde_json::json!({}))?;
        }
        collection.add_edge(edge)
    }

    // =========================================================================
    // Edge CRUD
    // =========================================================================

    #[test]
    fn test_add_edge_success() {
        let (collection, _temp) = create_test_collection();
        let edge = make_edge(1, 100, 200, "KNOWS");
        assert!(add_edge_with_nodes(&collection, edge).is_ok());
        assert_eq!(
            collection.edge_count(),
            1,
            "edge must be stored after add_edge"
        );
    }

    #[test]
    fn test_add_duplicate_edge_fails() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 100, 200, "KNOWS")).unwrap();
        let result = add_edge_with_nodes(&collection, make_edge(1, 100, 200, "KNOWS"));
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
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 2, 3, "KNOWS")).unwrap();
        assert_eq!(collection.edge_count(), 2);
    }

    #[test]
    fn test_remove_edge_existing() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "KNOWS")).unwrap();
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
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 2, 3, "LIKES")).unwrap();
        let edges = collection.get_all_edges();
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_get_edges_by_label_matching() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 1, 3, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(3, 1, 4, "LIKES")).unwrap();

        let knows = collection.get_edges_by_label("KNOWS");
        assert_eq!(knows.len(), 2);
        assert!(knows.iter().all(|e| e.label() == "KNOWS"));
    }

    #[test]
    fn test_get_edges_by_label_no_match() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "KNOWS")).unwrap();
        let result = collection.get_edges_by_label("NONEXISTENT");
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_outgoing_edges() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 10, 20, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 10, 30, "LIKES")).unwrap();
        add_edge_with_nodes(&collection, make_edge(3, 20, 30, "KNOWS")).unwrap();

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
        add_edge_with_nodes(&collection, make_edge(1, 10, 30, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 20, 30, "LIKES")).unwrap();
        add_edge_with_nodes(&collection, make_edge(3, 10, 20, "KNOWS")).unwrap();

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
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 1, 3, "KNOWS")).unwrap();
        let (in_deg, out_deg) = collection.get_node_degree(1);
        assert_eq!(in_deg, 0);
        assert_eq!(out_deg, 2);
    }

    #[test]
    fn test_get_node_degree_in_only() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 2, 5, "KNOWS")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 3, 5, "LIKES")).unwrap();
        let (in_deg, out_deg) = collection.get_node_degree(5);
        assert_eq!(in_deg, 2);
        assert_eq!(out_deg, 0);
    }

    #[test]
    fn test_get_node_degree_both() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 10, 20, "A")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 30, 20, "B")).unwrap();
        add_edge_with_nodes(&collection, make_edge(3, 20, 40, "C")).unwrap();
        let (in_deg, out_deg) = collection.get_node_degree(20);
        assert_eq!(in_deg, 2);
        assert_eq!(out_deg, 1);
    }

    // =========================================================================
    // BFS traversal
    // =========================================================================

    fn build_chain(collection: &Collection) {
        // 1 -> 2 -> 3 -> 4
        add_edge_with_nodes(collection, make_edge(1, 1, 2, "NEXT")).unwrap();
        add_edge_with_nodes(collection, make_edge(2, 2, 3, "NEXT")).unwrap();
        add_edge_with_nodes(collection, make_edge(3, 3, 4, "NEXT")).unwrap();
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
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "NEXT")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 1, 3, "OTHER")).unwrap();

        let results = collection.traverse_bfs(1, 3, Some(&["NEXT"]), 100).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, 2);
    }

    #[test]
    fn test_traverse_bfs_limit() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let results = collection.traverse_bfs(1, 10, None, 2).unwrap();
        assert_eq!(
            results.len(),
            2,
            "limit=2 must return exactly 2 of the 3 reachable nodes"
        );
        assert_eq!(results[0].target_id, 2);
        assert_eq!(results[1].target_id, 3); // node 4 is reachable but truncated by the limit
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
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "A")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 2, 3, "A")).unwrap();
        add_edge_with_nodes(&collection, make_edge(3, 3, 1, "A")).unwrap();

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
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "NEXT")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 1, 3, "OTHER")).unwrap();

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
            deadline: None,
        };
        let results = collection.traverse_bfs_config(1, &config);
        assert_eq!(
            results.len(),
            3,
            "chain 1->2->3->4 with max_depth=3, min_depth=0 should visit nodes 2, 3, 4"
        );
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
            deadline: None,
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
            deadline: None,
        };
        let results = collection.traverse_dfs_config(1, &config);
        assert_eq!(
            results.len(),
            3,
            "chain 1->2->3->4 with max_depth=3, min_depth=0 should visit nodes 2, 3, 4"
        );
    }

    #[test]
    fn test_traverse_dfs_config_rel_filter() {
        let (collection, _temp) = create_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "NEXT")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 1, 3, "OTHER")).unwrap();

        let config = TraversalConfig {
            max_depth: 2,
            min_depth: 0,
            rel_types: vec!["NEXT".to_string()],
            limit: 100,
            deadline: None,
        };
        let results = collection.traverse_dfs_config(1, &config);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, 2);
    }

    // =========================================================================
    // Wall-clock deadline on config traversal + GraphMetrics wiring
    // =========================================================================

    #[test]
    fn test_traverse_dfs_config_expired_deadline_returns_partial() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let config = TraversalConfig::with_range(1, 3)
            .with_limit(100)
            .with_deadline(
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .expect("test: clock before epoch"),
            );

        // Expired deadline aborts before expanding (counter seeded at threshold).
        let results = collection.traverse_dfs_config(1, &config);
        assert!(results.is_empty(), "expired deadline aborts DFS traversal");
    }

    #[test]
    fn test_traverse_bfs_config_expired_deadline_returns_partial() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let config = TraversalConfig::with_range(1, 3)
            .with_limit(100)
            .with_deadline(
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .expect("test: clock before epoch"),
            );

        let results = collection.traverse_bfs_config(1, &config);
        assert!(results.is_empty(), "expired deadline aborts BFS traversal");
    }

    #[test]
    fn test_traverse_config_far_future_deadline_no_premature_abort() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let future = Instant::now() + Duration::from_secs(3600);
        let with_deadline = TraversalConfig::with_range(1, 3)
            .with_limit(100)
            .with_deadline(future);
        let without = TraversalConfig::with_range(1, 3).with_limit(100);

        let a = collection.traverse_bfs_config(1, &with_deadline);
        let b = collection.traverse_bfs_config(1, &without);
        assert_eq!(a.len(), b.len(), "far-future deadline must not truncate");
        assert!(!a.is_empty());
    }

    #[test]
    fn test_traverse_bfs_config_records_metrics() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let before = collection.graph.edge_store.metrics().traversals_total();
        let config = TraversalConfig::with_range(1, 3).with_limit(100);
        let results = collection.traverse_bfs_config(1, &config);
        assert!(!results.is_empty());

        let metrics = collection.graph.edge_store.metrics();
        assert_eq!(
            metrics.traversals_total(),
            before + 1,
            "BFS config traversal increments the counter"
        );
        assert!(
            metrics.traversal_latency.count() > 0,
            "traversal latency observed"
        );
        assert!(
            metrics.traversal_nodes_visited() >= results.len() as u64,
            "nodes_visited recorded"
        );
    }

    #[test]
    fn test_traverse_dfs_config_records_metrics() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let before = collection.graph.edge_store.metrics().traversals_total();
        let config = TraversalConfig::with_range(1, 3).with_limit(100);
        let _ = collection.traverse_dfs_config(1, &config);

        assert_eq!(
            collection.graph.edge_store.metrics().traversals_total(),
            before + 1,
            "DFS config traversal increments the counter"
        );
    }

    // =========================================================================
    // Parallel BFS traversal
    // =========================================================================

    #[test]
    fn test_traverse_bfs_parallel_single_start() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let config = TraversalConfig {
            max_depth: 3,
            min_depth: 1,
            rel_types: vec![],
            limit: 100,
            deadline: None,
        };
        let results = collection.traverse_bfs_parallel(&[1], &config);
        assert!(!results.is_empty(), "parallel BFS should find neighbors");
        // Chain: 1->2->3->4, so we should get nodes 2, 3, 4
        let target_ids: std::collections::HashSet<u64> =
            results.iter().map(|r| r.target_id).collect();
        assert!(target_ids.contains(&2), "should reach node 2");
        assert!(target_ids.contains(&3), "should reach node 3");
        assert!(target_ids.contains(&4), "should reach node 4");
    }

    #[test]
    fn test_traverse_bfs_parallel_multiple_starts() {
        let (collection, _temp) = create_test_collection();
        // Two separate chains: 1->2->3 and 10->20->30
        add_edge_with_nodes(&collection, make_edge(1, 1, 2, "NEXT")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 2, 3, "NEXT")).unwrap();
        add_edge_with_nodes(&collection, make_edge(10, 10, 20, "NEXT")).unwrap();
        add_edge_with_nodes(&collection, make_edge(20, 20, 30, "NEXT")).unwrap();

        let config = TraversalConfig {
            max_depth: 2,
            min_depth: 1,
            rel_types: vec![],
            limit: 100,
            deadline: None,
        };
        let results = collection.traverse_bfs_parallel(&[1, 10], &config);

        let target_ids: std::collections::HashSet<u64> =
            results.iter().map(|r| r.target_id).collect();
        assert!(target_ids.contains(&2), "should reach node 2 from start 1");
        assert!(target_ids.contains(&3), "should reach node 3 from start 1");
        assert!(
            target_ids.contains(&20),
            "should reach node 20 from start 10"
        );
        assert!(
            target_ids.contains(&30),
            "should reach node 30 from start 10"
        );
    }

    #[test]
    fn test_traverse_bfs_parallel_empty_graph() {
        let (collection, _temp) = create_test_collection();
        let config = TraversalConfig::default();
        let results = collection.traverse_bfs_parallel(&[1], &config);
        // Start node itself has depth 0 (filtered by min_depth=1), so no results
        assert!(
            results.is_empty(),
            "empty graph parallel BFS should return no results"
        );
    }

    #[test]
    fn test_traverse_bfs_parallel_depth_limit() {
        let (collection, _temp) = create_test_collection();
        build_chain(&collection);

        let config = TraversalConfig {
            max_depth: 1,
            min_depth: 1,
            rel_types: vec![],
            limit: 100,
            deadline: None,
        };
        let results = collection.traverse_bfs_parallel(&[1], &config);
        assert!(results.iter().all(|r| r.depth <= 1));
        let target_ids: std::collections::HashSet<u64> =
            results.iter().map(|r| r.target_id).collect();
        assert!(target_ids.contains(&2), "should reach depth-1 neighbor");
        assert!(!target_ids.contains(&3), "should not reach depth-2 node");
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

    // =========================================================================
    // Issue #900 — node delete cascades to edges
    // =========================================================================

    fn create_graph_test_collection() -> (Collection, TempDir) {
        use crate::collection::graph::GraphSchema;
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let collection = Collection::create_graph_collection(
            temp_dir.path().to_path_buf(),
            "kg",
            GraphSchema::schemaless(),
            None,
            DistanceMetric::Cosine,
        )
        .expect("Failed to create graph collection");
        (collection, temp_dir)
    }

    #[test]
    fn test_delete_node_cascades_to_outgoing_edge() {
        // Insert nodes A=100, B=200 with an edge A -> B, delete A.
        let (collection, _temp) = create_graph_test_collection();
        collection
            .store_node_payload(100, &serde_json::json!({"name": "A"}))
            .unwrap();
        collection
            .store_node_payload(200, &serde_json::json!({"name": "B"}))
            .unwrap();
        add_edge_with_nodes(&collection, make_edge(1, 100, 200, "KNOWS")).unwrap();
        assert_eq!(collection.edge_count(), 1);

        collection.delete(&[100]).unwrap();

        // The edge must be gone: no outgoing from A, no incoming to B, and the
        // global edge count is zero (no dangling edge).
        assert_eq!(collection.edge_count(), 0, "edge should be cascaded away");
        assert!(collection.get_outgoing_edges(100).is_empty());
        assert!(
            collection.get_incoming_edges(200).is_empty(),
            "B must have no incoming edge to the deleted node A"
        );
        // Traversal from A returns nothing.
        let results = collection.traverse_bfs(100, 5, None, 100).unwrap();
        assert!(
            results.is_empty(),
            "traversal from deleted node yields nothing"
        );
    }

    #[test]
    fn test_delete_node_cascades_both_directions() {
        // A -> B and C -> B; deleting B must remove BOTH edges (outgoing from
        // others into B, i.e. incoming on the deleted node).
        let (collection, _temp) = create_graph_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 100, 200, "OUT")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 300, 200, "IN")).unwrap();
        // Also an outgoing edge from B so we cover both directions on the
        // deleted node itself.
        add_edge_with_nodes(&collection, make_edge(3, 200, 400, "OUT")).unwrap();
        assert_eq!(collection.edge_count(), 3);

        collection.delete(&[200]).unwrap();

        assert_eq!(
            collection.edge_count(),
            0,
            "all edges touching the deleted node must be gone"
        );
        assert!(collection.get_outgoing_edges(100).is_empty());
        assert!(collection.get_outgoing_edges(300).is_empty());
        assert!(collection.get_outgoing_edges(200).is_empty());
        assert!(collection.get_incoming_edges(400).is_empty());
    }

    #[test]
    fn test_delete_node_does_not_touch_unrelated_edges() {
        let (collection, _temp) = create_graph_test_collection();
        add_edge_with_nodes(&collection, make_edge(1, 100, 200, "A")).unwrap();
        add_edge_with_nodes(&collection, make_edge(2, 300, 400, "B")).unwrap();

        collection.delete(&[100]).unwrap();

        // Only the edge touching node 100 is removed.
        assert_eq!(collection.edge_count(), 1);
        assert_eq!(collection.get_outgoing_edges(300).len(), 1);
    }

    // =========================================================================
    // Issue #906 — eager traversal is bounded (no unbounded visited/parent map)
    // =========================================================================

    /// Builds a dense, highly-connected cyclic graph: every node points to the
    /// next `fanout` nodes (mod `n`), creating many cycles and a large frontier.
    fn build_dense_cyclic_graph(collection: &Collection, n: u64, fanout: u64) {
        // Bulk-create all n node payloads in one upsert (single fsync)
        // rather than re-storing each node's payload on every edge that
        // touches it (up to `fanout` redundant WAL fsyncs per node).
        let nodes: Vec<crate::point::Point> = (0..n)
            .map(|id| crate::point::Point::metadata_only(id, serde_json::json!({})))
            .collect();
        collection.upsert(nodes).expect("bulk-create nodes");

        let mut edge_id = 1u64;
        for src in 0..n {
            for step in 1..=fanout {
                let dst = (src + step) % n;
                collection
                    .add_edge(make_edge(edge_id, src, dst, "E"))
                    .unwrap();
                edge_id += 1;
            }
        }
    }

    #[test]
    fn test_eager_dfs_bounded_on_dense_cyclic_graph() {
        // A dense cyclic graph with an absurdly high depth/limit must still
        // terminate and stay bounded (visited cap prevents unbounded growth).
        let (collection, _temp) = create_graph_test_collection();
        build_dense_cyclic_graph(&collection, 200, 8);

        let config = TraversalConfig {
            max_depth: u32::MAX,
            min_depth: 0,
            rel_types: vec![],
            limit: usize::MAX,
            deadline: None,
        };
        let results = collection.traverse_dfs_config(0, &config);

        // Bounded by the number of distinct reachable nodes (199 targets, the
        // source itself is not emitted) and by MAX_VISITED_SIZE. The key
        // assertion is that the call terminates and never exceeds these bounds.
        assert!(
            results.len() <= crate::collection::graph::MAX_VISITED_SIZE,
            "DFS result must be bounded by the visited cap"
        );
        assert!(
            results.len() <= 199,
            "cannot exceed distinct reachable nodes"
        );
        assert!(
            !results.is_empty(),
            "should still traverse the reachable graph"
        );
    }

    #[test]
    fn test_expand_dfs_neighbors_bounds_push_growth() {
        // Regression (#906): a single high-out-degree hub must not push more
        // than `max_pending` neighbors into the stack / parent_map at PUSH time.
        // The pop-time `visited.len()` guard in `traverse_dfs_config` does NOT
        // cover this, because DFS records neighbors before they are popped.
        use crate::collection::core::graph_traversal_helpers::{expand_dfs_neighbors, DfsFrontier};
        use rustc_hash::{FxHashMap, FxHashSet};

        let (collection, _temp) = create_graph_test_collection();
        // Hub node 0 with 5_000 distinct out-edges. Bulk-create all endpoint
        // payloads in one upsert (single fsync) rather than fanout individual
        // store_node_payload calls (one WAL fsync each — a real hotspot at
        // this scale, see test_dfs_hub_stays_bounded_and_terminates).
        let fanout = 5_000u64;
        let nodes: Vec<crate::point::Point> = (0..=fanout)
            .map(|id| crate::point::Point::metadata_only(id, serde_json::json!({})))
            .collect();
        collection.upsert(nodes).expect("bulk-create nodes");
        for t in 1..=fanout {
            collection
                .add_edge(make_edge(t, 0, t, "E"))
                .expect("add edge");
        }

        let rel_filter: FxHashSet<&str> = FxHashSet::default();
        let visited: FxHashSet<u64> = FxHashSet::default();
        let mut stack: Vec<(u64, u32)> = Vec::new();
        let mut parent_map: FxHashMap<u64, (u64, u64)> = FxHashMap::default();
        let max_pending = 10usize;

        let mut frontier = DfsFrontier {
            stack: &mut stack,
            parent_map: &mut parent_map,
            max_pending,
        };
        expand_dfs_neighbors(
            collection.graph.edge_store.as_ref(),
            0,
            0,
            &rel_filter,
            &visited,
            &mut frontier,
        );

        assert!(
            parent_map.len() <= max_pending,
            "parent_map must be capped at max_pending, got {}",
            parent_map.len()
        );
        assert!(
            stack.len() <= max_pending,
            "stack must be capped at max_pending, got {}",
            stack.len()
        );
    }

    #[test]
    fn test_dfs_hub_stays_bounded_and_terminates() {
        // Regression (#906): DFS from a single very-high-out-degree hub with
        // min_depth past the graph's reach yields an empty result but must
        // terminate quickly without OOM. A star graph (hub -> many leaves) has
        // no depth-2 node, so with min_depth=2 the result is empty even though
        // the hub queues thousands of neighbors at push time.
        let (collection, _temp) = create_graph_test_collection();
        // Bulk-create endpoint payloads in one upsert (see
        // test_expand_dfs_neighbors_bounds_push_growth for why).
        let fanout = 20_000u64;
        let nodes: Vec<crate::point::Point> = (0..=fanout)
            .map(|id| crate::point::Point::metadata_only(id, serde_json::json!({})))
            .collect();
        collection.upsert(nodes).expect("bulk-create nodes");
        for t in 1..=fanout {
            collection
                .add_edge(make_edge(t, 0, t, "E"))
                .expect("add edge");
        }

        let config = TraversalConfig {
            max_depth: u32::MAX,
            min_depth: 2,
            rel_types: vec![],
            limit: usize::MAX,
            deadline: None,
        };
        let results = collection.traverse_dfs_config(0, &config);
        assert!(
            results.is_empty(),
            "star graph has no node at depth >= 2, result must be empty"
        );
    }

    #[test]
    fn test_eager_bfs_frontier_bounded_on_dense_cyclic_graph() {
        // traverse_bfs (frontier helper) on a dense cyclic graph with high
        // limit/depth must terminate and stay bounded.
        let (collection, _temp) = create_graph_test_collection();
        build_dense_cyclic_graph(&collection, 200, 8);

        let results = collection
            .traverse_bfs(0, u32::MAX, None, usize::MAX)
            .unwrap();

        assert!(
            results.len() <= crate::collection::graph::MAX_VISITED_SIZE,
            "BFS result must be bounded by the visited cap"
        );
        assert!(
            results.len() <= 199,
            "cannot exceed distinct reachable nodes"
        );
        assert!(!results.is_empty());
    }

    // =========================================================================
    // Referential integrity + schema enforcement (strict graph schema mode)
    // =========================================================================

    fn create_strict_graph_collection() -> (Collection, TempDir) {
        use crate::collection::graph::{EdgeType, GraphSchema, NodeType};
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let schema = GraphSchema::new()
            .with_node_type(NodeType::new("Person"))
            .with_node_type(NodeType::new("Company"))
            .with_edge_type(EdgeType::new("KNOWS", "Person", "Person"));
        let collection = Collection::create_graph_collection(
            temp_dir.path().to_path_buf(),
            "kg_strict",
            schema,
            None,
            DistanceMetric::Cosine,
        )
        .expect("Failed to create strict graph collection");
        (collection, temp_dir)
    }

    fn store_typed_node(collection: &Collection, id: u64, node_type: &str) {
        collection
            .store_node_payload(id, &serde_json::json!({ "_labels": [node_type] }))
            .expect("store node payload");
    }

    fn assert_schema_violation(result: crate::error::Result<()>) {
        match result {
            Err(crate::error::Error::SchemaValidation(_)) => {}
            other => panic!("expected SchemaValidation error, got {other:?}"),
        }
    }

    /// Asserts the error is `NodeNotFound(expected_id)` — the unified
    /// missing-endpoint contract (issue #1470): a genuinely missing endpoint
    /// is `NodeNotFound` in both schema modes, `SchemaValidation` being
    /// reserved for actual schema-shape violations (undeclared type, edge
    /// type / endpoint type mismatch).
    fn assert_node_not_found(result: crate::error::Result<()>, expected_id: u64) {
        match result {
            Err(crate::error::Error::NodeNotFound(id)) => assert_eq!(id, expected_id),
            other => panic!("expected NodeNotFound({expected_id}), got {other:?}"),
        }
    }

    #[test]
    fn test_strict_mode_rejects_dangling_edge() {
        // Issue #1470: a genuinely missing endpoint is NodeNotFound in
        // strict mode too, not SchemaValidation — that variant is reserved
        // for actual schema-shape violations (see the other
        // test_strict_mode_rejects_* below, which are unaffected).
        let (collection, _temp) = create_strict_graph_collection();
        // No node payloads stored: endpoints do not exist.
        assert_node_not_found(collection.add_edge(make_edge(1, 100, 200, "KNOWS")), 100);
        assert_eq!(collection.edge_count(), 0, "no partial write on rejection");
    }

    #[test]
    fn test_strict_mode_rejects_bad_edge_type() {
        let (collection, _temp) = create_strict_graph_collection();
        store_typed_node(&collection, 100, "Person");
        store_typed_node(&collection, 200, "Person");
        assert_schema_violation(collection.add_edge(make_edge(1, 100, 200, "UNKNOWN_REL")));
        assert_eq!(collection.edge_count(), 0);
    }

    #[test]
    fn test_strict_mode_rejects_endpoint_type_mismatch() {
        let (collection, _temp) = create_strict_graph_collection();
        store_typed_node(&collection, 100, "Person");
        store_typed_node(&collection, 200, "Company");
        // KNOWS is Person->Person; target is a Company.
        assert_schema_violation(collection.add_edge(make_edge(1, 100, 200, "KNOWS")));
        assert_eq!(collection.edge_count(), 0);
    }

    #[test]
    fn test_strict_mode_rejects_node_without_labels() {
        let (collection, _temp) = create_strict_graph_collection();
        // Node exists but carries no `_labels` -> type cannot be resolved.
        collection
            .store_node_payload(100, &serde_json::json!({ "name": "A" }))
            .unwrap();
        store_typed_node(&collection, 200, "Person");
        assert_schema_violation(collection.add_edge(make_edge(1, 100, 200, "KNOWS")));
        assert_eq!(collection.edge_count(), 0);
    }

    #[test]
    fn test_strict_mode_accepts_valid_edge() {
        let (collection, _temp) = create_strict_graph_collection();
        store_typed_node(&collection, 100, "Person");
        store_typed_node(&collection, 200, "Person");
        collection
            .add_edge(make_edge(1, 100, 200, "KNOWS"))
            .expect("valid edge should be accepted");
        assert_eq!(collection.edge_count(), 1);
    }

    #[test]
    fn test_schemaless_mode_rejects_dangling_edge() {
        // Regression guard (#1442): endpoint existence is required regardless
        // of schema strictness — a dangling edge would otherwise be invisible
        // to all_node_ids()/MATCH, which both resolve nodes from the payload
        // store rather than the edge store.
        let (collection, _temp) = create_graph_test_collection();
        let err = collection
            .add_edge(make_edge(1, 100, 200, "ANY_REL"))
            .expect_err("schemaless collection must reject edges to non-existent nodes");
        assert!(matches!(err, crate::error::Error::NodeNotFound(100)));
        assert_eq!(collection.edge_count(), 0, "no partial write on rejection");
    }

    #[test]
    fn test_schemaless_mode_rejects_edge_with_only_source_existing() {
        // Regression guard (#1442): validate_edge_endpoints_exist checks
        // BOTH endpoints — a partially-satisfied edge (only source stored)
        // must still be rejected, naming the missing target.
        let (collection, _temp) = create_graph_test_collection();
        collection
            .store_node_payload(100, &serde_json::json!({}))
            .expect("store source node");
        let err = collection
            .add_edge(make_edge(1, 100, 200, "ANY_REL"))
            .expect_err("edge with a missing target must be rejected");
        assert!(matches!(err, crate::error::Error::NodeNotFound(200)));
        assert_eq!(collection.edge_count(), 0, "no partial write on rejection");
    }

    #[test]
    fn test_schemaless_mode_rejects_edge_with_only_target_existing() {
        let (collection, _temp) = create_graph_test_collection();
        collection
            .store_node_payload(200, &serde_json::json!({}))
            .expect("store target node");
        let err = collection
            .add_edge(make_edge(1, 100, 200, "ANY_REL"))
            .expect_err("edge with a missing source must be rejected");
        assert!(matches!(err, crate::error::Error::NodeNotFound(100)));
        assert_eq!(collection.edge_count(), 0, "no partial write on rejection");
    }

    #[test]
    fn test_schemaless_mode_allows_edge_between_existing_nodes() {
        let (collection, _temp) = create_graph_test_collection();
        collection
            .store_node_payload(100, &serde_json::json!({}))
            .expect("store node 100");
        collection
            .store_node_payload(200, &serde_json::json!({}))
            .expect("store node 200");
        collection
            .add_edge(make_edge(1, 100, 200, "ANY_REL"))
            .expect("edge between existing nodes must be accepted");
        assert_eq!(collection.edge_count(), 1);
    }

    #[test]
    fn test_strict_mode_batch_rejects_dangling_edge() {
        // Issue #1470: same unification as test_strict_mode_rejects_dangling_edge,
        // for the batch path.
        let (collection, _temp) = create_strict_graph_collection();
        store_typed_node(&collection, 100, "Person");
        store_typed_node(&collection, 200, "Person");
        // First edge is valid, second references non-existent endpoints (300/400).
        let batch = vec![
            make_edge(1, 100, 200, "KNOWS"),
            make_edge(2, 300, 400, "KNOWS"),
        ];
        assert_node_not_found(collection.add_edges_batch(batch).map(|_| ()), 300);
        // Whole batch rejected before any mutation — no partial write.
        assert_eq!(
            collection.edge_count(),
            0,
            "a violating edge must fail the whole batch with no partial write"
        );
    }

    #[test]
    fn test_strict_mode_batch_rejects_bad_edge_type() {
        let (collection, _temp) = create_strict_graph_collection();
        store_typed_node(&collection, 100, "Person");
        store_typed_node(&collection, 200, "Person");
        let batch = vec![make_edge(1, 100, 200, "UNKNOWN_REL")];
        assert_schema_violation(collection.add_edges_batch(batch).map(|_| ()));
        assert_eq!(collection.edge_count(), 0);
    }

    #[test]
    fn test_strict_mode_batch_accepts_valid() {
        let (collection, _temp) = create_strict_graph_collection();
        store_typed_node(&collection, 100, "Person");
        store_typed_node(&collection, 200, "Person");
        store_typed_node(&collection, 300, "Person");
        let added = collection
            .add_edges_batch(vec![
                make_edge(1, 100, 200, "KNOWS"),
                make_edge(2, 200, 300, "KNOWS"),
            ])
            .expect("valid batch should be accepted in strict mode");
        assert_eq!(added, 2);
        assert_eq!(collection.edge_count(), 2);
    }

    #[test]
    fn test_schemaless_batch_rejects_dangling_edges() {
        // Regression guard (#1442): batch endpoint existence is required
        // regardless of schema strictness (see test_schemaless_mode_rejects_dangling_edge).
        let (collection, _temp) = create_graph_test_collection();
        let err = collection
            .add_edges_batch(vec![
                make_edge(1, 100, 200, "ANY_REL"),
                make_edge(2, 300, 400, "OTHER_REL"),
            ])
            .expect_err("schemaless batch must reject edges to non-existent nodes");
        assert!(matches!(err, crate::error::Error::NodeNotFound(100)));
        assert_eq!(
            collection.edge_count(),
            0,
            "a violating edge must fail the whole batch with no partial write"
        );
    }

    #[test]
    fn test_schemaless_batch_allows_edges_between_existing_nodes() {
        let (collection, _temp) = create_graph_test_collection();
        for id in [100, 200, 300, 400] {
            collection
                .store_node_payload(id, &serde_json::json!({}))
                .expect("store node");
        }
        let added = collection
            .add_edges_batch(vec![
                make_edge(1, 100, 200, "ANY_REL"),
                make_edge(2, 300, 400, "OTHER_REL"),
            ])
            .expect("batch between existing nodes must be accepted");
        assert_eq!(added, 2);
        assert_eq!(collection.edge_count(), 2);
    }

    // =========================================================================
    // EPIC-015: Strict-schema node-label validation in store_node_payload
    // =========================================================================

    fn create_strict_schema_collection() -> (Collection, TempDir) {
        use crate::collection::graph::{GraphSchema, NodeType};
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let schema = GraphSchema::new()
            .with_node_type(NodeType::new("Person"))
            .with_node_type(NodeType::new("Company"));
        let collection = Collection::create_graph_collection(
            temp_dir.path().to_path_buf(),
            "strict_kg",
            schema,
            None,
            DistanceMetric::Cosine,
        )
        .expect("Failed to create strict-schema collection");
        (collection, temp_dir)
    }

    #[test]
    fn test_store_node_payload_valid_label_accepted_in_strict_schema() {
        let (col, _tmp) = create_strict_schema_collection();
        let payload = serde_json::json!({"_labels": ["Person"], "name": "Alice"});
        assert!(
            col.store_node_payload(1, &payload).is_ok(),
            "declared label must be accepted"
        );
    }

    #[test]
    fn test_store_node_payload_undeclared_label_rejected_in_strict_schema() {
        let (col, _tmp) = create_strict_schema_collection();
        let payload = serde_json::json!({"_labels": ["Animal"], "name": "Dog"});
        let err = col
            .store_node_payload(1, &payload)
            .expect_err("undeclared label must be rejected");
        assert!(
            err.to_string().contains("Animal"),
            "error must name the offending label, got: {err}"
        );
    }

    #[test]
    fn test_store_node_payload_no_labels_allowed_in_strict_schema() {
        // Payloads without `_labels` cannot carry a type; they pass validation
        // (no type to reject) — the caller may add a typed payload later.
        let (col, _tmp) = create_strict_schema_collection();
        let payload = serde_json::json!({"name": "unlabelled"});
        assert!(
            col.store_node_payload(1, &payload).is_ok(),
            "payload without _labels must not be blocked by schema validation"
        );
    }

    #[test]
    fn test_store_node_payload_schemaless_accepts_any_label() {
        let (col, _tmp) = create_graph_test_collection();
        let payload = serde_json::json!({"_labels": ["ArbitraryType"], "data": 42});
        assert!(
            col.store_node_payload(1, &payload).is_ok(),
            "schemaless collection must accept any label"
        );
    }

    #[test]
    fn test_store_node_payload_multiple_labels_all_validated() {
        let (col, _tmp) = create_strict_schema_collection();
        // Both labels declared → ok
        let ok = serde_json::json!({"_labels": ["Person", "Company"]});
        assert!(col.store_node_payload(1, &ok).is_ok());
        // One undeclared label → rejected
        let bad = serde_json::json!({"_labels": ["Person", "Robot"]});
        let err = col
            .store_node_payload(2, &bad)
            .expect_err("second label is undeclared");
        assert!(err.to_string().contains("Robot"));
    }

    #[test]
    fn test_store_node_payload_rejected_does_not_mutate_state() {
        let (col, _tmp) = create_strict_schema_collection();
        let bad = serde_json::json!({"_labels": ["Alien"], "name": "ET"});
        col.store_node_payload(99, &bad).expect_err("must fail");
        // Node must not be stored
        assert!(
            col.get_node_payload(99)
                .expect("retrieve must not error")
                .is_none(),
            "failed write must leave no payload behind"
        );
    }

    // =========================================================================
    // Race regression (#1442 re-fix): a concurrent delete() of an edge
    // endpoint must never leave a phantom edge (edge present in the store,
    // endpoint payload gone). Deterministic interleaving: the test thread
    // holds `edge_wal_lock` itself so the writer (add_edge/add_edges_batch)
    // parks on it; post-fix the writer still holds `payload_storage`'s read
    // guard while parked there, so a racing delete() must block instead of
    // running ahead.
    // =========================================================================

    /// Runs one race iteration. `write` performs the edge write (via
    /// `add_edge` or `add_edges_batch`) on a fresh collection seeded with
    /// `source`/`target` payloads. Returns `(delete_raced_ahead, is_phantom)`:
    /// pre-fix, `delete()` is not blocked by the writer (raced ahead = true)
    /// and can produce a phantom edge; post-fix `delete()` must block on
    /// `payload_storage` until the writer finishes.
    fn race_write_vs_delete_iteration(
        iteration: u64,
        edge_id: u64,
        write: impl FnOnce(&Collection, GraphEdge) -> crate::error::Result<()> + Send + 'static,
    ) -> (bool, bool) {
        let (collection, _temp) = create_graph_test_collection();
        let source = iteration * 10 + 1;
        let target = iteration * 10 + 2;
        collection
            .store_node_payload(source, &serde_json::json!({}))
            .expect("seed source");
        collection
            .store_node_payload(target, &serde_json::json!({}))
            .expect("seed target");
        let collection = std::sync::Arc::new(collection);

        // Hold the WAL lock from the test thread so the writer parks on it
        // right after (post-fix) or well after releasing payload_storage
        // (pre-fix).
        let wal_guard = collection.graph.edge_wal_lock.lock();

        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let c1 = std::sync::Arc::clone(&collection);
        let edge = make_edge(edge_id, source, target, "REL");
        let t1 = std::thread::spawn(move || {
            started_tx.send(()).ok();
            write(&c1, edge)
        });
        started_rx.recv().expect("writer thread should start");
        std::thread::sleep(std::time::Duration::from_millis(30));

        let c2 = std::sync::Arc::clone(&collection);
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let t2 = std::thread::spawn(move || {
            let _ = c2.delete(&[source]);
            done_tx.send(()).ok();
        });
        let delete_raced_ahead = done_rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_ok();

        drop(wal_guard);
        t1.join().expect("writer thread must not panic").ok();
        t2.join().expect("delete thread must not panic");

        let edge_exists = collection.edge_exists(edge_id);
        let payload_present = collection
            .get_node_payload(source)
            .expect("retrieve must not error")
            .is_some();
        (delete_raced_ahead, edge_exists && !payload_present)
    }

    #[test]
    fn add_edge_concurrent_delete_never_leaves_phantom_edge() {
        for i in 0..10 {
            let (delete_raced_ahead, is_phantom) =
                race_write_vs_delete_iteration(i, i, Collection::add_edge);
            assert!(
                !delete_raced_ahead,
                "iteration {i}: delete() must block on payload_storage while \
                 add_edge holds its read guard through the WAL pause"
            );
            assert!(
                !is_phantom,
                "iteration {i}: phantom edge — edge exists but endpoint payload is gone"
            );
        }
    }

    #[test]
    fn add_edges_batch_concurrent_delete_never_leaves_phantom_edge() {
        for i in 0..10 {
            let (delete_raced_ahead, is_phantom) =
                race_write_vs_delete_iteration(i, i, |c, edge| {
                    c.add_edges_batch(vec![edge]).map(|_| ())
                });
            assert!(
                !delete_raced_ahead,
                "iteration {i}: delete() must block on payload_storage while \
                 add_edges_batch holds its read guard through the WAL pause"
            );
            assert!(
                !is_phantom,
                "iteration {i}: phantom edge — edge exists but endpoint payload is gone"
            );
        }
    }

    #[test]
    fn stress_add_edge_concurrent_delete_never_leaves_phantom_edge() {
        // Probabilistic filet (pattern: edge_concurrent_tests.rs) — no forced
        // rendezvous, just many yield_now-interleaved iterations to catch the
        // race by chance in addition to the deterministic tests above.
        for i in 0..2_000u64 {
            let (collection, _temp) = create_graph_test_collection();
            let source = i * 10 + 1;
            let target = i * 10 + 2;
            collection
                .store_node_payload(source, &serde_json::json!({}))
                .expect("seed source");
            collection
                .store_node_payload(target, &serde_json::json!({}))
                .expect("seed target");
            let collection = std::sync::Arc::new(collection);

            let c1 = std::sync::Arc::clone(&collection);
            let edge = make_edge(i, source, target, "REL");
            let h1 = std::thread::spawn(move || {
                std::thread::yield_now();
                let _ = c1.add_edge(edge);
            });
            let c2 = std::sync::Arc::clone(&collection);
            let h2 = std::thread::spawn(move || {
                std::thread::yield_now();
                let _ = c2.delete(&[source]);
            });
            h1.join().expect("add_edge thread must not panic");
            h2.join().expect("delete thread must not panic");

            let is_phantom =
                collection.edge_exists(i) && collection.get_node_payload(source).unwrap().is_none();
            assert!(!is_phantom, "iteration {i}: phantom edge under stress");
        }
    }
}
