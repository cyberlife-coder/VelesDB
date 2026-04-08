//! Graph traversal helper functions and types.
//!
//! Extracted from `graph_api.rs` to reduce NLOC below the 500 threshold.

// SAFETY: Functions used by graph_api.rs via Collection — false positive
// from pub(crate) visibility on Collection.
#![allow(dead_code)]

use rustc_hash::{FxHashMap, FxHashSet};

use crate::collection::graph::{ConcurrentEdgeStore, GraphEdge, TraversalResult};

/// Returns `true` if the edge's label is accepted by the relationship filter.
#[inline]
pub(super) fn edge_passes_rel_filter(edge: &GraphEdge, rel_types: &[&str]) -> bool {
    rel_types.is_empty() || rel_types.contains(&edge.label())
}

/// Reconstructs the edge-ID path from `source` to `target` using parent pointers.
pub(super) fn reconstruct_path(
    parent_map: &FxHashMap<u64, (u64, u64)>,
    source: u64,
    target: u64,
) -> Vec<u64> {
    let mut path = Vec::new();
    let mut current = target;
    while current != source {
        if let Some(&(parent, edge_id)) = parent_map.get(&current) {
            path.push(edge_id);
            current = parent;
        } else {
            break;
        }
    }
    path.reverse();
    path
}

/// Collects unvisited, rel-type-filtered neighbor targets for a node.
///
/// Records parent pointers for lazy path reconstruction (G4).
#[inline]
pub(super) fn collect_neighbor_expansions(
    edges: &[GraphEdge],
    node: u64,
    depth: u32,
    rel_types: &[&str],
    visited: &mut FxHashSet<u64>,
    parent_map: &mut FxHashMap<u64, (u64, u64)>,
) -> Vec<(u64, u32)> {
    edges
        .iter()
        .filter(|e| edge_passes_rel_filter(e, rel_types))
        .filter(|e| visited.insert(e.target()))
        .map(|e| {
            parent_map.insert(e.target(), (node, e.id()));
            (e.target(), depth + 1)
        })
        .collect()
}

/// Pushes unvisited, rel-type-filtered neighbors onto the DFS stack.
///
/// Records parent pointers for lazy path reconstruction (G4).
#[inline]
pub(super) fn expand_dfs_neighbors(
    store: &ConcurrentEdgeStore,
    node_id: u64,
    depth: u32,
    rel_filter: &FxHashSet<&str>,
    visited: &FxHashSet<u64>,
    stack: &mut Vec<TraversalEntry>,
    parent_map: &mut FxHashMap<u64, (u64, u64)>,
) {
    let outgoing = store.get_outgoing(node_id);
    for edge in outgoing.iter().rev() {
        if !rel_filter.is_empty() && !rel_filter.contains(edge.label()) {
            continue;
        }
        if visited.contains(&edge.target()) {
            continue;
        }
        parent_map.insert(edge.target(), (node_id, edge.id()));
        stack.push((edge.target(), depth + 1));
    }
}

/// Frontier entry: `(node_id, depth)`. Paths live in the parent-pointer map.
pub(super) type TraversalEntry = (u64, u32);

/// Bundled parameters for `traverse_with_frontier`.
pub(super) struct TraversalParams<'a> {
    pub(super) store: &'a ConcurrentEdgeStore,
    pub(super) filter: &'a [&'a str],
    pub(super) limit: usize,
    pub(super) max_depth: u32,
    pub(super) source: u64,
}

/// Shared traversal loop for both BFS and DFS.
///
/// Uses parent-pointer map for zero-clone path reconstruction (G4).
pub(super) fn traverse_with_frontier<F>(
    params: &TraversalParams<'_>,
    pop_fn: fn(&mut F) -> Option<TraversalEntry>,
    push_fn: fn(&mut F, TraversalEntry),
    frontier: &mut F,
) -> Vec<TraversalResult> {
    let mut visited = FxHashSet::default();
    let mut parent_map: FxHashMap<u64, (u64, u64)> = FxHashMap::default();
    let mut results = Vec::new();
    visited.insert(params.source);

    while let Some((node, depth)) = (pop_fn)(frontier) {
        if results.len() >= params.limit {
            break;
        }
        if depth >= params.max_depth {
            continue;
        }

        let outgoing = params.store.get_outgoing(node);
        let neighbors = collect_neighbor_expansions(
            &outgoing,
            node,
            depth,
            params.filter,
            &mut visited,
            &mut parent_map,
        );

        for (target, next_depth) in neighbors {
            let path = reconstruct_path(&parent_map, params.source, target);
            results.push(TraversalResult::new(target, path, next_depth));
            if results.len() >= params.limit {
                break;
            }
            (push_fn)(frontier, (target, next_depth));
        }
    }

    results
}

/// BFS pop: removes from the front of the `VecDeque`.
pub(super) fn bfs_pop(
    q: &mut std::collections::VecDeque<TraversalEntry>,
) -> Option<TraversalEntry> {
    q.pop_front()
}

/// BFS push: appends to the back of the `VecDeque`.
pub(super) fn bfs_push(q: &mut std::collections::VecDeque<TraversalEntry>, item: TraversalEntry) {
    q.push_back(item);
}

/// DFS pop: removes from the end of the `Vec`.
pub(super) fn dfs_pop(s: &mut Vec<TraversalEntry>) -> Option<TraversalEntry> {
    s.pop()
}

/// DFS push: appends to the end of the `Vec`.
pub(super) fn dfs_push(s: &mut Vec<TraversalEntry>, item: TraversalEntry) {
    s.push(item);
}
