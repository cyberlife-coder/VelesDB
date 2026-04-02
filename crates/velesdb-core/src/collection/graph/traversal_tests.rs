//! Tests for `traversal` module - Graph traversal algorithms.

use super::traversal::*;
use super::{EdgeStore, GraphEdge};

fn create_test_edge_store() -> EdgeStore {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(102, 3, 4, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(103, 2, 5, "WROTE").unwrap())
        .unwrap();
    store
}

fn create_cyclic_edge_store() -> EdgeStore {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(102, 3, 1, "KNOWS").unwrap())
        .unwrap();
    store
}

#[test]
fn test_bfs_single_hop() {
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 1);

    let results = bfs_traverse(&store, 1, &config);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].target_id, 2);
    assert_eq!(results[0].depth, 1);
}

#[test]
fn test_bfs_multi_hop() {
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 3);

    let results = bfs_traverse(&store, 1, &config);

    assert!(results.len() >= 4);
    assert!(results.iter().any(|r| r.target_id == 4 && r.depth == 3));
}

#[test]
fn test_bfs_with_rel_type_filter() {
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 3).with_rel_types(vec!["KNOWS".to_string()]);

    let results = bfs_traverse(&store, 1, &config);

    assert!(!results.iter().any(|r| r.target_id == 5));
    assert!(results.iter().any(|r| r.target_id == 4));
}

#[test]
fn test_bfs_min_depth() {
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(2, 3);

    let results = bfs_traverse(&store, 1, &config);

    assert!(!results.iter().any(|r| r.depth == 1));
    assert!(results.iter().any(|r| r.depth == 2));
    assert!(results.iter().any(|r| r.depth == 3));
}

#[test]
fn test_bfs_limit() {
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 3).with_limit(2);

    let results = bfs_traverse(&store, 1, &config);

    assert!(results.len() <= 2);
}

#[test]
fn test_bfs_reverse() {
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 2);

    let results = bfs_traverse_reverse(&store, 4, &config);

    assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 1));
    assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 2));
}

#[test]
fn test_default_max_depth() {
    assert_eq!(DEFAULT_MAX_DEPTH, 3);

    let config = TraversalConfig::default();
    assert_eq!(config.min_depth, 1);
    assert_eq!(config.max_depth, 3);
}

#[test]
fn test_path_tracking() {
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 2);

    let results = bfs_traverse(&store, 1, &config);

    let to_node_3 = results.iter().find(|r| r.target_id == 3 && r.depth == 2);
    assert!(to_node_3.is_some());

    let path = &to_node_3.unwrap().path;
    assert_eq!(path.len(), 2);
    assert_eq!(path[0], 100);
    assert_eq!(path[1], 101);
}

#[test]
fn test_with_range_respects_max_depth() {
    let config = TraversalConfig::with_range(1, 5);
    assert_eq!(config.max_depth, 5);

    let config = TraversalConfig::with_range(1, 10);
    assert_eq!(config.max_depth, 10);
}

#[test]
fn test_unbounded_range_applies_safety_cap() {
    let config = TraversalConfig::with_unbounded_range(1);
    assert_eq!(config.max_depth, SAFETY_MAX_DEPTH);
    assert_eq!(SAFETY_MAX_DEPTH, 100);
}

#[test]
fn test_bfs_cyclic_graph_no_infinite_loop() {
    let store = create_cyclic_edge_store();
    let config = TraversalConfig::with_range(1, 5).with_limit(100);

    let results = bfs_traverse(&store, 1, &config);

    assert!(results.len() < 100);

    let mut target_counts = std::collections::HashMap::new();
    for r in &results {
        *target_counts.entry(r.target_id).or_insert(0) += 1;
    }

    for (node_id, count) in &target_counts {
        assert_eq!(
            *count, 1,
            "Node {} appeared {} times, expected 1",
            node_id, count
        );
    }

    assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
    assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 2));
    assert!(results.iter().any(|r| r.target_id == 1 && r.depth == 3));
}

#[test]
fn test_with_max_depth_custom() {
    let config = TraversalConfig::default().with_max_depth(7);
    assert_eq!(config.max_depth, 7);
}

// =========================================================================
// Resolution 1: TraversalResult::path must be Vec<u64> at public API
// =========================================================================

#[test]
fn test_traversal_result_path_is_vec_u64() {
    // GIVEN: a traversal result built from a known graph
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 2);

    // WHEN: performing a BFS traversal
    let results = bfs_traverse(&store, 1, &config);

    // THEN: path field is Vec<u64> (compile-time type check)
    let result = results
        .iter()
        .find(|r| r.target_id == 3 && r.depth == 2)
        .expect("test: should find node 3 at depth 2");
    let path: Vec<u64> = result.path.clone();
    assert_eq!(path, vec![100, 101]);
}

#[test]
fn test_traversal_result_new_accepts_vec() {
    // GIVEN / WHEN: constructing a TraversalResult with Vec<u64>
    let path: Vec<u64> = vec![10, 20, 30];
    let result = TraversalResult::new(42, path.clone(), 3);

    // THEN: the path field round-trips correctly
    let recovered: Vec<u64> = result.path;
    assert_eq!(recovered, path);
}

// =========================================================================
// Resolution 2: bfs_traverse_both deduplicates by target_id
// =========================================================================

#[test]
fn test_bfs_traverse_both_dedup_by_target_id() {
    // GIVEN: a graph A->B->C and C->B (bidirectional path through B)
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 10, 20, "LINK").unwrap())
        .unwrap(); // A->B
    store
        .add_edge(GraphEdge::new(2, 20, 30, "LINK").unwrap())
        .unwrap(); // B->C
    store
        .add_edge(GraphEdge::new(3, 30, 20, "LINK").unwrap())
        .unwrap(); // C->B (reverse)

    let config = TraversalConfig::with_range(1, 2).with_limit(100);

    // WHEN: traverse_both from A with depth 2
    let results = bfs_traverse_both(&store, 10, &config);

    // THEN: each target node appears at most once in results
    let mut seen = std::collections::HashMap::new();
    for r in &results {
        *seen.entry(r.target_id).or_insert(0u32) += 1;
    }
    for (node_id, count) in &seen {
        assert_eq!(
            *count, 1,
            "Node {} appeared {} times in traverse_both, expected 1",
            node_id, count
        );
    }
    // Verify expected nodes are present
    assert!(
        results.iter().any(|r| r.target_id == 20),
        "Node B (20) should appear in results"
    );
    assert!(
        results.iter().any(|r| r.target_id == 30),
        "Node C (30) should appear in results"
    );
}

// =========================================================================
// G2: Parent-pointer path reconstruction correctness
// =========================================================================

#[test]
fn test_parent_pointer_path_matches_expected() {
    // GIVEN: a linear chain 1->2->3->4 with known edge IDs
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 3);

    // WHEN: BFS traversal uses parent-pointer reconstruction
    let results = bfs_traverse(&store, 1, &config);

    // THEN: depth-1 result (node 2) has path [100]
    let node2 = results
        .iter()
        .find(|r| r.target_id == 2 && r.depth == 1)
        .expect("test: node 2 at depth 1");
    assert_eq!(node2.path, vec![100], "1->2 via edge 100");

    // THEN: depth-2 result (node 3) has path [100, 101]
    let node3 = results
        .iter()
        .find(|r| r.target_id == 3 && r.depth == 2)
        .expect("test: node 3 at depth 2");
    assert_eq!(node3.path, vec![100, 101], "1->2->3 via edges 100,101");

    // THEN: depth-3 result (node 4) has path [100, 101, 102]
    let node4 = results
        .iter()
        .find(|r| r.target_id == 4 && r.depth == 3)
        .expect("test: node 4 at depth 3");
    assert_eq!(
        node4.path,
        vec![100, 101, 102],
        "1->2->3->4 via edges 100,101,102"
    );

    // THEN: depth-2 result (node 5) has path [100, 103] (branch from 2)
    let node5 = results
        .iter()
        .find(|r| r.target_id == 5 && r.depth == 2)
        .expect("test: node 5 at depth 2");
    assert_eq!(node5.path, vec![100, 103], "1->2->5 via edges 100,103");
}

#[test]
fn test_parent_pointer_reverse_path() {
    // GIVEN: chain 1->2->3->4, reverse from node 4
    let store = create_test_edge_store();
    let config = TraversalConfig::with_range(1, 3);

    // WHEN: reverse BFS from node 4
    let results = bfs_traverse_reverse(&store, 4, &config);

    // THEN: depth-1 result (node 3) has path [102] (edge 3->4 followed in reverse)
    let node3 = results
        .iter()
        .find(|r| r.target_id == 3 && r.depth == 1)
        .expect("test: node 3 at depth 1 (reverse)");
    assert_eq!(node3.path, vec![102], "4<-3 via edge 102");

    // THEN: depth-2 result (node 2) has path [102, 101]
    let node2 = results
        .iter()
        .find(|r| r.target_id == 2 && r.depth == 2)
        .expect("test: node 2 at depth 2 (reverse)");
    assert_eq!(node2.path, vec![102, 101], "4<-3<-2 via edges 102,101");
}

#[test]
fn test_parent_pointer_cyclic_graph_shortest_path() {
    // GIVEN: cycle 1->2->3->1
    let store = create_cyclic_edge_store();
    let config = TraversalConfig::with_range(1, 5).with_limit(100);

    // WHEN: BFS from node 1
    let results = bfs_traverse(&store, 1, &config);

    // THEN: each node appears exactly once (shortest path only)
    let mut target_counts = std::collections::HashMap::new();
    for r in &results {
        *target_counts.entry(r.target_id).or_insert(0) += 1;
    }
    for (node_id, count) in &target_counts {
        assert_eq!(
            *count, 1,
            "Node {} appeared {} times, expected 1 (parent-pointer BFS)",
            node_id, count
        );
    }

    // Verify paths are correct via parent pointers
    let node2 = results
        .iter()
        .find(|r| r.target_id == 2)
        .expect("test: node 2");
    assert_eq!(node2.path, vec![100], "1->2 via edge 100");

    let node3 = results
        .iter()
        .find(|r| r.target_id == 3)
        .expect("test: node 3");
    assert_eq!(node3.path, vec![100, 101], "1->2->3 via edges 100,101");
}
