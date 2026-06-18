//! Tests for `traversal` module - Graph traversal algorithms.

use super::csr_snapshot::SnapshotBuilder;
use super::label_table::LabelTable;
use super::traversal::*;
use super::traversal_bidir::bfs_traverse_both;
use super::traversal_csr::bfs_traverse_csr;
use super::{EdgeStore, GraphEdge};
use std::time::{Duration, Instant};

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

    assert_eq!(
        results.len(),
        4,
        "reachable set {{2,3,4,5}} from node 1 within 1..3"
    );
    assert!(results.iter().any(|r| r.target_id == 4 && r.depth == 3));
    assert!(
        !results.iter().any(|r| r.target_id == 1),
        "source node must not be emitted"
    );
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

    assert_eq!(
        results.len(),
        2,
        "limit=2 must truncate a >=4-result traversal to exactly 2"
    );
    assert!(
        results.iter().any(|r| r.target_id == 2 && r.depth == 1),
        "node 2 (the sole depth-1 hop) must be present among the limited results"
    );
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

    // Nodes 2 and 3 are discovered at depths 1 and 2 respectively.
    assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
    assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 2));
    // Node 1 (source) is NOT re-emitted when the cycle closes (3→1),
    // because it was already visited at depth 0. Standard BFS semantics.
    assert!(
        !results.iter().any(|r| r.target_id == 1),
        "Source node should not appear in results (already visited)"
    );
}

#[test]
fn test_with_max_depth_caps_traversal() {
    let store = create_test_edge_store();
    // with_max_depth(1) must bound BFS to a single hop.
    let config = TraversalConfig::default().with_max_depth(1);
    assert_eq!(config.max_depth, 1, "builder sets the field");

    let results = bfs_traverse(&store, 1, &config);
    // Only node 2 (depth 1) is reachable; nodes 3 and 4 are beyond max_depth.
    assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
    assert!(!results.iter().any(|r| r.target_id == 3));
    assert!(!results.iter().any(|r| r.target_id == 4));
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

// =========================================================================
// Wall-clock deadline: eager BFS / CSR BFS abort cleanly (partial result)
// =========================================================================

#[test]
fn test_bfs_traverse_expired_deadline_returns_partial() {
    // GIVEN: a cyclic store and an already-expired deadline
    let store = create_cyclic_edge_store();
    let expired = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("test: clock before epoch");
    let config = TraversalConfig::with_range(1, 5)
        .with_limit(100)
        .with_deadline(expired);

    // WHEN: BFS runs with the expired deadline
    let results = bfs_traverse(&store, 1, &config);

    // THEN: it aborts immediately (counter seeded at threshold) with no hang.
    // The first pop triggers the check, so no neighbours are expanded.
    assert!(
        results.is_empty(),
        "expired deadline must abort before expanding, got {} results",
        results.len()
    );
}

#[test]
fn test_bfs_traverse_far_future_deadline_no_premature_abort() {
    // GIVEN: a far-future deadline (effectively disabled)
    let store = create_test_edge_store();
    let future = Instant::now() + Duration::from_secs(3600);
    let with_deadline = TraversalConfig::with_range(1, 3)
        .with_limit(100)
        .with_deadline(future);
    let without = TraversalConfig::with_range(1, 3).with_limit(100);

    // WHEN: BFS runs with and without the deadline
    let a = bfs_traverse(&store, 1, &with_deadline);
    let b = bfs_traverse(&store, 1, &without);

    // THEN: a far-future deadline yields the identical full result set.
    assert_eq!(a.len(), b.len(), "far-future deadline must not truncate");
    assert!(a.iter().any(|r| r.target_id == 4 && r.depth == 3));
}

#[test]
fn test_bfs_traverse_csr_expired_deadline_returns_partial() {
    // GIVEN: a CSR snapshot over a chain and an expired deadline
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
        .unwrap();
    let snapshot = SnapshotBuilder::build(&store, &LabelTable::new());

    let expired = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("test: clock before epoch");
    let config = TraversalConfig::with_range(1, 3)
        .with_limit(100)
        .with_deadline(expired);

    // WHEN / THEN: CSR BFS aborts immediately with a bounded (empty) result.
    let results = bfs_traverse_csr(&snapshot, 1, &config);
    assert!(results.is_empty(), "expired deadline aborts CSR BFS");
}

#[test]
fn test_bfs_traverse_csr_far_future_deadline_no_premature_abort() {
    // GIVEN: a CSR snapshot and a far-future deadline
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
        .unwrap();
    let snapshot = SnapshotBuilder::build(&store, &LabelTable::new());

    let future = Instant::now() + Duration::from_secs(3600);
    let with_deadline = TraversalConfig::with_range(1, 3).with_deadline(future);
    let without = TraversalConfig::with_range(1, 3);

    // WHEN / THEN: far-future deadline yields the same full result set.
    let a = bfs_traverse_csr(&snapshot, 1, &with_deadline);
    let b = bfs_traverse_csr(&snapshot, 1, &without);
    assert_eq!(a.len(), b.len());
    assert!(a.iter().any(|r| r.target_id == 3 && r.depth == 2));
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
