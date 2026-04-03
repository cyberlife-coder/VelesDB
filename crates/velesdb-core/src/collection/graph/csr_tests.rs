//! Tests for `CsrSnapshot` and `SnapshotBuilder` (Task 1).
//!
//! Unit tests validate specific examples and edge cases.
//! Property-based tests validate structural invariants across random inputs.

use super::edge::{EdgeStore, GraphEdge, SnapshotBuilder};
use super::label_table::LabelTable;
use std::collections::HashSet;

// =============================================================================
// Unit tests — Task 1.4
// =============================================================================

/// Empty EdgeStore → offsets == [0], all arrays empty.
#[test]
fn test_csr_empty_graph() {
    let store = EdgeStore::new();
    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    assert_eq!(snapshot.offsets(), &[0]);
    assert_eq!(snapshot.node_count(), 0);
    assert_eq!(snapshot.edge_count(), 0);
}

/// SnapshotBuilder::empty() produces a valid empty snapshot.
#[test]
fn test_csr_snapshot_empty() {
    let snapshot = SnapshotBuilder::empty();

    assert_eq!(snapshot.offsets(), &[0]);
    assert_eq!(snapshot.node_count(), 0);
    assert_eq!(snapshot.edge_count(), 0);
    assert!(snapshot.neighbors(42).is_empty());
    assert!(snapshot.edge_ids(42).is_empty());
    assert!(snapshot.label_ids(42).is_empty());
    assert_eq!(snapshot.degree(42), 0);
    assert!(!snapshot.contains_node(42));
}

/// Node that is a source but has no outgoing edges after filtering
/// (e.g., all edge IDs in outgoing map point to removed edges).
/// In practice, EdgeStore keeps outgoing in sync, so a node with
/// an empty outgoing vec still appears in the CSR with degree 0.
#[test]
fn test_csr_single_node_no_edges() {
    // A node that only appears as a target (no outgoing edges)
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "KNOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    // Node 100 has outgoing edges, node 200 does not
    assert!(snapshot.contains_node(100));
    assert!(!snapshot.contains_node(200)); // 200 is only a target
    assert_eq!(snapshot.degree(100), 1);
    assert_eq!(snapshot.degree(200), 0);
}

/// Known graph: verify O(1) slice access returns correct neighbors.
#[test]
fn test_csr_neighbors_returns_correct_slice() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 1, 3, "LIKES").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(12, 2, 3, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(13, 3, 1, "FOLLOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    // Node 1 → {2, 3}
    let n1: HashSet<u64> = snapshot.neighbors(1).iter().copied().collect();
    assert_eq!(n1, HashSet::from([2, 3]));
    assert_eq!(snapshot.degree(1), 2);

    // Node 2 → {3}
    assert_eq!(snapshot.neighbors(2), &[3]);
    assert_eq!(snapshot.degree(2), 1);

    // Node 3 → {1}
    assert_eq!(snapshot.neighbors(3), &[1]);
    assert_eq!(snapshot.degree(3), 1);

    // Edge IDs parallel to neighbors
    let eids_1: HashSet<u64> = snapshot.edge_ids(1).iter().copied().collect();
    assert_eq!(eids_1, HashSet::from([10, 11]));

    // Label IDs parallel to neighbors
    let lids = snapshot.label_ids(1);
    assert_eq!(lids.len(), 2);

    // Total counts
    assert_eq!(snapshot.node_count(), 3); // sources: 1, 2, 3
    assert_eq!(snapshot.edge_count(), 4);
}

/// Nonexistent node_id → empty slices.
#[test]
fn test_csr_unknown_node_returns_empty() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 100, 200, "A").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    assert!(snapshot.neighbors(999).is_empty());
    assert!(snapshot.edge_ids(999).is_empty());
    assert!(snapshot.label_ids(999).is_empty());
    assert_eq!(snapshot.degree(999), 0);
    assert!(!snapshot.contains_node(999));
}

/// label_at returns correct labels for each neighbor position.
#[test]
fn test_csr_label_at_correct() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 1, 3, "FOLLOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    let targets = snapshot.neighbors(1);
    let eids = snapshot.edge_ids(1);

    // Each label_at should match the original edge's label
    for (i, &eid) in eids.iter().enumerate() {
        let label = snapshot.label_at(1, i).expect("label exists");
        let edge = store.get_edge(eid).expect("edge exists");
        assert_eq!(label, edge.label(), "label mismatch at position {i} for target {}", targets[i]);
    }

    // Out of range
    assert!(snapshot.label_at(1, 10).is_none());
    assert!(snapshot.label_at(999, 0).is_none());
}

/// has_label checks the interned label table.
#[test]
fn test_csr_has_label() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 2, 3, "FOLLOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    assert!(snapshot.has_label("KNOWS"));
    assert!(snapshot.has_label("FOLLOWS"));
    assert!(!snapshot.has_label("LIKES"));
}

/// Deterministic layout: nodes are sorted by ID.
#[test]
fn test_csr_deterministic_node_order() {
    let mut store = EdgeStore::new();
    // Add edges in non-sorted order
    store
        .add_edge(GraphEdge::new(1, 300, 400, "C").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 200, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 200, 300, "B").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    // offsets should reflect sorted node order: 100, 200, 300
    assert_eq!(snapshot.node_count(), 3);
    assert!(snapshot.contains_node(100));
    assert!(snapshot.contains_node(200));
    assert!(snapshot.contains_node(300));

    // Offsets should be monotonically increasing
    let offsets = snapshot.offsets();
    for i in 0..offsets.len() - 1 {
        assert!(offsets[i] <= offsets[i + 1], "offsets not monotone at {i}");
    }
}

// =============================================================================
// Property-based tests — Tasks 1.2 and 1.3
// =============================================================================

mod property_tests {
    use super::*;
    use proptest::prelude::*;

    /// Generates a random `(EdgeStore, LabelTable)` with 1-50 nodes and 0-200 edges.
    fn arb_edge_store() -> impl Strategy<Value = (EdgeStore, LabelTable)> {
        // Generate node count, then edge list
        (1_u64..=50, 0_usize..=200).prop_flat_map(|(max_node, edge_count)| {
            let labels = vec!["KNOWS", "FOLLOWS", "LIKES", "WORKS_AT", "CREATED"];
            prop::collection::vec(
                (1..=max_node, 1..=max_node, 0..labels.len()),
                0..=edge_count,
            )
            .prop_map(move |edges| {
                let mut store = EdgeStore::new();
                let label_table = LabelTable::new();
                let labels = vec!["KNOWS", "FOLLOWS", "LIKES", "WORKS_AT", "CREATED"];
                for (i, (src, tgt, label_idx)) in edges.into_iter().enumerate() {
                    let label = labels[label_idx];
                    // Use index as edge ID to guarantee uniqueness
                    let eid = (i + 1) as u64;
                    if let Ok(edge) = GraphEdge::new(eid, src, tgt, label) {
                        let _ = store.add_edge(edge);
                    }
                }
                (store, label_table)
            })
        })
    }

    // Feature: graph-traversal-v2, Property 1: CSR structural invariants
    // **Validates: Requirements 1.1, 1.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_csr_structural_invariants((store, label_table) in arb_edge_store()) {
            let snapshot = SnapshotBuilder::build(&store, &label_table);

            let total_edges = snapshot.edge_count();
            let node_count = snapshot.node_count();
            let offsets = snapshot.offsets();

            // Parallel arrays: neighbors, edge_ids, label_ids all same length per node.
            let mut sum_degrees = 0usize;
            let source_nodes: HashSet<u64> = store.all_edges().iter().map(|e| e.source()).collect();
            for &nid in &source_nodes {
                if snapshot.contains_node(nid) {
                    let n = snapshot.neighbors(nid).len();
                    let e = snapshot.edge_ids(nid).len();
                    let l = snapshot.label_ids(nid).len();
                    prop_assert_eq!(n, e);
                    prop_assert_eq!(n, l);
                    sum_degrees += n;
                }
            }
            prop_assert_eq!(sum_degrees, total_edges);

            // offsets.len() == M + 1
            prop_assert_eq!(offsets.len(), node_count + 1);

            // offsets is monotonically non-decreasing
            for i in 0..offsets.len() - 1 {
                prop_assert!(offsets[i] <= offsets[i + 1],
                    "offsets not monotone at index {}: {} > {}", i, offsets[i], offsets[i + 1]);
            }

            // offsets[M] == N (last offset == total edges)
            prop_assert_eq!(*offsets.last().unwrap(), total_edges);

            // Per-node degree matches EdgeStore
            for &nid in &source_nodes {
                if snapshot.contains_node(nid) {
                    let csr_degree = snapshot.degree(nid);
                    let store_degree = store.get_outgoing(nid).len();
                    prop_assert_eq!(csr_degree, store_degree);
                }
            }
        }
    }

    // Feature: graph-traversal-v2, Property 2: Round-trip CsrSnapshot ↔ EdgeStore
    // **Validates: Requirements 1.6**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_csr_round_trip((store, label_table) in arb_edge_store()) {
            let snapshot = SnapshotBuilder::build(&store, &label_table);

            // For each source node in the EdgeStore, verify the CSR contains
            // the same set of (target, edge_id) pairs.
            let source_nodes: HashSet<u64> = store.all_edges().iter().map(|e| e.source()).collect();

            for &nid in &source_nodes {
                let csr_targets = snapshot.neighbors(nid);
                let csr_eids = snapshot.edge_ids(nid);

                // Build set from CSR
                let csr_set: HashSet<(u64, u64)> = csr_targets
                    .iter()
                    .zip(csr_eids.iter())
                    .map(|(&t, &e)| (t, e))
                    .collect();

                // Build set from EdgeStore
                let store_set: HashSet<(u64, u64)> = store
                    .get_outgoing(nid)
                    .iter()
                    .map(|e| (e.target(), e.id()))
                    .collect();

                prop_assert_eq!(csr_set, store_set);
            }
        }
    }
}
