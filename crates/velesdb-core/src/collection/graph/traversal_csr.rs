//! CSR-based BFS traversal functions.
//!
//! Extracted from `traversal.rs` to reduce NLOC.

use super::csr_snapshot::{CsrSnapshot, EdgePredicate};
use super::traversal::{reconstruct_path, BfsState, TraversalConfig, TraversalResult};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Bundles the mutable BFS state shared across expansion helpers.
struct BfsContext<'a> {
    config: &'a TraversalConfig,
    source_id: u64,
    results: &'a mut Vec<TraversalResult>,
    visited: &'a mut FxHashSet<u64>,
    queue: &'a mut VecDeque<BfsState>,
    parent_map: &'a mut FxHashMap<u64, (u64, u64)>,
}

impl BfsContext<'_> {
    /// Processes a single BFS candidate: checks depth/visited,
    /// records parent pointer, emits result, and enqueues for further expansion.
    #[inline]
    fn visit_candidate(&mut self, target: u64, edge_id: u64, parent_node: u64, current_depth: u32) {
        let new_depth = current_depth + 1;
        if new_depth > self.config.max_depth {
            return;
        }

        if self.visited.insert(target) {
            self.parent_map.insert(target, (parent_node, edge_id));

            if new_depth >= self.config.min_depth {
                let path = reconstruct_path(target, self.source_id, self.parent_map);
                self.results
                    .push(TraversalResult::new(target, path, new_depth));
            }

            if new_depth < self.config.max_depth {
                self.queue.push_back(BfsState {
                    node_id: target,
                    depth: new_depth,
                });
            }
        }
    }
}

/// BFS traversal on a `CsrSnapshot` for zero-copy graph exploration.
///
/// Uses `FxHashSet` for the visited set and accesses neighbors via
/// `snapshot.neighbors()` (O(1) slice lookup). Uses parent-pointer
/// reconstruction for path building.
///
/// Returns `Vec::new()` if `source_id` is not in the snapshot.
#[must_use]
pub fn bfs_traverse_csr(
    snapshot: &CsrSnapshot,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    if !snapshot.contains_node(source_id) {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut visited = FxHashSet::default();
    let mut queue = VecDeque::new();
    let mut parent_map: FxHashMap<u64, (u64, u64)> = FxHashMap::default();

    let rel_filter: FxHashSet<&str> = config.rel_types.iter().map(String::as_str).collect();

    visited.insert(source_id);
    queue.push_back(BfsState {
        node_id: source_id,
        depth: 0,
    });

    let mut ctx = BfsContext {
        config,
        source_id,
        results: &mut results,
        visited: &mut visited,
        queue: &mut queue,
        parent_map: &mut parent_map,
    };

    while let Some(state) = ctx.queue.pop_front() {
        if ctx.results.len() >= ctx.config.limit {
            break;
        }

        expand_csr_node(snapshot, &state, &rel_filter, &mut ctx);
    }

    results
}

/// Expands a single BFS node using CSR snapshot neighbors.
///
/// Filters by relationship type, records parent pointers, and enqueues
/// unvisited targets for the next BFS level.
fn expand_csr_node(
    snapshot: &CsrSnapshot,
    state: &BfsState,
    rel_filter: &FxHashSet<&str>,
    ctx: &mut BfsContext<'_>,
) {
    let targets = snapshot.neighbors(state.node_id);
    let edge_ids = snapshot.edge_ids(state.node_id);

    for (i, (&target, &eid)) in targets.iter().zip(edge_ids.iter()).enumerate() {
        if ctx.results.len() >= ctx.config.limit {
            break;
        }

        if !label_passes_csr_filter(snapshot, state.node_id, i, rel_filter) {
            continue;
        }

        ctx.visit_candidate(target, eid, state.node_id, state.depth);
    }
}

/// Checks whether the edge at index `i` from `node_id` passes the rel-type filter.
///
/// Returns `true` if the filter is empty or the label matches.
#[inline]
fn label_passes_csr_filter(
    snapshot: &CsrSnapshot,
    node_id: u64,
    edge_index: usize,
    rel_filter: &FxHashSet<&str>,
) -> bool {
    if rel_filter.is_empty() {
        return true;
    }
    snapshot
        .label_at(node_id, edge_index)
        .is_some_and(|label| rel_filter.contains(label))
}

/// BFS traversal on a `CsrSnapshot` with predicate pushdown filtering.
#[must_use]
pub fn bfs_traverse_csr_filtered<P: EdgePredicate>(
    snapshot: &CsrSnapshot,
    source_id: u64,
    config: &TraversalConfig,
    predicate: &P,
) -> Vec<TraversalResult> {
    if !snapshot.contains_node(source_id) {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut visited = FxHashSet::default();
    let mut queue = VecDeque::new();
    let mut parent_map: FxHashMap<u64, (u64, u64)> = FxHashMap::default();

    visited.insert(source_id);
    queue.push_back(BfsState {
        node_id: source_id,
        depth: 0,
    });

    let mut ctx = BfsContext {
        config,
        source_id,
        results: &mut results,
        visited: &mut visited,
        queue: &mut queue,
        parent_map: &mut parent_map,
    };

    while let Some(state) = ctx.queue.pop_front() {
        if ctx.results.len() >= ctx.config.limit {
            break;
        }

        expand_csr_filtered_node(snapshot, &state, predicate, &mut ctx);
    }

    results
}

/// Expands a single BFS node using CSR snapshot with predicate filtering.
///
/// Uses `neighbors_filtered` for predicate pushdown, records parent pointers,
/// and enqueues unvisited targets.
fn expand_csr_filtered_node<P: EdgePredicate>(
    snapshot: &CsrSnapshot,
    state: &BfsState,
    predicate: &P,
    ctx: &mut BfsContext<'_>,
) {
    for (target, eid, _label_id) in snapshot.neighbors_filtered(state.node_id, predicate) {
        if ctx.results.len() >= ctx.config.limit {
            break;
        }

        let new_depth = state.depth + 1;
        if new_depth > ctx.config.max_depth {
            continue;
        }

        ctx.visit_candidate(target, eid, state.node_id, state.depth);
    }
}
