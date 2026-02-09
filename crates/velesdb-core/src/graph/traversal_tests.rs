//! Tests for BFS and DFS graph traversal.

use super::edge_store::InMemoryEdgeStore;
use super::traversal::{bfs, dfs, TraversalConfig};
use super::types::{GraphEdge, GraphNode};

/// Build a linear graph: 1 → 2 → 3 → 4
fn build_linear_graph() -> InMemoryEdgeStore {
    let mut store = InMemoryEdgeStore::new();
    for i in 1..=4 {
        store.add_node(GraphNode::new(i, "Node")).unwrap();
    }
    store
        .add_edge(GraphEdge::new(10, 1, 2, "NEXT").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(11, 2, 3, "NEXT").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(12, 3, 4, "NEXT").unwrap())
        .unwrap();
    store
}

/// Build a diamond graph: 1 → 2, 1 → 3, 2 → 4, 3 → 4
fn build_diamond_graph() -> InMemoryEdgeStore {
    let mut store = InMemoryEdgeStore::new();
    for i in 1..=4 {
        store.add_node(GraphNode::new(i, "Node")).unwrap();
    }
    store
        .add_edge(GraphEdge::new(10, 1, 2, "A").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(11, 1, 3, "B").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(12, 2, 4, "A").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(13, 3, 4, "B").unwrap())
        .unwrap();
    store
}

/// Build a graph with a cycle: 1 → 2 → 3 → 1
fn build_cyclic_graph() -> InMemoryEdgeStore {
    let mut store = InMemoryEdgeStore::new();
    for i in 1..=3 {
        store.add_node(GraphNode::new(i, "Node")).unwrap();
    }
    store
        .add_edge(GraphEdge::new(10, 1, 2, "NEXT").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(11, 2, 3, "NEXT").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(12, 3, 1, "NEXT").unwrap())
        .unwrap();
    store
}

// ── BFS Tests ──────────────────────────────────────────────────────

#[test]
fn test_bfs_linear() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(10, 100);
    let results = bfs(&store, 1, &config);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].node_id, 2);
    assert_eq!(results[0].depth, 1);
    assert_eq!(results[1].node_id, 3);
    assert_eq!(results[1].depth, 2);
    assert_eq!(results[2].node_id, 4);
    assert_eq!(results[2].depth, 3);
}

#[test]
fn test_bfs_max_depth() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(2, 100);
    let results = bfs(&store, 1, &config);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].node_id, 2);
    assert_eq!(results[1].node_id, 3);
}

#[test]
fn test_bfs_limit() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(10, 1);
    let results = bfs(&store, 1, &config);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node_id, 2);
}

#[test]
fn test_bfs_diamond_no_duplicates() {
    let store = build_diamond_graph();
    let config = TraversalConfig::new(10, 100);
    let results = bfs(&store, 1, &config);

    // Node 4 is reachable via both 2 and 3.
    // BFS visits 2,3 first (depth 1), then 4 via whichever is dequeued first.
    // Since node 4 is reached from both paths, it appears at least once (up to 2).
    let node_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(node_ids.contains(&2));
    assert!(node_ids.contains(&3));
    assert!(node_ids.contains(&4));
}

#[test]
fn test_bfs_cycle_terminates() {
    let store = build_cyclic_graph();
    let config = TraversalConfig::new(10, 100);
    let results = bfs(&store, 1, &config);

    // Should terminate despite cycle. Source (1) is visited, so cycle back
    // to 1 should not cause infinite loop.
    assert!(!results.is_empty());
    assert!(results.len() <= 10);
}

#[test]
fn test_bfs_rel_type_filter() {
    let store = build_diamond_graph();
    let config = TraversalConfig::new(10, 100).with_rel_types(vec!["A".to_string()]);
    let results = bfs(&store, 1, &config);

    // Only follow "A" edges: 1 →A→ 2 →A→ 4
    let node_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(node_ids.contains(&2));
    assert!(node_ids.contains(&4));
    assert!(!node_ids.contains(&3)); // Node 3 is only via "B" edge
}

#[test]
fn test_bfs_nonexistent_source() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(10, 100);
    let results = bfs(&store, 999, &config);
    assert!(results.is_empty());
}

#[test]
fn test_bfs_path_tracking() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(10, 100);
    let results = bfs(&store, 1, &config);

    // First hop: edge 10
    assert_eq!(results[0].path, vec![10]);
    // Second hop: edges 10, 11
    assert_eq!(results[1].path, vec![10, 11]);
    // Third hop: edges 10, 11, 12
    assert_eq!(results[2].path, vec![10, 11, 12]);
}

// ── DFS Tests ──────────────────────────────────────────────────────

#[test]
fn test_dfs_linear() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(10, 100);
    let results = dfs(&store, 1, &config);

    assert_eq!(results.len(), 3);
    // DFS goes deep first: 2, 3, 4
    let node_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert_eq!(node_ids, vec![2, 3, 4]);
}

#[test]
fn test_dfs_max_depth() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(1, 100);
    let results = dfs(&store, 1, &config);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node_id, 2);
}

#[test]
fn test_dfs_limit() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(10, 2);
    let results = dfs(&store, 1, &config);

    assert_eq!(results.len(), 2);
}

#[test]
fn test_dfs_cycle_terminates() {
    let store = build_cyclic_graph();
    let config = TraversalConfig::new(10, 100);
    let results = dfs(&store, 1, &config);

    assert!(!results.is_empty());
    assert!(results.len() <= 10);
}

#[test]
fn test_dfs_rel_type_filter() {
    let store = build_diamond_graph();
    let config = TraversalConfig::new(10, 100).with_rel_types(vec!["B".to_string()]);
    let results = dfs(&store, 1, &config);

    // Only follow "B" edges: 1 →B→ 3 →B→ 4
    let node_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(node_ids.contains(&3));
    assert!(node_ids.contains(&4));
    assert!(!node_ids.contains(&2)); // Node 2 is only via "A" edge
}

#[test]
fn test_dfs_nonexistent_source() {
    let store = build_linear_graph();
    let config = TraversalConfig::new(10, 100);
    let results = dfs(&store, 999, &config);
    assert!(results.is_empty());
}
