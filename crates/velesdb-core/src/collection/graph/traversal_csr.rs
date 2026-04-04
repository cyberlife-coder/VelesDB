//! CSR-based BFS traversal functions.
//!
//! Extracted from `traversal.rs` to reduce NLOC.

use super::csr_snapshot::{CsrSnapshot, EdgePredicate};
use super::traversal::{reconstruct_path, BfsState, TraversalConfig, TraversalResult};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

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

    while let Some(state) = queue.pop_front() {
        if results.len() >= config.limit {
            break;
        }

        let targets = snapshot.neighbors(state.node_id);
        let edge_ids = snapshot.edge_ids(state.node_id);

        for (i, (&target, &eid)) in targets.iter().zip(edge_ids.iter()).enumerate() {
            if results.len() >= config.limit {
                break;
            }

            if let Some(label) = snapshot.label_at(state.node_id, i) {
                if !rel_filter.is_empty() && !rel_filter.contains(label) {
                    continue;
                }
            }

            let new_depth = state.depth + 1;
            if new_depth > config.max_depth {
                continue;
            }

            let is_new = visited.insert(target);
            if is_new {
                parent_map.insert(target, (state.node_id, eid));

                if new_depth >= config.min_depth {
                    let path = reconstruct_path(target, source_id, &parent_map);
                    results.push(TraversalResult::new(target, path, new_depth));
                }

                if new_depth < config.max_depth {
                    queue.push_back(BfsState {
                        node_id: target,
                        depth: new_depth,
                    });
                }
            }
        }
    }

    results
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

    while let Some(state) = queue.pop_front() {
        if results.len() >= config.limit {
            break;
        }

        for (target, eid, _label_id) in snapshot.neighbors_filtered(state.node_id, predicate) {
            if results.len() >= config.limit {
                break;
            }

            let new_depth = state.depth + 1;
            if new_depth > config.max_depth {
                continue;
            }

            let is_new = visited.insert(target);
            if is_new {
                parent_map.insert(target, (state.node_id, eid));

                if new_depth >= config.min_depth {
                    let path = reconstruct_path(target, source_id, &parent_map);
                    results.push(TraversalResult::new(target, path, new_depth));
                }

                if new_depth < config.max_depth {
                    queue.push_back(BfsState {
                        node_id: target,
                        depth: new_depth,
                    });
                }
            }
        }
    }

    results
}
