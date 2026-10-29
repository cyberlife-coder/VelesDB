//! Tests for `traversal_dfs` — the DFS shared by `EdgeStore` and collections.

use super::traversal_dfs::{dfs_traverse, expand_dfs_neighbors, DfsFrontier};
use super::{EdgeStore, GraphEdge, TraversalConfig};
use rustc_hash::{FxHashMap, FxHashSet};

fn store_with(edges: &[(u64, u64, u64, &str)]) -> EdgeStore {
    let mut store = EdgeStore::new();
    for &(id, source, target, label) in edges {
        store
            .add_edge(GraphEdge::new(id, source, target, label).expect("test: valid edge"))
            .expect("test: add edge");
    }
    store
}

fn config(min_depth: u32, max_depth: u32, limit: usize) -> TraversalConfig {
    TraversalConfig {
        min_depth,
        max_depth,
        limit,
        rel_types: Vec::new(),
        deadline: None,
    }
}

/// 1 -> 2 -> 4, 1 -> 3: depth-first reaches 4 before 3.
fn branching_store() -> EdgeStore {
    store_with(&[(10, 1, 2, "A"), (11, 1, 3, "A"), (12, 2, 4, "A")])
}

#[test]
fn walks_depth_first_with_paths() {
    let results = dfs_traverse(&branching_store(), 1, &config(1, 5, 100));
    let order: Vec<(u64, u32, Vec<u64>)> = results
        .into_iter()
        .map(|r| (r.target_id, r.depth, r.path))
        .collect();
    assert_eq!(
        order,
        vec![(2, 1, vec![10]), (4, 2, vec![10, 12]), (3, 1, vec![11])]
    );
}

#[test]
fn a_node_reached_twice_is_one_result() {
    // Two edges into 2, and a diamond 1 -> {2, 3} -> 4.
    let store = store_with(&[
        (10, 1, 2, "A"),
        (11, 1, 2, "B"),
        (12, 1, 3, "A"),
        (13, 2, 4, "A"),
        (14, 3, 4, "A"),
    ]);
    let mut targets: Vec<u64> = dfs_traverse(&store, 1, &config(1, 5, 100))
        .into_iter()
        .map(|r| r.target_id)
        .collect();
    targets.sort_unstable();
    assert_eq!(targets, vec![2, 3, 4]);
}

#[test]
fn honours_min_depth_max_depth_limit_and_labels() {
    let store = branching_store();
    let deep_only: Vec<u64> = dfs_traverse(&store, 1, &config(2, 5, 100))
        .into_iter()
        .map(|r| r.target_id)
        .collect();
    assert_eq!(deep_only, vec![4]);

    assert!(dfs_traverse(&store, 1, &config(1, 1, 100))
        .iter()
        .all(|r| r.depth == 1));
    assert_eq!(dfs_traverse(&store, 1, &config(1, 5, 2)).len(), 2);

    let labelled = store_with(&[(10, 1, 2, "KNOWS"), (11, 1, 3, "BLOCKED")]);
    let knows = TraversalConfig {
        rel_types: vec!["KNOWS".to_string()],
        ..config(1, 3, 100)
    };
    let reached: Vec<u64> = dfs_traverse(&labelled, 1, &knows)
        .into_iter()
        .map(|r| r.target_id)
        .collect();
    assert_eq!(reached, vec![2]);
}

#[test]
fn terminates_on_a_cycle() {
    let store = store_with(&[(10, 1, 2, "A"), (11, 2, 3, "A"), (12, 3, 1, "A")]);
    let targets: Vec<u64> = dfs_traverse(&store, 1, &config(1, 100, 100))
        .into_iter()
        .map(|r| r.target_id)
        .collect();
    assert_eq!(targets, vec![2, 3]);
}

#[test]
fn expand_dfs_neighbors_bounds_push_growth() {
    // Regression (#906): a single high-out-degree hub must not push more
    // than `max_pending` neighbors into the stack / parent_map at PUSH time.
    // The pop-time `visited.len()` guard does NOT cover this, because DFS
    // records neighbors before they are popped.
    let fanout = 5_000u64;
    let edges: Vec<(u64, u64, u64, &str)> = (1..=fanout).map(|t| (t, 0, t, "E")).collect();
    let store = store_with(&edges);

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
        &store.get_outgoing(0),
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
