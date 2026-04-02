//! Tests for `GraphEdge` and `EdgeStore`.
//!
//! TDD: Tests written BEFORE implementation (RED phase).

use super::*;
use serde_json::json;
use std::collections::HashMap;

// =============================================================================
// AC-1: Edge creation and properties
// =============================================================================

#[test]
fn test_create_edge_basic() {
    let edge = GraphEdge::new(1, 100, 200, "MENTIONS").expect("valid edge");

    assert_eq!(edge.id(), 1);
    assert_eq!(edge.source(), 100);
    assert_eq!(edge.target(), 200);
    assert_eq!(edge.label(), "MENTIONS");
    assert!(edge.properties().is_empty());
}

#[test]
fn test_edge_with_properties() {
    let mut props = HashMap::new();
    props.insert("date".to_string(), json!("2026-01-15"));
    props.insert("role".to_string(), json!("author"));

    let edge = GraphEdge::new(1, 10, 20, "WROTE")
        .expect("valid edge")
        .with_properties(props);

    assert_eq!(edge.properties().len(), 2);
    assert_eq!(edge.property("date"), Some(&json!("2026-01-15")));
    assert_eq!(edge.property("role"), Some(&json!("author")));
}

#[test]
fn test_edge_serialization_roundtrip() {
    let mut props = HashMap::new();
    props.insert("weight".to_string(), json!(0.95));

    let edge = GraphEdge::new(42, 1, 2, "RELATED")
        .expect("valid edge")
        .with_properties(props);

    let json = serde_json::to_string(&edge).expect("serialization failed");
    let deserialized: GraphEdge = serde_json::from_str(&json).expect("deserialization failed");

    assert_eq!(edge.id(), deserialized.id());
    assert_eq!(edge.source(), deserialized.source());
    assert_eq!(edge.target(), deserialized.target());
    assert_eq!(edge.label(), deserialized.label());
}

// =============================================================================
// AC-2: EdgeStore - add and get neighbors
// =============================================================================

#[test]
fn test_edge_store_add_edge() {
    let mut store = EdgeStore::new();
    let edge = GraphEdge::new(1, 100, 200, "KNOWS").expect("valid edge");

    store.add_edge(edge).expect("add edge");

    assert_eq!(store.edge_count(), 1);
}

#[test]
fn test_edge_store_get_outgoing() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "WORKS_AT").expect("valid"))
        .expect("add");

    let outgoing = store.get_outgoing(100);
    assert_eq!(outgoing.len(), 2);

    // Node 200 has no outgoing edges
    let outgoing_200 = store.get_outgoing(200);
    assert!(outgoing_200.is_empty());
}

#[test]
fn test_edge_store_get_incoming() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 300, 200, "KNOWS").expect("valid"))
        .expect("add");

    let incoming = store.get_incoming(200);
    assert_eq!(incoming.len(), 2);

    // Node 100 has no incoming edges
    let incoming_100 = store.get_incoming(100);
    assert!(incoming_100.is_empty());
}

#[test]
fn test_edge_store_bidirectional_traversal() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 10, 20, "RELATED").expect("valid"))
        .expect("add");

    // Can traverse from source
    let from_source = store.get_outgoing(10);
    assert_eq!(from_source.len(), 1);
    assert_eq!(from_source[0].target(), 20);

    // Can traverse from target
    let from_target = store.get_incoming(20);
    assert_eq!(from_target.len(), 1);
    assert_eq!(from_target[0].source(), 10);
}

// =============================================================================
// AC-3: Cascade delete
// =============================================================================

#[test]
fn test_edge_store_remove_edge() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "WORKS_AT").expect("valid"))
        .expect("add");

    assert_eq!(store.edge_count(), 2);

    store.remove_edge(1);

    assert_eq!(store.edge_count(), 1);
    assert!(store.get_outgoing(100).iter().all(|e| e.id() != 1));
}

#[test]
fn test_cascade_delete_removes_all_edges() {
    let mut store = EdgeStore::new();
    // Node 100 has 3 outgoing and 2 incoming edges
    store
        .add_edge(GraphEdge::new(1, 100, 200, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "B").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 100, 400, "C").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(4, 500, 100, "D").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(5, 600, 100, "E").expect("valid"))
        .expect("add");

    assert_eq!(store.edge_count(), 5);

    store.remove_node_edges(100);

    assert_eq!(store.edge_count(), 0);
    assert!(store.get_outgoing(100).is_empty());
    assert!(store.get_incoming(100).is_empty());
}

#[test]
fn test_cascade_delete_preserves_other_edges() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 300, 400, "B").expect("valid")) // Unrelated edge
        .expect("add");

    store.remove_node_edges(100);

    assert_eq!(store.edge_count(), 1);
    assert!(!store.get_outgoing(300).is_empty());
}

// =============================================================================
// Edge filtering by label
// =============================================================================

#[test]
fn test_get_outgoing_by_label() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "WORKS_AT").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 100, 400, "KNOWS").expect("valid"))
        .expect("add");

    let knows_edges = store.get_outgoing_by_label(100, "KNOWS");
    assert_eq!(knows_edges.len(), 2);

    let works_at_edges = store.get_outgoing_by_label(100, "WORKS_AT");
    assert_eq!(works_at_edges.len(), 1);

    let none_edges = store.get_outgoing_by_label(100, "UNKNOWN");
    assert!(none_edges.is_empty());
}

// =============================================================================
// Get edge by ID
// =============================================================================

#[test]
fn test_get_edge_by_id() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(42, 100, 200, "TEST").expect("valid"))
        .expect("add");

    let edge = store.get_edge(42);
    assert!(edge.is_some());
    assert_eq!(edge.unwrap().label(), "TEST");

    let missing = store.get_edge(999);
    assert!(missing.is_none());
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_empty_store() {
    let store = EdgeStore::new();

    assert_eq!(store.edge_count(), 0);
    assert!(store.get_outgoing(100).is_empty());
    assert!(store.get_incoming(100).is_empty());
    assert!(store.get_edge(1).is_none());
}

#[test]
fn test_self_loop_edge() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 100, "SELF_REF").expect("valid"))
        .expect("add");

    // Self-loop appears in both outgoing and incoming
    assert_eq!(store.get_outgoing(100).len(), 1);
    assert_eq!(store.get_incoming(100).len(), 1);
}

// =============================================================================
// Bug fix tests: Duplicate ID and empty label validation
// =============================================================================

#[test]
fn test_edge_empty_label_rejected() {
    let result = GraphEdge::new(1, 100, 200, "");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), "VELES-021");
}

#[test]
fn test_edge_whitespace_only_label_rejected() {
    // Bug #10: whitespace-only labels should be rejected
    let result = GraphEdge::new(1, 100, 200, "   ");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), "VELES-021");

    // Also test tabs and newlines
    assert!(GraphEdge::new(2, 100, 200, "\t\n").is_err());
}

#[test]
fn test_edge_label_trimmed_on_storage() {
    // Bug: label with whitespace should be trimmed when stored
    let edge = GraphEdge::new(1, 100, 200, "  KNOWS  ").expect("valid");
    assert_eq!(edge.label(), "KNOWS", "label should be trimmed");

    // Verify lookup works with trimmed label
    let mut store = EdgeStore::new();
    store.add_edge(edge).expect("add");

    let found = store.get_outgoing_by_label(100, "KNOWS");
    assert_eq!(found.len(), 1, "should find edge with trimmed label");
}

#[test]
fn test_edge_store_duplicate_id_rejected() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "FIRST").expect("valid"))
        .expect("add first");

    // Adding edge with same ID should fail
    let result = store.add_edge(GraphEdge::new(1, 300, 400, "SECOND").expect("valid"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), "VELES-019");

    // Original edge should still be intact
    assert_eq!(store.edge_count(), 1);
    let edge = store.get_edge(1).expect("edge exists");
    assert_eq!(edge.source(), 100);
    assert_eq!(edge.target(), 200);
    assert_eq!(edge.label(), "FIRST");
}

#[test]
fn test_edge_store_no_index_corruption_on_duplicate() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "ORIGINAL").expect("valid"))
        .expect("add");

    // Try to add duplicate (should fail)
    let _ = store.add_edge(GraphEdge::new(1, 100, 300, "DUPLICATE").expect("valid"));

    // Indices should NOT be corrupted
    let outgoing = store.get_outgoing(100);
    assert_eq!(outgoing.len(), 1, "outgoing should have exactly 1 entry");

    let incoming_200 = store.get_incoming(200);
    assert_eq!(incoming_200.len(), 1, "incoming to 200 should have 1 entry");

    let incoming_300 = store.get_incoming(300);
    assert!(incoming_300.is_empty(), "incoming to 300 should be empty");
}

#[test]
fn test_get_incoming_by_label() {
    let mut store = EdgeStore::new();

    store
        .add_edge(GraphEdge::new(1, 100, 500, "FOLLOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 200, 500, "FOLLOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 300, 500, "BLOCKS").expect("valid"))
        .expect("add");

    let follows = store.get_incoming_by_label(500, "FOLLOWS");
    assert_eq!(follows.len(), 2);

    let blocks = store.get_incoming_by_label(500, "BLOCKS");
    assert_eq!(blocks.len(), 1);

    let none = store.get_incoming_by_label(500, "LIKES");
    assert!(none.is_empty());
}

#[test]
fn test_contains_edge() {
    let mut store = EdgeStore::new();

    assert!(!store.contains_edge(1));

    store
        .add_edge(GraphEdge::new(1, 100, 200, "TEST").expect("valid"))
        .expect("add");

    assert!(store.contains_edge(1));
    assert!(!store.contains_edge(2));

    store.remove_edge(1);
    assert!(!store.contains_edge(1));
}

#[test]
fn test_outgoing_edge_count() {
    let mut store = EdgeStore::new();

    assert_eq!(store.outgoing_edge_count(), 0);

    store
        .add_edge(GraphEdge::new(1, 100, 200, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "B").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 200, 300, "C").expect("valid"))
        .expect("add");

    assert_eq!(store.outgoing_edge_count(), 3);
    assert_eq!(store.edge_count(), 3);
}

#[test]
fn test_property_access() {
    use serde_json::json;
    use std::collections::HashMap;

    let mut props = HashMap::new();
    props.insert("weight".to_string(), json!(0.95));
    props.insert("since".to_string(), json!("2024-01-01"));

    let edge = GraphEdge::new(1, 100, 200, "KNOWS")
        .expect("valid")
        .with_properties(props);

    assert_eq!(edge.property("weight"), Some(&json!(0.95)));
    assert_eq!(edge.property("since"), Some(&json!("2024-01-01")));
    assert_eq!(edge.property("missing"), None);
    assert_eq!(edge.properties().len(), 2);
}

// =============================================================================
// EPIC-019 US-002: Pré-allocation HashMap avec with_capacity
// =============================================================================

/// AC-1: EdgeStore::with_capacity creates store with pre-allocated HashMaps
#[test]
fn test_edgestore_with_capacity_basic() {
    let store = EdgeStore::with_capacity(1000, 100);
    // Store is empty but has capacity
    assert_eq!(store.edge_count(), 0);
}

/// AC-2: EdgeStore::with_capacity reduces reallocations during bulk insert
#[test]
fn test_edgestore_with_capacity_no_realloc_1000_edges() {
    let mut store = EdgeStore::with_capacity(1000, 500);

    // Insert 1000 edges - should not cause reallocations
    for i in 0..1000u64 {
        let source = i % 500; // 500 unique sources
        let target = (i % 500) + 500; // 500 unique targets
        store
            .add_edge(GraphEdge::new(i, source, target, "LINK").expect("valid"))
            .expect("add");
    }

    assert_eq!(store.edge_count(), 1000);
}

/// AC-3: Default constructor still works (backward compatibility)
#[test]
fn test_edgestore_default_constructor_still_works() {
    let mut store = EdgeStore::new();

    store
        .add_edge(GraphEdge::new(1, 100, 200, "TEST").expect("valid"))
        .expect("add");

    assert_eq!(store.edge_count(), 1);
    assert_eq!(store.get_outgoing(100).len(), 1);
}

// =============================================================================
// EPIC-019 US-003: Index label → edge_ids pour Requêtes Rapides
// =============================================================================

/// AC-1: Adding edge updates the label index
#[test]
fn test_add_edge_updates_label_index() {
    let mut store = EdgeStore::new();

    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "FOLLOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 200, 300, "KNOWS").expect("valid"))
        .expect("add");

    // get_edges_by_label should return all edges with that label
    let knows_edges = store.get_edges_by_label("KNOWS");
    assert_eq!(knows_edges.len(), 2);

    let follows_edges = store.get_edges_by_label("FOLLOWS");
    assert_eq!(follows_edges.len(), 1);
    assert_eq!(follows_edges[0].id(), 2);
}

/// AC-2: Removing edge updates the label index
#[test]
fn test_remove_edge_updates_label_index() {
    let mut store = EdgeStore::new();

    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "KNOWS").expect("valid"))
        .expect("add");

    assert_eq!(store.get_edges_by_label("KNOWS").len(), 2);

    store.remove_edge(1);

    let knows_edges = store.get_edges_by_label("KNOWS");
    assert_eq!(knows_edges.len(), 1);
    assert_eq!(knows_edges[0].id(), 2);
}

/// AC-3: get_edges_by_label returns correct edges
#[test]
fn test_get_edges_by_label_returns_correct_edges() {
    let mut store = EdgeStore::new();

    for i in 0..100 {
        let label = if i % 3 == 0 {
            "A"
        } else if i % 3 == 1 {
            "B"
        } else {
            "C"
        };
        store
            .add_edge(GraphEdge::new(i, i, i + 1000, label).expect("valid"))
            .expect("add");
    }

    let a_edges = store.get_edges_by_label("A");
    let b_edges = store.get_edges_by_label("B");
    let c_edges = store.get_edges_by_label("C");

    assert_eq!(a_edges.len(), 34); // 0, 3, 6, ..., 99 = 34 values
    assert_eq!(b_edges.len(), 33);
    assert_eq!(c_edges.len(), 33);

    // Verify all A edges have label "A"
    assert!(a_edges.iter().all(|e| e.label() == "A"));
}

/// AC-4: get_outgoing_by_label uses composite index - O(k) not O(degree)
#[test]
fn test_get_outgoing_by_label_uses_index() {
    let mut store = EdgeStore::new();

    // Create node 1 with high degree (1000 edges) but only 10 with label "TARGET"
    for i in 0..1000 {
        let label = if i < 10 { "TARGET" } else { "OTHER" };
        store
            .add_edge(GraphEdge::new(i, 1, i + 1000, label).expect("valid"))
            .expect("add");
    }

    // get_outgoing_by_label should return exactly 10 edges
    let target_edges = store.get_outgoing_by_label(1, "TARGET");
    assert_eq!(target_edges.len(), 10);
    assert!(target_edges.iter().all(|e| e.label() == "TARGET"));
    assert!(target_edges.iter().all(|e| e.source() == 1));
}

/// AC-5: Empty label query returns empty vec
#[test]
fn test_get_edges_by_label_nonexistent_returns_empty() {
    let mut store = EdgeStore::new();

    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");

    let edges = store.get_edges_by_label("NONEXISTENT");
    assert!(edges.is_empty());
}

/// AC-6: remove_node_edges cleans up label indices
#[test]
fn test_remove_node_edges_cleans_label_index() {
    let mut store = EdgeStore::new();

    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "FOLLOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 400, 100, "KNOWS").expect("valid"))
        .expect("add");

    assert_eq!(store.get_edges_by_label("KNOWS").len(), 2);
    assert_eq!(store.get_edges_by_label("FOLLOWS").len(), 1);

    store.remove_node_edges(100);

    // All edges connected to node 100 should be removed from label index
    assert!(store.get_edges_by_label("KNOWS").is_empty());
    assert!(store.get_edges_by_label("FOLLOWS").is_empty());
}

// =============================================================================
// G1: CSR Snapshot — zero-copy BFS via contiguous memory layout
// =============================================================================

#[test]
fn test_csr_snapshot_build_and_neighbors() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 1, 3, "FOLLOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(12, 2, 4, "KNOWS").expect("valid"))
        .expect("add");

    assert!(!store.has_csr_snapshot(), "no snapshot before build");

    store.build_read_snapshot();
    assert!(store.has_csr_snapshot(), "snapshot exists after build");

    let snapshot = store.csr_snapshot().expect("snapshot exists");
    let neighbors = snapshot.neighbors(1);
    assert_eq!(neighbors.len(), 2, "node 1 has 2 outgoing edges");
    assert!(neighbors.contains(&2));
    assert!(neighbors.contains(&3));

    let neighbors_2 = snapshot.neighbors(2);
    assert_eq!(neighbors_2, &[4]);

    // Non-existent node returns empty slice
    assert!(snapshot.neighbors(999).is_empty());
}

#[test]
fn test_csr_snapshot_edge_ids_parallel() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(101, 1, 3, "B").expect("valid"))
        .expect("add");

    store.build_read_snapshot();
    let snapshot = store.csr_snapshot().expect("snapshot");

    let targets = snapshot.neighbors(1);
    let eids = snapshot.edge_ids(1);
    assert_eq!(targets.len(), eids.len(), "parallel arrays same length");

    // Each edge_id corresponds to the correct target
    for (i, &target) in targets.iter().enumerate() {
        let edge = store.get_edge(eids[i]).expect("edge exists");
        assert_eq!(
            edge.target(),
            target,
            "edge_id at position {i} maps to matching target"
        );
    }
}

#[test]
fn test_csr_snapshot_label_at() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 1, 3, "FOLLOWS").expect("valid"))
        .expect("add");

    store.build_read_snapshot();
    let snapshot = store.csr_snapshot().expect("snapshot");
    let targets = snapshot.neighbors(1);

    // Each neighbor should have a label
    for (i, &target) in targets.iter().enumerate() {
        let label = snapshot
            .label_at(1, i)
            .expect("label exists for each neighbor");
        let edge = store.get_edge(snapshot.edge_ids(1)[i]).expect("edge");
        assert_eq!(label, edge.label(), "label matches for target {target}");
    }

    // Out of range returns None
    assert!(snapshot.label_at(1, 10).is_none());
    assert!(snapshot.label_at(999, 0).is_none());
}

#[test]
fn test_csr_snapshot_invalidated_on_add() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "A").expect("valid"))
        .expect("add");

    store.build_read_snapshot();
    assert!(store.has_csr_snapshot());

    // Adding a new edge invalidates the snapshot
    store
        .add_edge(GraphEdge::new(11, 1, 3, "B").expect("valid"))
        .expect("add");
    assert!(
        !store.has_csr_snapshot(),
        "snapshot invalidated after add_edge"
    );
}

#[test]
fn test_csr_snapshot_invalidated_on_remove() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 2, 3, "B").expect("valid"))
        .expect("add");

    store.build_read_snapshot();
    assert!(store.has_csr_snapshot());

    store.remove_edge(10);
    assert!(
        !store.has_csr_snapshot(),
        "snapshot invalidated after remove_edge"
    );
}

#[test]
fn test_csr_snapshot_invalidated_on_remove_node() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "A").expect("valid"))
        .expect("add");

    store.build_read_snapshot();
    assert!(store.has_csr_snapshot());

    store.remove_node_edges(1);
    assert!(
        !store.has_csr_snapshot(),
        "snapshot invalidated after remove_node_edges"
    );
}

#[test]
fn test_with_neighbors_uses_csr_when_available() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 1, 3, "B").expect("valid"))
        .expect("add");

    // Without CSR: falls back to get_outgoing
    let neighbors_no_csr = store.with_neighbors(1, <[u64]>::to_vec);
    assert_eq!(neighbors_no_csr.len(), 2);

    // Build CSR
    store.build_read_snapshot();

    // With CSR: uses contiguous snapshot
    let neighbors_csr = store.with_neighbors(1, <[u64]>::to_vec);
    assert_eq!(neighbors_csr.len(), 2);

    // Both paths produce the same set of neighbors
    let mut s1: Vec<u64> = neighbors_no_csr;
    let mut s2: Vec<u64> = neighbors_csr;
    s1.sort_unstable();
    s2.sort_unstable();
    assert_eq!(s1, s2, "CSR and legacy paths return same neighbors");
}

#[test]
fn test_with_neighbor_edges_parallel_access() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "X").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(101, 1, 3, "Y").expect("valid"))
        .expect("add");

    store.build_read_snapshot();

    store.with_neighbor_edges(1, |targets, eids| {
        assert_eq!(targets.len(), 2);
        assert_eq!(eids.len(), 2);
        for (i, &t) in targets.iter().enumerate() {
            let edge = store.get_edge(eids[i]).expect("edge exists");
            assert_eq!(edge.target(), t);
        }
    });
}

#[test]
fn test_csr_snapshot_empty_store() {
    let mut store = EdgeStore::new();
    store.build_read_snapshot();
    assert!(store.has_csr_snapshot());

    let snapshot = store.csr_snapshot().expect("snapshot");
    assert!(snapshot.neighbors(1).is_empty());
    assert!(snapshot.edge_ids(1).is_empty());
}

#[test]
fn test_csr_snapshot_rebuild_after_mutation() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "A").expect("valid"))
        .expect("add");

    store.build_read_snapshot();
    assert_eq!(store.csr_snapshot().expect("s").neighbors(1).len(), 1);

    // Mutate: add another edge (invalidates snapshot)
    store
        .add_edge(GraphEdge::new(11, 1, 3, "B").expect("valid"))
        .expect("add");
    assert!(!store.has_csr_snapshot());

    // Rebuild: should reflect new edge
    store.build_read_snapshot();
    assert_eq!(store.csr_snapshot().expect("s").neighbors(1).len(), 2);
}

#[test]
fn test_csr_has_label() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 2, 3, "FOLLOWS").expect("valid"))
        .expect("add");

    store.build_read_snapshot();
    let snapshot = store.csr_snapshot().expect("snapshot");

    assert!(snapshot.has_label("KNOWS"));
    assert!(snapshot.has_label("FOLLOWS"));
    assert!(!snapshot.has_label("LIKES"));
}
