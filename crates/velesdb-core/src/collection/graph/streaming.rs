//! Streaming BFS iterator for memory-bounded graph traversal (EPIC-019 US-005).
//!
//! This module provides lazy iterators that yield traversal results one at a time,
//! avoiding the need to load all visited nodes into memory at once.

use super::edge_concurrent::ConcurrentEdgeStore;
use super::traversal::{reconstruct_path, BfsState};
use super::{EdgeStore, TraversalResult, DEFAULT_MAX_DEPTH};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Default upper bound on the visited-set / parent-map size for a single
/// traversal (issue #906).
///
/// ~800 KB for an `FxHashSet<u64>` of this size. The streaming iterators
/// switch to approximate mode at this bound; the eager BFS/DFS helpers stop
/// expanding and return the bounded result they have accumulated so far.
pub const MAX_VISITED_SIZE: usize = 100_000;

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
            max_visited_size: MAX_VISITED_SIZE, // ~800KB for FxHashSet<u64>
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

/// Shared BFS bookkeeping for the streaming iterators.
///
/// Owns the traversal frontier, the visited set (with overflow handling), the
/// rel-type filter, the parent-pointer map, and the pending-result buffer. Both
/// [`BfsIterator`] and [`ConcurrentBfsIterator`] delegate per-edge processing
/// and result pumping here, so the filter/visit/record logic lives in exactly
/// one place regardless of the underlying edge store.
struct BfsBookkeeping {
    config: StreamingConfig,
    queue: VecDeque<BfsState>,
    visited: FxHashSet<u64>,
    rel_types_set: FxHashSet<String>,
    visited_overflow: bool,
    pending_results: VecDeque<TraversalResult>,
    parent_map: FxHashMap<u64, (u64, u64)>,
    source_id: u64,
    yielded: usize,
}

impl BfsBookkeeping {
    /// Seeds the frontier and visited set with `start_id`.
    fn new(start_id: u64, config: StreamingConfig) -> Self {
        let rel_types_set: FxHashSet<String> = config.rel_types.iter().cloned().collect();
        let mut visited = FxHashSet::default();
        visited.insert(start_id);
        let mut queue = VecDeque::new();
        queue.push_back(BfsState {
            node_id: start_id,
            depth: 0,
        });
        Self {
            config,
            queue,
            visited,
            rel_types_set,
            visited_overflow: false,
            pending_results: VecDeque::new(),
            parent_map: FxHashMap::default(),
            source_id: start_id,
            yielded: 0,
        }
    }

    /// Checks whether `label` passes the rel-type filter (empty filter = all).
    #[inline]
    fn label_passes_filter(&self, label: &str) -> bool {
        self.rel_types_set.is_empty() || self.rel_types_set.contains(label)
    }

    /// Records a visited target, handling overflow when the visited set exceeds
    /// `max_visited_size`. Returns `true` if the target should be processed.
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

    /// Processes one candidate edge: applies the rel-type (when `label` is
    /// `Some`), depth, and visited filters, then on acceptance records the
    /// parent pointer, enqueues the target, and buffers a pending result.
    fn process_candidate(
        &mut self,
        parent_id: u64,
        target: u64,
        edge_id: u64,
        parent_depth: u32,
        label: Option<&str>,
    ) {
        if let Some(label) = label {
            if !self.label_passes_filter(label) {
                return;
            }
        }
        let new_depth = parent_depth + 1;
        if new_depth > self.config.max_depth {
            return;
        }
        if !self.try_visit(target) {
            return;
        }
        self.parent_map.insert(target, (parent_id, edge_id));
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

    /// Pops the next buffered result, incrementing the yielded counter.
    #[inline]
    fn next_pending(&mut self) -> Option<TraversalResult> {
        let result = self.pending_results.pop_front()?;
        self.yielded += 1;
        Some(result)
    }

    /// Pumps the BFS: yields any buffered result, otherwise expands queued
    /// nodes (via `expand`) until one yields. `expand` supplies the edge-store
    /// access, keeping this driver independent of the store type.
    fn drive(&mut self, mut expand: impl FnMut(&mut Self, &BfsState)) -> Option<TraversalResult> {
        if self.config.limit.is_some_and(|limit| self.yielded >= limit) {
            return None;
        }
        if let Some(result) = self.next_pending() {
            return Some(result);
        }
        while let Some(state) = self.queue.pop_front() {
            expand(self, &state);
            if let Some(result) = self.next_pending() {
                return Some(result);
            }
        }
        None
    }
}

/// Expands a node over the CSR zero-copy path (contiguous `&[u64]` neighbours).
fn expand_csr(edge_store: &EdgeStore, core: &mut BfsBookkeeping, state: &BfsState) {
    let Some(snapshot) = edge_store.csr_snapshot() else {
        return;
    };
    let targets = snapshot.neighbors(state.node_id);
    let edge_ids = snapshot.edge_ids(state.node_id);
    for (i, (&target, &eid)) in targets.iter().zip(edge_ids.iter()).enumerate() {
        let label = snapshot.label_at(state.node_id, i);
        core.process_candidate(state.node_id, target, eid, state.depth, label);
    }
}

/// Expands a node over the legacy `EdgeStore` path (owned `GraphEdge` values).
fn expand_legacy(edge_store: &EdgeStore, core: &mut BfsBookkeeping, state: &BfsState) {
    for edge in edge_store.get_outgoing(state.node_id) {
        core.process_candidate(
            state.node_id,
            edge.target(),
            edge.id(),
            state.depth,
            Some(edge.label()),
        );
    }
}

/// Expands a node over a [`ConcurrentEdgeStore`] (per-shard locked reads).
fn expand_concurrent(
    edge_store: &ConcurrentEdgeStore,
    core: &mut BfsBookkeeping,
    state: &BfsState,
) {
    for edge in &edge_store.get_outgoing(state.node_id) {
        core.process_candidate(
            state.node_id,
            edge.target(),
            edge.id(),
            state.depth,
            Some(edge.label()),
        );
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
    core: BfsBookkeeping,
}

impl<'a> BfsIterator<'a> {
    /// Creates a new BFS iterator starting from the given node.
    #[must_use]
    pub fn new(edge_store: &'a EdgeStore, start_id: u64, config: StreamingConfig) -> Self {
        Self {
            edge_store,
            core: BfsBookkeeping::new(start_id, config),
        }
    }

    /// Returns the number of results yielded so far.
    #[must_use]
    pub fn yielded_count(&self) -> usize {
        self.core.yielded
    }

    /// Returns true if the visited set has overflowed its limit.
    ///
    /// When overflowed, cycle detection is disabled and some nodes
    /// may be visited multiple times.
    #[must_use]
    pub fn is_visited_overflow(&self) -> bool {
        self.core.visited_overflow
    }

    /// Returns the current size of the visited set.
    #[must_use]
    pub fn visited_size(&self) -> usize {
        self.core.visited.len()
    }
}

impl Iterator for BfsIterator<'_> {
    type Item = TraversalResult;

    fn next(&mut self) -> Option<Self::Item> {
        let edge_store = self.edge_store;
        // Dispatch: CSR zero-copy path when a snapshot exists, legacy otherwise.
        self.core.drive(|core, state| {
            if edge_store.has_csr_snapshot() {
                expand_csr(edge_store, core, state);
            } else {
                expand_legacy(edge_store, core, state);
            }
        })
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
    core: BfsBookkeeping,
}

impl<'a> ConcurrentBfsIterator<'a> {
    /// Creates a new concurrent BFS iterator starting from the given node.
    #[must_use]
    pub fn new(
        edge_store: &'a ConcurrentEdgeStore,
        start_id: u64,
        config: StreamingConfig,
    ) -> Self {
        Self {
            edge_store,
            core: BfsBookkeeping::new(start_id, config),
        }
    }
}

impl Iterator for ConcurrentBfsIterator<'_> {
    type Item = TraversalResult;

    fn next(&mut self) -> Option<Self::Item> {
        let edge_store = self.edge_store;
        self.core
            .drive(|core, state| expand_concurrent(edge_store, core, state))
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
