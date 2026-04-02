//! Streaming BFS iterator for memory-bounded graph traversal (EPIC-019 US-005).
//!
//! This module provides lazy iterators that yield traversal results one at a time,
//! avoiding the need to load all visited nodes into memory at once.

use super::edge_concurrent::ConcurrentEdgeStore;
use super::traversal::{reconstruct_path, BfsState};
use super::{EdgeStore, TraversalResult, DEFAULT_MAX_DEPTH};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Configuration for streaming traversal.
///
/// Unlike `TraversalConfig`, this is optimized for memory-bounded streaming
/// where results are yielded lazily via an iterator.
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Maximum depth for traversal.
    pub max_depth: u32,
    /// Maximum number of results to yield (None = unlimited).
    pub limit: Option<usize>,
    /// Maximum size of visited set before switching to approximate mode.
    /// When exceeded, the iterator stops tracking visited nodes exactly,
    /// which may cause some nodes to be visited multiple times in cyclic graphs.
    pub max_visited_size: usize,
    /// Filter by relationship types (empty = all types).
    pub rel_types: Vec<String>,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            limit: None,
            max_visited_size: 100_000, // ~800KB for FxHashSet<u64>
            rel_types: Vec::new(),
        }
    }
}

impl StreamingConfig {
    /// Creates a config with a result limit.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets the maximum depth.
    #[must_use]
    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Sets the maximum visited set size.
    #[must_use]
    pub fn with_max_visited(mut self, max_visited: usize) -> Self {
        self.max_visited_size = max_visited;
        self
    }

    /// Filters by relationship types.
    #[must_use]
    pub fn with_rel_types(mut self, types: Vec<String>) -> Self {
        self.rel_types = types;
        self
    }
}

/// Streaming BFS iterator that yields results lazily.
///
/// This iterator provides memory-bounded traversal by:
/// 1. Yielding results one at a time instead of collecting all
/// 2. Limiting the visited set size to prevent OOM
/// 3. Early termination when limit is reached
///
/// # Memory Characteristics
///
/// - Queue: O(width × depth) - typically small for sparse graphs
/// - Visited: O(min(nodes_traversed, max_visited_size))
/// - Total: Bounded by `max_visited_size` configuration
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::collection::graph::{EdgeStore, BfsIterator, StreamingConfig};
///
/// let store = EdgeStore::new();
/// // ... add edges ...
///
/// // Stream up to 1000 results with max 10 depth
/// let config = StreamingConfig::default()
///     .with_limit(1000)
///     .with_max_depth(10);
///
/// for result in BfsIterator::new(&store, start_id, config) {
///     println!("Reached node {} at depth {}", result.target_id, result.depth);
/// }
/// ```
pub struct BfsIterator<'a> {
    edge_store: &'a EdgeStore,
    queue: VecDeque<BfsState>,
    visited: FxHashSet<u64>,
    config: StreamingConfig,
    /// Pre-built set for O(1) relationship-type filtering without per-edge allocation.
    /// Empty when no filter is configured (all edge types accepted).
    rel_types_set: FxHashSet<String>,
    yielded: usize,
    visited_overflow: bool,
    /// Buffer for pending results from current node being processed.
    /// This ensures all edges from a node are yielded before moving to next node.
    pending_results: VecDeque<TraversalResult>,
    /// Parent-pointer map: target_node -> (parent_node, edge_id).
    /// Replaces per-state path cloning with O(visited_nodes) memory.
    parent_map: FxHashMap<u64, (u64, u64)>,
    /// Source node ID for path reconstruction.
    source_id: u64,
}

impl<'a> BfsIterator<'a> {
    /// Creates a new BFS iterator starting from the given node.
    #[must_use]
    pub fn new(edge_store: &'a EdgeStore, start_id: u64, config: StreamingConfig) -> Self {
        let mut visited = FxHashSet::default();
        visited.insert(start_id);

        let mut queue = VecDeque::new();
        queue.push_back(BfsState {
            node_id: start_id,
            depth: 0,
        });

        // Pre-build FxHashSet from Vec<String> once, not per-edge.
        let rel_types_set: FxHashSet<String> = config.rel_types.iter().cloned().collect();

        Self {
            edge_store,
            queue,
            visited,
            config,
            rel_types_set,
            yielded: 0,
            visited_overflow: false,
            pending_results: VecDeque::new(),
            parent_map: FxHashMap::default(),
            source_id: start_id,
        }
    }

    /// Returns the number of results yielded so far.
    #[must_use]
    pub fn yielded_count(&self) -> usize {
        self.yielded
    }

    /// Returns true if the visited set has overflowed its limit.
    ///
    /// When overflowed, cycle detection is disabled and some nodes
    /// may be visited multiple times.
    #[must_use]
    pub fn is_visited_overflow(&self) -> bool {
        self.visited_overflow
    }

    /// Returns the current size of the visited set.
    #[must_use]
    pub fn visited_size(&self) -> usize {
        self.visited.len()
    }

    /// Checks whether the given label passes the rel-type filter.
    ///
    /// Empty filter = accept all labels.
    #[inline]
    fn label_passes_filter(&self, label: &str) -> bool {
        self.rel_types_set.is_empty() || self.rel_types_set.contains(label)
    }

    /// Records a visited target, handling overflow when the visited set
    /// exceeds `max_visited_size`.
    ///
    /// Returns `true` if the target should be processed (not already visited).
    #[inline]
    fn try_visit(&mut self, target: u64) -> bool {
        if self.visited_overflow {
            return true;
        }
        if self.visited.contains(&target) {
            return false;
        }
        if self.visited.len() >= self.config.max_visited_size {
            self.visited_overflow = true;
            self.visited.clear();
            return true;
        }
        self.visited.insert(target);
        true
    }

    /// Processes outgoing edges using CSR zero-copy path.
    ///
    /// Uses `CsrSnapshot` for contiguous `&[u64]` access to target IDs,
    /// edge IDs, and interned labels — no `GraphEdge` cloning.
    /// Uses parent-pointer insertion instead of path cloning.
    fn expand_node_csr(&mut self, state: &BfsState) {
        let snapshot = self
            .edge_store
            .csr_snapshot()
            .expect("invariant: CSR snapshot checked before calling expand_node_csr");
        let targets = snapshot.neighbors(state.node_id);
        let edge_ids = snapshot.edge_ids(state.node_id);

        for (i, (&target, &eid)) in targets.iter().zip(edge_ids.iter()).enumerate() {
            let new_depth = state.depth + 1;
            if new_depth > self.config.max_depth {
                continue;
            }
            if let Some(label) = snapshot.label_at(state.node_id, i) {
                if !self.label_passes_filter(label) {
                    continue;
                }
            }
            if !self.try_visit(target) {
                continue;
            }
            self.parent_map.insert(target, (state.node_id, eid));
            if new_depth < self.config.max_depth {
                self.queue.push_back(BfsState {
                    node_id: target,
                    depth: new_depth,
                });
            }
            let path = reconstruct_path(target, self.source_id, &self.parent_map);
            self.pending_results
                .push_back(TraversalResult::new(target, path, new_depth));
        }
    }

    /// Processes outgoing edges using the legacy path (clones `GraphEdge`).
    /// Uses parent-pointer insertion instead of path cloning.
    fn expand_node_legacy(&mut self, state: &BfsState) {
        let edges = self.edge_store.get_outgoing(state.node_id);
        for edge in edges {
            if !self.label_passes_filter(edge.label()) {
                continue;
            }
            let target = edge.target();
            let new_depth = state.depth + 1;
            if new_depth > self.config.max_depth {
                continue;
            }
            if !self.try_visit(target) {
                continue;
            }
            self.parent_map.insert(target, (state.node_id, edge.id()));
            if new_depth < self.config.max_depth {
                self.queue.push_back(BfsState {
                    node_id: target,
                    depth: new_depth,
                });
            }
            let path = reconstruct_path(target, self.source_id, &self.parent_map);
            self.pending_results
                .push_back(TraversalResult::new(target, path, new_depth));
        }
    }
}

impl Iterator for BfsIterator<'_> {
    type Item = TraversalResult;

    fn next(&mut self) -> Option<Self::Item> {
        // Check limit
        if let Some(limit) = self.config.limit {
            if self.yielded >= limit {
                return None;
            }
        }

        // First, yield any pending results from previous node processing
        if let Some(result) = self.pending_results.pop_front() {
            self.yielded += 1;
            return Some(result);
        }

        // Process nodes from queue until we have results to yield
        while let Some(state) = self.queue.pop_front() {
            // Dispatch: CSR zero-copy path when snapshot exists, legacy otherwise.
            if self.edge_store.has_csr_snapshot() {
                self.expand_node_csr(&state);
            } else {
                self.expand_node_legacy(&state);
            }

            // After processing all edges from this node, yield first pending result if any
            if let Some(result) = self.pending_results.pop_front() {
                self.yielded += 1;
                return Some(result);
            }
        }

        None
    }
}

/// Convenience function to create a streaming BFS iterator.
#[must_use]
pub fn bfs_stream(
    edge_store: &EdgeStore,
    start_id: u64,
    config: StreamingConfig,
) -> BfsIterator<'_> {
    BfsIterator::new(edge_store, start_id, config)
}

// ---------------------------------------------------------------------------
// ConcurrentBfsIterator — BFS over ConcurrentEdgeStore (sharded)
// ---------------------------------------------------------------------------

/// Streaming BFS iterator that works with [`ConcurrentEdgeStore`].
///
/// Unlike [`BfsIterator`] (which borrows `&EdgeStore` and returns edge
/// references), this iterator acquires per-shard read locks on each
/// `get_outgoing()` call and works with owned `GraphEdge` values.
/// No shard lock is held across iterations, maximising concurrency.
/// Uses parent-pointer map for zero-clone path reconstruction.
pub struct ConcurrentBfsIterator<'a> {
    edge_store: &'a ConcurrentEdgeStore,
    queue: VecDeque<BfsState>,
    visited: FxHashSet<u64>,
    config: StreamingConfig,
    rel_types_set: FxHashSet<String>,
    yielded: usize,
    visited_overflow: bool,
    pending_results: VecDeque<TraversalResult>,
    /// Parent-pointer map: target_node -> (parent_node, edge_id).
    parent_map: FxHashMap<u64, (u64, u64)>,
    /// Source node ID for path reconstruction.
    source_id: u64,
}

impl<'a> ConcurrentBfsIterator<'a> {
    /// Creates a new concurrent BFS iterator starting from the given node.
    #[must_use]
    pub fn new(
        edge_store: &'a ConcurrentEdgeStore,
        start_id: u64,
        config: StreamingConfig,
    ) -> Self {
        let mut visited = FxHashSet::default();
        visited.insert(start_id);

        let mut queue = VecDeque::new();
        queue.push_back(BfsState {
            node_id: start_id,
            depth: 0,
        });

        let rel_types_set: FxHashSet<String> = config.rel_types.iter().cloned().collect();

        Self {
            edge_store,
            queue,
            visited,
            config,
            rel_types_set,
            yielded: 0,
            visited_overflow: false,
            pending_results: VecDeque::new(),
            parent_map: FxHashMap::default(),
            source_id: start_id,
        }
    }
}

impl Iterator for ConcurrentBfsIterator<'_> {
    type Item = TraversalResult;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(limit) = self.config.limit {
            if self.yielded >= limit {
                return None;
            }
        }

        if let Some(result) = self.pending_results.pop_front() {
            self.yielded += 1;
            return Some(result);
        }

        while let Some(state) = self.queue.pop_front() {
            // Per-node shard lock: acquired and released within this call.
            let edges = self.edge_store.get_outgoing(state.node_id);

            for edge in &edges {
                if !self.rel_types_set.is_empty() && !self.rel_types_set.contains(edge.label()) {
                    continue;
                }

                let target = edge.target();
                let new_depth = state.depth + 1;

                if new_depth > self.config.max_depth {
                    continue;
                }

                if !self.visited_overflow && self.visited.contains(&target) {
                    continue;
                }

                if !self.visited_overflow {
                    if self.visited.len() >= self.config.max_visited_size {
                        self.visited_overflow = true;
                        self.visited.clear();
                    } else {
                        self.visited.insert(target);
                    }
                }

                // Record parent pointer; path reconstructed lazily for results.
                self.parent_map.insert(target, (state.node_id, edge.id()));

                if new_depth < self.config.max_depth {
                    self.queue.push_back(BfsState {
                        node_id: target,
                        depth: new_depth,
                    });
                }

                let path = reconstruct_path(target, self.source_id, &self.parent_map);
                self.pending_results
                    .push_back(TraversalResult::new(target, path, new_depth));
            }

            if let Some(result) = self.pending_results.pop_front() {
                self.yielded += 1;
                return Some(result);
            }
        }

        None
    }
}

/// Convenience function to create a streaming BFS iterator over a
/// [`ConcurrentEdgeStore`].
#[must_use]
pub fn concurrent_bfs_stream(
    edge_store: &ConcurrentEdgeStore,
    start_id: u64,
    config: StreamingConfig,
) -> ConcurrentBfsIterator<'_> {
    ConcurrentBfsIterator::new(edge_store, start_id, config)
}

// Tests moved to streaming_tests.rs per project rules
