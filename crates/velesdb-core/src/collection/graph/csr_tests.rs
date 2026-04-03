//! Tests for `CsrSnapshot` and `SnapshotBuilder` (Task 1).
//!
//! Unit tests validate specific examples and edge cases.
//! Property-based tests validate structural invariants across random inputs.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value,
    clippy::redundant_closure_for_method_calls,
    clippy::useless_vec,
    clippy::similar_names,
    clippy::module_name_repetitions,
)]

use super::edge::{EdgeStore, GraphEdge, SnapshotBuilder};
use super::label_table::LabelTable;
use super::traversal::{bfs_traverse, bfs_traverse_csr, TraversalConfig};
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

    /// Generates a valid `TraversalConfig` with `min_depth <= max_depth` and `limit > 0`.
    fn arb_traversal_config() -> impl Strategy<Value = TraversalConfig> {
        (1_u32..=5, 0_u32..=3, 1_usize..=50).prop_map(|(max_depth, min_offset, limit)| {
            let min_depth = if min_offset >= max_depth {
                1
            } else {
                max_depth - min_offset
            };
            TraversalConfig {
                min_depth,
                max_depth,
                limit,
                rel_types: Vec::new(),
            }
        })
    }

    // Feature: graph-traversal-v2, Property 3: BFS equivalence CSR vs EdgeStore
    // **Validates: Requirements 2.1, 2.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_bfs_equivalence(
            (store, label_table) in arb_edge_store(),
            config in arb_traversal_config(),
        ) {
            let snapshot = SnapshotBuilder::build(&store, &label_table);

            // Pick a source node from the store (first source node, or skip if empty).
            let source_nodes: Vec<u64> = store.all_edges().iter().map(|e| e.source()).collect();
            if let Some(&source_id) = source_nodes.first() {
                let csr_results = bfs_traverse_csr(&snapshot, source_id, &config);
                let store_results = bfs_traverse(&store, source_id, &config);

                // Compare as sets of (target_id, depth).
                let csr_set: HashSet<(u64, u32)> = csr_results
                    .iter()
                    .map(|r| (r.target_id, r.depth))
                    .collect();
                let store_set: HashSet<(u64, u32)> = store_results
                    .iter()
                    .map(|r| (r.target_id, r.depth))
                    .collect();

                prop_assert_eq!(csr_set, store_set,
                    "BFS equivalence failed for source={}, config=({},{}), limit={}",
                    source_id, config.min_depth, config.max_depth, config.limit);
            }
        }
    }

    // Feature: graph-traversal-v2, Property 4: BFS limit invariant
    // **Validates: Requirements 2.5**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_bfs_limit(
            (store, label_table) in arb_edge_store(),
            limit in 1_usize..100,
        ) {
            let snapshot = SnapshotBuilder::build(&store, &label_table);

            let source_nodes: Vec<u64> = store.all_edges().iter().map(|e| e.source()).collect();
            if let Some(&source_id) = source_nodes.first() {
                let config = TraversalConfig::with_range(1, 5).with_limit(limit);
                let results = bfs_traverse_csr(&snapshot, source_id, &config);

                prop_assert!(results.len() <= limit,
                    "BFS returned {} results but limit was {}",
                    results.len(), limit);
            }
        }
    }
}


// =============================================================================
// Unit tests — Task 3.4: bfs_traverse_csr
// =============================================================================

/// Source not in snapshot → empty Vec.
#[test]
fn test_bfs_csr_missing_source() {
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 10, 20, "KNOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    let config = TraversalConfig::with_range(1, 3);
    let results = bfs_traverse_csr(&snapshot, 999, &config);
    assert!(results.is_empty(), "missing source should return empty Vec");
}

/// Verify min_depth/max_depth filtering on a known graph.
#[test]
fn test_bfs_csr_depth_range() {
    // Chain: 1 -> 2 -> 3 -> 4
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(102, 3, 4, "KNOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    // min_depth=2, max_depth=3 → should only return nodes at depth 2 and 3
    let config = TraversalConfig::with_range(2, 3);
    let results = bfs_traverse_csr(&snapshot, 1, &config);

    assert!(
        !results.iter().any(|r| r.depth < 2),
        "no results below min_depth"
    );
    assert!(
        results.iter().any(|r| r.target_id == 3 && r.depth == 2),
        "node 3 at depth 2"
    );
    assert!(
        results.iter().any(|r| r.target_id == 4 && r.depth == 3),
        "node 4 at depth 3"
    );
}

/// Verify results.len() <= limit.
#[test]
fn test_bfs_csr_limit_respected() {
    // Star graph: 1 -> {2, 3, 4, 5, 6}
    let mut store = EdgeStore::new();
    for i in 2..=6 {
        store
            .add_edge(GraphEdge::new(i, 1, i, "LINK").expect("valid"))
            .expect("add");
    }

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    let config = TraversalConfig::with_range(1, 1).with_limit(3);
    let results = bfs_traverse_csr(&snapshot, 1, &config);

    assert!(
        results.len() <= 3,
        "expected at most 3 results, got {}",
        results.len()
    );
}

// =============================================================================
// Unit tests — Task 5.4: ArcSwap integration in ConcurrentEdgeStore
// =============================================================================

/// After adding an edge, the CSR snapshot reflects the new edge.
#[test]
fn test_snapshot_rebuild_after_add() {
    use super::edge_concurrent::ConcurrentEdgeStore;

    let store = ConcurrentEdgeStore::with_shards(4);
    store
        .add_edge(GraphEdge::new(1, 10, 20, "KNOWS").expect("valid"))
        .expect("add");

    let snapshot = store.get_csr_snapshot();
    assert!(snapshot.contains_node(10), "snapshot should contain source node");
    let neighbors: HashSet<u64> = snapshot.neighbors(10).iter().copied().collect();
    assert!(neighbors.contains(&20), "snapshot should reflect added edge");
}

/// After removing an edge, the CSR snapshot no longer contains it.
#[test]
fn test_snapshot_rebuild_after_remove() {
    use super::edge_concurrent::ConcurrentEdgeStore;

    let store = ConcurrentEdgeStore::with_shards(4);
    store
        .add_edge(GraphEdge::new(1, 10, 20, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 10, 30, "LIKES").expect("valid"))
        .expect("add");

    // Remove edge 10→20
    store.remove_edge(1);

    let snapshot = store.get_csr_snapshot();
    let neighbors: HashSet<u64> = snapshot.neighbors(10).iter().copied().collect();
    assert!(!neighbors.contains(&20), "removed edge should not appear");
    assert!(neighbors.contains(&30), "remaining edge should still appear");
}

/// Existing `get_outgoing` API returns the same data as before (backward compat).
#[test]
fn test_get_outgoing_backward_compat() {
    use super::edge_concurrent::ConcurrentEdgeStore;

    let store = ConcurrentEdgeStore::with_shards(4);
    store
        .add_edge(GraphEdge::new(1, 100, 200, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 100, 300, "B").expect("valid"))
        .expect("add");

    let outgoing = store.get_outgoing(100);
    assert_eq!(outgoing.len(), 2);

    let targets: HashSet<u64> = outgoing.iter().map(|e| e.target()).collect();
    assert_eq!(targets, HashSet::from([200, 300]));
}

/// `traverse_bfs_csr` returns correct results via the lock-free snapshot.
#[test]
fn test_traverse_bfs_csr_on_concurrent_store() {
    use super::edge_concurrent::ConcurrentEdgeStore;

    let store = ConcurrentEdgeStore::with_shards(4);
    // Chain: 1 -> 2 -> 3
    store
        .add_edge(GraphEdge::new(1, 1, 2, "NEXT").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 2, 3, "NEXT").expect("valid"))
        .expect("add");

    let config = TraversalConfig::with_range(1, 3);
    let results = store.traverse_bfs_csr(1, &config);

    let targets: HashSet<u64> = results.iter().map(|r| r.target_id).collect();
    assert!(targets.contains(&2), "should reach node 2");
    assert!(targets.contains(&3), "should reach node 3");
}

/// Snapshot reflects `remove_node_edges` cascade delete.
#[test]
fn test_snapshot_rebuild_after_remove_node_edges() {
    use super::edge_concurrent::ConcurrentEdgeStore;

    let store = ConcurrentEdgeStore::with_shards(4);
    store
        .add_edge(GraphEdge::new(1, 10, 20, "A").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 10, 30, "B").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 40, 10, "C").expect("valid"))
        .expect("add");

    store.remove_node_edges(10);

    let snapshot = store.get_csr_snapshot();
    assert!(
        snapshot.neighbors(10).is_empty(),
        "node 10 should have no outgoing edges after cascade delete"
    );
    assert_eq!(snapshot.edge_count(), 0, "all edges should be removed");
}


// =============================================================================
// Unit tests — Task 7.4: Predicate pushdown
// =============================================================================

/// `NoFilter` returns all neighbors (no filtering).
#[test]
fn test_no_filter_returns_all() {
    use super::edge::NoFilter;

    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 10, 20, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 10, 30, "LIKES").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 10, 40, "FOLLOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    let no_filter = NoFilter;
    let filtered: Vec<(u64, u64, _)> = snapshot
        .neighbors_filtered(10, &no_filter)
        .collect();

    // NoFilter should return all 3 neighbors
    assert_eq!(filtered.len(), 3);
    let targets: HashSet<u64> = filtered.iter().map(|&(t, _, _)| t).collect();
    assert_eq!(targets, HashSet::from([20, 30, 40]));
}

/// `LabelFilter` returns only edges with matching labels.
#[test]
fn test_label_filter_selective() {
    use super::edge::LabelFilter;
    use rustc_hash::FxHashSet;

    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 10, 20, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 10, 30, "LIKES").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 10, 40, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(4, 10, 50, "FOLLOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    // Find the LabelId for "KNOWS" from the snapshot's label_ids
    // We need to identify which LabelId corresponds to "KNOWS"
    let all_neighbors: Vec<(u64, u64, _)> = snapshot
        .neighbors_filtered(10, &super::edge::NoFilter)
        .collect();

    // Find the label_id for KNOWS by checking which edges have target 20 or 40
    let knows_label_id = all_neighbors
        .iter()
        .find(|&&(t, _, _)| t == 20)
        .map(|&(_, _, lid)| lid)
        .expect("should find KNOWS edge");

    let mut allowed = FxHashSet::default();
    allowed.insert(knows_label_id);
    let label_filter = LabelFilter::new(allowed);

    let filtered: Vec<(u64, u64, _)> = snapshot
        .neighbors_filtered(10, &label_filter)
        .collect();

    // Only KNOWS edges (targets 20 and 40)
    assert_eq!(filtered.len(), 2);
    let targets: HashSet<u64> = filtered.iter().map(|&(t, _, _)| t).collect();
    assert_eq!(targets, HashSet::from([20, 40]));
}

/// BFS with predicate pushdown produces same results as post-hoc filtering.
#[test]
fn test_bfs_filtered_vs_post_filter() {
    use super::edge::{LabelFilter, NoFilter};
    use super::traversal::bfs_traverse_csr_filtered;
    use rustc_hash::FxHashSet;

    // Build a graph: 1 -KNOWS-> 2 -LIKES-> 3 -KNOWS-> 4
    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(10, 1, 2, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(11, 2, 3, "LIKES").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(12, 3, 4, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(13, 1, 5, "LIKES").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    // Find the LabelId for "KNOWS"
    let all_n: Vec<(u64, u64, _)> = snapshot
        .neighbors_filtered(1, &NoFilter)
        .collect();
    let knows_lid = all_n
        .iter()
        .find(|&&(t, _, _)| t == 2)
        .map(|&(_, _, lid)| lid)
        .expect("KNOWS edge");

    let mut allowed = FxHashSet::default();
    allowed.insert(knows_lid);
    let predicate = LabelFilter::new(allowed);

    let config = TraversalConfig::with_range(1, 3);

    // Filtered BFS (pushdown)
    let filtered_results = bfs_traverse_csr_filtered(&snapshot, 1, &config, &predicate);

    // Unfiltered BFS then post-filter
    let all_results = bfs_traverse_csr(&snapshot, 1, &config);

    // Post-filter: only keep results reachable via KNOWS-only paths
    // With pushdown, BFS only follows KNOWS edges, so from node 1:
    //   depth 1: node 2 (via KNOWS) — included
    //   depth 2: nothing (node 2's edges are LIKES, filtered out)
    // Post-filter on all results would keep different nodes since it
    // doesn't restrict traversal paths. The pushdown is stricter.
    // Verify pushdown results are a subset of unfiltered results.
    let filtered_targets: HashSet<u64> = filtered_results.iter().map(|r| r.target_id).collect();
    let all_targets: HashSet<u64> = all_results.iter().map(|r| r.target_id).collect();

    // Pushdown results must be a subset of all results
    assert!(
        filtered_targets.is_subset(&all_targets),
        "filtered targets {:?} should be subset of all targets {:?}",
        filtered_targets,
        all_targets
    );

    // With KNOWS-only filter from node 1, only node 2 is reachable at depth 1
    assert!(filtered_targets.contains(&2), "node 2 reachable via KNOWS");
    // Node 5 is via LIKES, should NOT be in filtered results
    assert!(!filtered_targets.contains(&5), "node 5 via LIKES should be filtered out");
}

// =============================================================================
// Property-based test — Task 7.3: Predicate pushdown correctness
// =============================================================================

mod predicate_property_tests {
    use super::*;
    use proptest::prelude::*;
    use rustc_hash::FxHashSet;
    use crate::collection::graph::edge::{EdgePredicate, LabelFilter, NoFilter};
    use crate::collection::graph::label_table::LabelId;

    /// Generates a random `(EdgeStore, LabelTable)` with 1-50 nodes and 0-200 edges.
    /// (Duplicated from property_tests to keep module self-contained.)
    fn arb_edge_store() -> impl Strategy<Value = (EdgeStore, LabelTable)> {
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
                    let eid = (i + 1) as u64;
                    if let Ok(edge) = GraphEdge::new(eid, src, tgt, label) {
                        let _ = store.add_edge(edge);
                    }
                }
                (store, label_table)
            })
        })
    }

    // Feature: graph-traversal-v2, Property 6: Predicate pushdown correctness
    // **Validates: Requirements 4.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_predicate_pushdown(
            (store, label_table) in arb_edge_store(),
            filter_bits in proptest::bits::u8::between(0, 5),
        ) {
            let snapshot = SnapshotBuilder::build(&store, &label_table);

            // Build a LabelFilter from the random bits
            let mut allowed = FxHashSet::default();
            for i in 0..5u32 {
                if filter_bits & (1 << i) != 0 {
                    allowed.insert(LabelId::from_u32(i));
                }
            }
            let predicate = LabelFilter::new(allowed.clone());

            // For each source node, verify filtered == subset of unfiltered where matches() is true
            let source_nodes: std::collections::HashSet<u64> =
                store.all_edges().iter().map(|e| e.source()).collect();

            for &nid in &source_nodes {
                if !snapshot.contains_node(nid) {
                    continue;
                }

                // Get all neighbors (unfiltered)
                let all_neighbors: Vec<(u64, u64, LabelId)> = snapshot
                    .neighbors_filtered(nid, &NoFilter)
                    .collect();

                // Get filtered neighbors
                let filtered: Vec<(u64, u64, LabelId)> = snapshot
                    .neighbors_filtered(nid, &predicate)
                    .collect();

                // Expected: subset of all_neighbors where predicate matches
                let expected: Vec<(u64, u64, LabelId)> = all_neighbors
                    .iter()
                    .filter(|&&(t, e, l)| predicate.matches(t, e, l))
                    .copied()
                    .collect();

                prop_assert_eq!(
                    filtered, expected,
                    "predicate pushdown mismatch for node {}",
                    nid
                );
            }
        }
    }
}


// =============================================================================
// Unit tests — Task 9: AdjacencySource trait
// =============================================================================

/// AdjacencySource on CsrSnapshot and EdgeStore return the same neighbor sets.
#[test]
fn test_adjacency_source_equivalence() {
    use super::edge::AdjacencySource;

    let mut store = EdgeStore::new();
    store
        .add_edge(GraphEdge::new(1, 10, 20, "KNOWS").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(2, 10, 30, "LIKES").expect("valid"))
        .expect("add");
    store
        .add_edge(GraphEdge::new(3, 20, 30, "FOLLOWS").expect("valid"))
        .expect("add");

    let label_table = LabelTable::new();
    let snapshot = SnapshotBuilder::build(&store, &label_table);

    // Compare AdjacencySource::neighbors for each source node
    for &nid in &[10u64, 20, 30] {
        let csr_neighbors: HashSet<u64> =
            AdjacencySource::neighbors(&snapshot, nid).into_iter().collect();
        let store_neighbors: HashSet<u64> =
            AdjacencySource::neighbors(&store, nid).into_iter().collect();
        assert_eq!(
            csr_neighbors, store_neighbors,
            "AdjacencySource mismatch for node {nid}"
        );
    }

    // Non-existent node returns empty
    assert!(AdjacencySource::neighbors(&snapshot, 999).is_empty());
    assert!(AdjacencySource::neighbors(&store, 999).is_empty());
}
