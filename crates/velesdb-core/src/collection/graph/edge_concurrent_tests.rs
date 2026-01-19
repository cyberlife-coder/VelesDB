//! Tests for `ConcurrentEdgeStore` - thread-safety and performance.

use super::edge::GraphEdge;
use super::edge_concurrent::ConcurrentEdgeStore;
use std::sync::Arc;
use std::thread;

// =============================================================================
// Basic functionality tests
// =============================================================================

#[test]
fn test_concurrent_store_add_and_get() {
    let store = ConcurrentEdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "KNOWS"));

    let outgoing = store.get_outgoing(100);
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].target(), 200);
}

#[test]
fn test_concurrent_store_get_neighbors() {
    let store = ConcurrentEdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "A"));
    store.add_edge(GraphEdge::new(2, 100, 300, "B"));

    let neighbors = store.get_neighbors(100);
    assert_eq!(neighbors.len(), 2);
    assert!(neighbors.contains(&200));
    assert!(neighbors.contains(&300));
}

#[test]
fn test_concurrent_store_cascade_delete() {
    let store = ConcurrentEdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "A"));
    store.add_edge(GraphEdge::new(2, 100, 300, "B"));
    store.add_edge(GraphEdge::new(3, 400, 100, "C"));

    store.remove_node_edges(100);

    // Note: cascade delete in sharded store only cleans the source shard
    // Full cross-shard cleanup would require more complex logic
    assert!(store.get_outgoing(100).is_empty());
}

// =============================================================================
// BFS Traversal tests (AC-2: Multi-hop traversal)
// =============================================================================

#[test]
fn test_traverse_bfs_single_hop() {
    let store = ConcurrentEdgeStore::new();
    store.add_edge(GraphEdge::new(1, 1, 2, "LINK"));
    store.add_edge(GraphEdge::new(2, 1, 3, "LINK"));

    let reachable = store.traverse_bfs(1, 1);
    assert!(reachable.contains(&1));
    assert!(reachable.contains(&2));
    assert!(reachable.contains(&3));
}

#[test]
fn test_traverse_bfs_multi_hop() {
    let store = ConcurrentEdgeStore::new();
    // Chain: 1 -> 2 -> 3 -> 4 -> 5
    store.add_edge(GraphEdge::new(1, 1, 2, "NEXT"));
    store.add_edge(GraphEdge::new(2, 2, 3, "NEXT"));
    store.add_edge(GraphEdge::new(3, 3, 4, "NEXT"));
    store.add_edge(GraphEdge::new(4, 4, 5, "NEXT"));

    // Depth 2: should reach 1, 2, 3
    let depth2 = store.traverse_bfs(1, 2);
    assert!(depth2.contains(&1));
    assert!(depth2.contains(&2));
    assert!(depth2.contains(&3));
    assert!(!depth2.contains(&4));

    // Depth 4: should reach all
    let depth4 = store.traverse_bfs(1, 4);
    assert_eq!(depth4.len(), 5);
}

#[test]
fn test_traverse_bfs_with_cycle() {
    let store = ConcurrentEdgeStore::new();
    // Cycle: 1 -> 2 -> 3 -> 1
    store.add_edge(GraphEdge::new(1, 1, 2, "NEXT"));
    store.add_edge(GraphEdge::new(2, 2, 3, "NEXT"));
    store.add_edge(GraphEdge::new(3, 3, 1, "NEXT"));

    // Should not infinite loop
    let reachable = store.traverse_bfs(1, 10);
    assert_eq!(reachable.len(), 3);
}

#[test]
fn test_traverse_bfs_disconnected() {
    let store = ConcurrentEdgeStore::new();
    store.add_edge(GraphEdge::new(1, 1, 2, "LINK"));
    store.add_edge(GraphEdge::new(2, 100, 200, "OTHER")); // Disconnected

    let reachable = store.traverse_bfs(1, 10);
    assert!(reachable.contains(&1));
    assert!(reachable.contains(&2));
    assert!(!reachable.contains(&100));
    assert!(!reachable.contains(&200));
}

// =============================================================================
// Concurrency tests
// =============================================================================

#[test]
fn test_concurrent_reads_no_block() {
    let store = Arc::new(ConcurrentEdgeStore::new());

    // Add some edges
    for i in 0..100 {
        store.add_edge(GraphEdge::new(i, i, i + 1, "LINK"));
    }

    // Spawn many readers
    let mut handles = vec![];
    for _ in 0..10 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let _ = store_clone.get_outgoing(i);
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread panicked");
    }
}

#[test]
fn test_concurrent_write_different_shards() {
    let store = Arc::new(ConcurrentEdgeStore::with_shards(64));

    let mut handles = vec![];
    for t in 0..8 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let id = (t * 1000 + i) as u64;
                let source = t as u64 * 1000 + i as u64;
                let target = source + 1;
                store_clone.add_edge(GraphEdge::new(id, source, target, "LINK"));
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread panicked");
    }

    assert_eq!(store.edge_count(), 800);
}

#[test]
fn test_concurrent_read_write_same_shard() {
    let store = Arc::new(ConcurrentEdgeStore::with_shards(1)); // Single shard

    let store_writer = Arc::clone(&store);
    let store_reader = Arc::clone(&store);

    let writer = thread::spawn(move || {
        for i in 0..100 {
            store_writer.add_edge(GraphEdge::new(i, 1, i + 100, "LINK"));
        }
    });

    let reader = thread::spawn(move || {
        for _ in 0..100 {
            let _ = store_reader.get_outgoing(1);
        }
    });

    writer.join().expect("Writer panicked");
    reader.join().expect("Reader panicked");
}

#[test]
fn test_sharded_lock_ordering_no_deadlock() {
    let store = Arc::new(ConcurrentEdgeStore::with_shards(4));

    // Create edges that cross shards in different orders
    let mut handles = vec![];
    for t in 0..4 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for i in 0..50 {
                let source = (t * 100 + i) as u64;
                let target = ((t + 1) % 4 * 100 + i) as u64;
                store_clone.add_edge(GraphEdge::new(
                    (t * 1000 + i) as u64,
                    source,
                    target,
                    "CROSS",
                ));
            }
        }));
    }

    // If there's a deadlock, this will hang
    for h in handles {
        h.join().expect("Thread panicked - possible deadlock");
    }
}

// =============================================================================
// Cross-shard incoming edges test (Bug fix verification)
// =============================================================================

#[test]
fn test_get_incoming_cross_shard() {
    // Use 64 shards to ensure source and target are in different shards
    let store = ConcurrentEdgeStore::with_shards(64);

    // source=100 → shard 36 (100 % 64)
    // target=200 → shard 8 (200 % 64)
    // These are in DIFFERENT shards
    store.add_edge(GraphEdge::new(1, 100, 200, "WROTE"));

    // get_outgoing should work (looks in source shard)
    let outgoing = store.get_outgoing(100);
    assert_eq!(outgoing.len(), 1, "get_outgoing should find the edge");
    assert_eq!(outgoing[0].target(), 200);

    // get_incoming MUST also work (must look in correct shard)
    let incoming = store.get_incoming(200);
    assert_eq!(
        incoming.len(),
        1,
        "get_incoming must find cross-shard edges"
    );
    assert_eq!(incoming[0].source(), 100);
}

#[test]
fn test_bidirectional_traversal_cross_shard() {
    let store = ConcurrentEdgeStore::with_shards(64);

    // Create edges that definitely cross shards
    // Node IDs chosen to be in different shards
    store.add_edge(GraphEdge::new(1, 0, 64, "A")); // shard 0 -> shard 0
    store.add_edge(GraphEdge::new(2, 1, 65, "B")); // shard 1 -> shard 1
    store.add_edge(GraphEdge::new(3, 2, 100, "C")); // shard 2 -> shard 36

    // All incoming lookups must work
    assert_eq!(store.get_incoming(64).len(), 1);
    assert_eq!(store.get_incoming(65).len(), 1);
    assert_eq!(store.get_incoming(100).len(), 1);
}

// =============================================================================
// Edge count
// =============================================================================

#[test]
#[should_panic(expected = "num_shards must be at least 1")]
fn test_with_shards_zero_panics() {
    let _ = ConcurrentEdgeStore::with_shards(0);
}

// =============================================================================
// Cross-shard remove_node_edges cleanup test (Bug fix verification)
// =============================================================================

#[test]
fn test_remove_node_edges_cross_shard_cleanup() {
    // Use 64 shards to ensure source and target are in different shards
    let store = ConcurrentEdgeStore::with_shards(64);

    // source=100 → shard 36 (100 % 64)
    // target=200 → shard 8 (200 % 64)
    store.add_edge(GraphEdge::new(1, 100, 200, "WROTE"));

    // Verify edge exists in both directions
    assert_eq!(store.get_outgoing(100).len(), 1);
    assert_eq!(store.get_incoming(200).len(), 1);
    assert_eq!(store.edge_count(), 1);

    // Remove edges for node 100 (source node)
    store.remove_node_edges(100);

    // Edge should be completely removed from both shards
    assert_eq!(
        store.get_outgoing(100).len(),
        0,
        "Outgoing edges should be removed"
    );
    assert_eq!(
        store.get_incoming(200).len(),
        0,
        "Incoming edges in other shard should also be cleaned up"
    );
    assert_eq!(
        store.edge_count(),
        0,
        "Edge count should be 0 after cleanup"
    );
}

#[test]
fn test_remove_node_edges_incoming_cross_shard() {
    let store = ConcurrentEdgeStore::with_shards(64);

    // source=200 → shard 8
    // target=100 → shard 36
    store.add_edge(GraphEdge::new(1, 200, 100, "POINTS_TO"));

    // Remove edges for node 100 (target node)
    store.remove_node_edges(100);

    // Edge should be completely removed from both shards
    assert_eq!(store.get_outgoing(200).len(), 0);
    assert_eq!(store.get_incoming(100).len(), 0);
    assert_eq!(store.edge_count(), 0);
}

#[test]
fn test_edge_count_across_shards() {
    let store = ConcurrentEdgeStore::with_shards(4);

    for i in 0..100 {
        store.add_edge(GraphEdge::new(i, i, i + 1, "LINK"));
    }

    assert_eq!(store.edge_count(), 100);
}
