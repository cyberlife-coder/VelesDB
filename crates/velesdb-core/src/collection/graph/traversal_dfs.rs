//! Iterative depth-first traversal, the one DFS every graph surface runs.
//!
//! [`dfs_traverse`] walks a standalone [`EdgeStore`]; a collection's
//! `traverse_dfs_config` walks its sharded
//! [`ConcurrentEdgeStore`](super::ConcurrentEdgeStore) through the same
//! [`dfs_traverse_with`] loop, so a binding that holds a bare `EdgeStore`
//! (the Python `GraphStore`) and a collection answer a DFS identically.

use std::borrow::Borrow;

use rustc_hash::{FxHashMap, FxHashSet};

use super::traversal::{deadline_reached, reconstruct_path, DEADLINE_CHECK_INTERVAL};
use super::{EdgeStore, GraphEdge, TraversalConfig, TraversalResult, MAX_VISITED_SIZE};

/// Frontier entry: `(node_id, depth)`. Paths live in the parent-pointer map.
pub(super) type DfsEntry = (u64, u32);

/// Mutable DFS push-side state plus its growth cap.
pub(super) struct DfsFrontier<'a> {
    pub(super) stack: &'a mut Vec<DfsEntry>,
    pub(super) parent_map: &'a mut FxHashMap<u64, (u64, u64)>,
    /// Maximum number of pending (queued-but-unpopped) neighbors. Once
    /// `parent_map` reaches this, no further neighbors are queued.
    pub(super) max_pending: usize,
}

/// Performs an iterative DFS from `source_id` over a standalone [`EdgeStore`].
///
/// One result per reached node, in depth-first order, each carrying the
/// edge-ID path from `source_id`; the source itself is never a result. A
/// node is visited once, so two edges into it yield one result.
/// `config.min_depth`, `max_depth`, `limit`, `rel_types` and `deadline` are
/// honoured as by a collection's `traverse_dfs_config`, which runs the same
/// loop, and the visited set is bounded by [`MAX_VISITED_SIZE`].
#[must_use]
pub fn dfs_traverse(
    edge_store: &EdgeStore,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    dfs_traverse_with(source_id, config, |node| edge_store.get_outgoing(node))
}

/// The DFS loop, over any source of a node's outgoing edges.
///
/// `outgoing` is asked for a node's edges only when that node is expanded,
/// so an owned (`ConcurrentEdgeStore`) or borrowed (`EdgeStore`) edge list
/// both work without an extra copy.
pub(crate) fn dfs_traverse_with<E, F>(
    source_id: u64,
    config: &TraversalConfig,
    mut outgoing: F,
) -> Vec<TraversalResult>
where
    E: Borrow<GraphEdge>,
    F: FnMut(u64) -> Vec<E>,
{
    let rel_filter: FxHashSet<&str> = config.rel_types.iter().map(String::as_str).collect();
    let mut walk = DfsWalk::new(source_id, config);

    while let Some((node_id, depth)) = walk.stack.pop() {
        if walk.must_stop() {
            break;
        }
        if !walk.visited.insert(node_id) {
            continue;
        }
        if walk.record(node_id, depth) {
            break;
        }
        if depth < config.max_depth {
            let edges = outgoing(node_id);
            walk.expand(&edges, node_id, depth, &rel_filter);
        }
    }
    walk.results
}

/// Mutable state of one DFS run.
struct DfsWalk<'a> {
    config: &'a TraversalConfig,
    source_id: u64,
    results: Vec<TraversalResult>,
    visited: FxHashSet<u64>,
    /// Parent-pointer map: `target -> (parent, edge_id)`, so a path is
    /// rebuilt only for a node that becomes a result.
    parent_map: FxHashMap<u64, (u64, u64)>,
    stack: Vec<DfsEntry>,
    /// Starts at the threshold so an already-expired deadline aborts on the
    /// first pop; otherwise the clock is only read every N pops.
    nodes_since_check: u32,
}

impl<'a> DfsWalk<'a> {
    fn new(source_id: u64, config: &'a TraversalConfig) -> Self {
        Self {
            config,
            source_id,
            results: Vec::new(),
            visited: FxHashSet::default(),
            parent_map: FxHashMap::default(),
            stack: vec![(source_id, 0)],
            nodes_since_check: DEADLINE_CHECK_INTERVAL,
        }
    }

    /// Pop-time guards: the result limit, the deadline, and the visited-set
    /// bound (#906). A guard that fires returns the bounded result so far.
    fn must_stop(&mut self) -> bool {
        self.results.len() >= self.config.limit
            || deadline_reached(self.config.deadline, &mut self.nodes_since_check)
            || self.visited.len() >= MAX_VISITED_SIZE
    }

    /// Emits `node_id` when its depth is in range; returns `true` once the
    /// limit is reached.
    fn record(&mut self, node_id: u64, depth: u32) -> bool {
        if depth >= self.config.min_depth && depth > 0 {
            let path = reconstruct_path(node_id, self.source_id, &self.parent_map);
            self.results
                .push(TraversalResult::new(node_id, path, depth));
        }
        self.results.len() >= self.config.limit
    }

    fn expand<E: Borrow<GraphEdge>>(
        &mut self,
        edges: &[E],
        node_id: u64,
        depth: u32,
        rel_filter: &FxHashSet<&str>,
    ) {
        let mut frontier = DfsFrontier {
            stack: &mut self.stack,
            parent_map: &mut self.parent_map,
            max_pending: MAX_VISITED_SIZE,
        };
        expand_dfs_neighbors(
            edges,
            node_id,
            depth,
            rel_filter,
            &self.visited,
            &mut frontier,
        );
    }
}

/// Pushes unvisited, rel-type-filtered neighbors onto the DFS stack, in
/// reverse so the first edge is popped first.
///
/// Issue #906: bounds **push-time** growth of `stack` / `parent_map`. The
/// pop-time `visited.len()` guard does not protect these structures, because
/// DFS inserts every unvisited neighbor at PUSH time while `visited` only
/// grows at POP time. A single high-out-degree hub would otherwise add
/// millions of entries in one expansion before the pop-time guard ever fires.
/// Once `parent_map` reaches `frontier.max_pending`, no new neighbor is
/// queued, capping peak memory regardless of out-degree.
pub(super) fn expand_dfs_neighbors<E: Borrow<GraphEdge>>(
    edges: &[E],
    node_id: u64,
    depth: u32,
    rel_filter: &FxHashSet<&str>,
    visited: &FxHashSet<u64>,
    frontier: &mut DfsFrontier<'_>,
) {
    for edge in edges.iter().rev().map(Borrow::borrow) {
        // `parent_map` is monotonic (one entry per ever-queued node, never
        // removed), so its length is an upper bound on the live `stack` size;
        // gating total queued-node memory on it bounds both.
        if frontier.parent_map.len() >= frontier.max_pending {
            break;
        }
        if !rel_filter.is_empty() && !rel_filter.contains(edge.label()) {
            continue;
        }
        if visited.contains(&edge.target()) {
            continue;
        }
        frontier
            .parent_map
            .insert(edge.target(), (node_id, edge.id()));
        frontier.stack.push((edge.target(), depth + 1));
    }
}
