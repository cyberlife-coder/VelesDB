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
            deadline: None,
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

        let before = collection.edge_store.metrics().traversals_total();
        let config = TraversalConfig::with_range(1, 3).with_limit(100);
        let results = collection.traverse_bfs_config(1, &config);
        assert!(!results.is_empty());

        let metrics = collection.edge_store.metrics();
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

        let before = collection.edge_store.metrics().traversals_total();
        let config = TraversalConfig::with_range(1, 3).with_limit(100);
        let _ = collection.traverse_dfs_config(1, &config);

        assert_eq!(
            collection.edge_store.metrics().traversals_total(),
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
        collection.add_edge(make_edge(1, 1, 2, "NEXT")).unwrap();
        collection.add_edge(make_edge(2, 2, 3, "NEXT")).unwrap();
        collection.add_edge(make_edge(10, 10, 20, "NEXT")).unwrap();
        collection.add_edge(make_edge(20, 20, 30, "NEXT")).unwrap();

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
        collection
            .add_edge(make_edge(1, 100, 200, "KNOWS"))
            .unwrap();
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
        collection.add_edge(make_edge(1, 100, 200, "OUT")).unwrap();
        collection.add_edge(make_edge(2, 300, 200, "IN")).unwrap();
        // Also an outgoing edge from B so we cover both directions on the
        // deleted node itself.
        collection.add_edge(make_edge(3, 200, 400, "OUT")).unwrap();
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
        collection.add_edge(make_edge(1, 100, 200, "A")).unwrap();
        collection.add_edge(make_edge(2, 300, 400, "B")).unwrap();

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
        // Hub node 0 with 5_000 distinct out-edges.
        let fanout = 5_000u64;
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
            collection.edge_store.as_ref(),
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
        let fanout = 20_000u64;
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
}
