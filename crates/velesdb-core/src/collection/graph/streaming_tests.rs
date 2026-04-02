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

// =============================================================================
// G1: CSR snapshot BFS — verify identical results via zero-copy path
// =============================================================================

/// Creates a test store and builds its CSR snapshot for zero-copy BFS.
fn create_test_edge_store_with_csr() -> EdgeStore {
    let mut store = create_test_edge_store();
    store.build_read_snapshot();
    store
}

#[test]
fn test_bfs_csr_basic_same_as_legacy() {
    let legacy_store = create_test_edge_store();
    let csr_store = create_test_edge_store_with_csr();

    let config = StreamingConfig::default().with_max_depth(3);
    let legacy: Vec<_> = BfsIterator::new(&legacy_store, 1, config.clone()).collect();
    let csr: Vec<_> = BfsIterator::new(&csr_store, 1, config).collect();

    // Same number of results
    assert_eq!(legacy.len(), csr.len(), "CSR and legacy yield same count");

    // Same set of (target_id, depth) pairs
    let mut legacy_set: Vec<(u64, u32)> = legacy.iter().map(|r| (r.target_id, r.depth)).collect();
    let mut csr_set: Vec<(u64, u32)> = csr.iter().map(|r| (r.target_id, r.depth)).collect();
    legacy_set.sort_unstable();
    csr_set.sort_unstable();
    assert_eq!(legacy_set, csr_set, "CSR and legacy produce same targets");
}

#[test]
fn test_bfs_csr_rel_type_filter() {
    let mut store = create_test_edge_store();
    store.build_read_snapshot();

    let config = StreamingConfig::default()
        .with_max_depth(5)
        .with_rel_types(vec!["KNOWS".to_string()]);

    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    // Node 5 is via WROTE edge, should be filtered out
    assert!(!results.iter().any(|r| r.target_id == 5));
    // Node 4 is via KNOWS chain: 1->2->3->4
    assert!(results.iter().any(|r| r.target_id == 4));
}

#[test]
fn test_bfs_csr_cyclic_graph() {
    let mut store = create_cyclic_edge_store();
    store.build_read_snapshot();

    let config = StreamingConfig::default().with_max_depth(5).with_limit(10);
    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    assert!(results.len() <= 10);
    assert!(results.iter().any(|r| r.target_id == 2));
    assert!(results.iter().any(|r| r.target_id == 3));
}

#[test]
fn test_bfs_csr_with_limit() {
    let mut store = create_test_edge_store();
    store.build_read_snapshot();

    let config = StreamingConfig::default().with_max_depth(5).with_limit(2);
    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_bfs_csr_path_contains_edge_ids() {
    let mut store = create_test_edge_store();
    store.build_read_snapshot();

    let config = StreamingConfig::default().with_max_depth(3);
    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    // Node 2 at depth 1 should have path [100] (edge ID 100: 1->2)
    let node2 = results.iter().find(|r| r.target_id == 2).expect("node 2");
    assert_eq!(node2.path.len(), 1);
    assert_eq!(node2.path[0], 100, "path through edge 100 (1->2)");

    // Node 3 at depth 2 should have path [100, 101] (1->2->3)
    let node3 = results.iter().find(|r| r.target_id == 3).expect("node 3");
    assert_eq!(node3.path.len(), 2);
    assert_eq!(node3.path[0], 100, "first hop via edge 100");
    assert_eq!(node3.path[1], 101, "second hop via edge 101");
}

// =============================================================================
// G2: Parent-pointer path reconstruction — streaming BFS
// =============================================================================

#[test]
fn test_streaming_parent_pointer_full_path() {
    // GIVEN: a graph 1->2->3->4 with branch 2->5
    let store = create_test_edge_store();
    let config = StreamingConfig::default().with_max_depth(3);

    // WHEN: streaming BFS from node 1
    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    // THEN: all paths reconstructed correctly via parent pointers
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

    let node5 = results
        .iter()
        .find(|r| r.target_id == 5)
        .expect("test: node 5");
    assert_eq!(node5.path, vec![100, 103], "1->2->5 via edges 100,103");

    let node4 = results
        .iter()
        .find(|r| r.target_id == 4)
        .expect("test: node 4");
    assert_eq!(
        node4.path,
        vec![100, 101, 102],
        "1->2->3->4 via edges 100,101,102"
    );
}

#[test]
fn test_streaming_parent_pointer_csr_path() {
    // GIVEN: same graph with CSR snapshot
    let mut store = create_test_edge_store();
    store.build_read_snapshot();
    let config = StreamingConfig::default().with_max_depth(3);

    // WHEN: streaming BFS via CSR path
    let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

    // THEN: parent-pointer paths match expected values
    let node4 = results
        .iter()
        .find(|r| r.target_id == 4)
        .expect("test: node 4");
    assert_eq!(
        node4.path,
        vec![100, 101, 102],
        "CSR path: 1->2->3->4 via edges 100,101,102"
    );
}
