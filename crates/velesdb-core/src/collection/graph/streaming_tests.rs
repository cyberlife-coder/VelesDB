//! Tests for `streaming` module - Streaming BFS traversal.

use super::streaming::*;
use super::{EdgeStore, GraphEdge, DEFAULT_MAX_DEPTH};

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
fn test_bfs_iterator_basic() {
    let store = create_test_edge_store();
    let config = StreamingConfig::default().with_max_depth(3);

    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
    assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 2));
    assert!(results.iter().any(|r| r.target_id == 4 && r.depth == 3));
    assert!(
        results.iter().any(|r| r.target_id == 5 && r.depth == 2),
        "Node 5 should be reachable at depth 2 via edge 2->5. Found nodes: {:?}",
        results
            .iter()
            .map(|r| (r.target_id, r.depth))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_bfs_iterator_multiple_outgoing_edges() {
    let store = create_test_edge_store();
    let config = StreamingConfig::default().with_max_depth(3);

    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    let targets: Vec<u64> = results.iter().map(|r| r.target_id).collect();

    assert!(targets.contains(&3), "Node 3 should be reachable via 2->3");
    assert!(targets.contains(&5), "Node 5 should be reachable via 2->5");

    assert_eq!(results.len(), 4, "Should have 4 results: nodes 2, 3, 5, 4");
}

#[test]
fn test_bfs_iterator_with_limit() {
    let store = create_test_edge_store();
    let config = StreamingConfig::default().with_max_depth(5).with_limit(2);

    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    assert_eq!(results.len(), 2);
}

#[test]
fn test_bfs_iterator_early_exit() {
    let store = create_test_edge_store();
    let config = StreamingConfig::default().with_max_depth(5).with_limit(1);

    let mut iter = BfsIterator::new(&store, 1, config);

    let first = iter.next();
    assert!(first.is_some());
    assert_eq!(iter.yielded_count(), 1);

    assert!(iter.next().is_none());
}

#[test]
fn test_bfs_iterator_rel_type_filter() {
    let store = create_test_edge_store();
    let config = StreamingConfig::default()
        .with_max_depth(5)
        .with_rel_types(vec!["KNOWS".to_string()]);

    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    assert!(!results.iter().any(|r| r.target_id == 5));
    assert!(results.iter().any(|r| r.target_id == 4));
}

#[test]
fn test_bfs_iterator_visited_overflow() {
    let store = create_test_edge_store();
    let config = StreamingConfig::default()
        .with_max_depth(5)
        .with_max_visited(2);

    let mut iter = BfsIterator::new(&store, 1, config);

    let mut count = 0;
    while iter.next().is_some() {
        count += 1;
        if count > 10 {
            break;
        }
    }

    assert!(iter.is_visited_overflow() || count <= 2);
}

#[test]
fn test_bfs_iterator_cyclic_graph() {
    let store = create_cyclic_edge_store();
    let config = StreamingConfig::default().with_max_depth(5).with_limit(10);

    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    assert!(results.len() <= 10);

    assert!(results.iter().any(|r| r.target_id == 2));
    assert!(results.iter().any(|r| r.target_id == 3));
}

#[test]
fn test_bfs_stream_convenience_function() {
    let store = create_test_edge_store();
    let config = StreamingConfig::default().with_max_depth(2);

    let results: Vec<_> = bfs_stream(&store, 1, config).collect();

    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.depth <= 2));
}

#[test]
fn test_streaming_config_defaults() {
    let config = StreamingConfig::default();

    assert_eq!(config.max_depth, DEFAULT_MAX_DEPTH);
    assert!(config.limit.is_none());
    assert_eq!(config.max_visited_size, 100_000);
    assert!(config.rel_types.is_empty());
}

// --- B-05 Regression Tests: BFS visited overflow must not produce duplicates ---

#[test]
fn test_bfs_no_duplicates_on_overflow() {
    // Cyclic graph: A→B→C→A with small max_visited to trigger overflow
    let store = create_cyclic_edge_store();
    let config = StreamingConfig::default()
        .with_max_depth(5)
        .with_max_visited(2)
        .with_limit(20);

    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    // Count occurrences of each target_id at each depth
    let mut seen: std::collections::HashSet<(u64, u32)> = std::collections::HashSet::new();
    for r in &results {
        assert!(
            seen.insert((r.target_id, r.depth)),
            "Duplicate result: node {} at depth {} — B-05 regression",
            r.target_id,
            r.depth
        );
    }
}

#[test]
fn test_bfs_overflow_preserves_visited() {
    // After overflow, nodes already in visited set must still be skipped
    let store = create_cyclic_edge_store();
    let config = StreamingConfig::default()
        .with_max_depth(3)
        .with_max_visited(2)
        .with_limit(20);

    let mut iter = BfsIterator::new(&store, 1, config);

    // Consume all results
    let mut results = Vec::new();
    for r in iter.by_ref() {
        results.push(r);
    }

    // Visited set should NOT be empty after overflow
    assert!(
        iter.visited_size() > 0,
        "Visited set should be preserved after overflow, not cleared"
    );
}
