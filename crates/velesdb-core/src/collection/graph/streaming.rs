//! Streaming BFS iterator for memory-bounded graph traversal (EPIC-019 US-005).
//!
//! This module provides lazy iterators that yield traversal results one at a time,
//! avoiding the need to load all visited nodes into memory at once.

use super::traversal::BfsState;
use super::{EdgeStore, TraversalResult, DEFAULT_MAX_DEPTH};
use std::collections::{HashSet, VecDeque};

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
            max_visited_size: 100_000, // ~800KB for HashSet<u64>
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
    visited: HashSet<u64>,
    config: StreamingConfig,
    yielded: usize,
    visited_overflow: bool,
    /// Buffer for pending results from current node being processed.
    /// This ensures all edges from a node are yielded before moving to next node.
    pending_results: VecDeque<TraversalResult>,
}

impl<'a> BfsIterator<'a> {
    /// Creates a new BFS iterator starting from the given node.
    #[must_use]
    pub fn new(edge_store: &'a EdgeStore, start_id: u64, config: StreamingConfig) -> Self {
        let mut visited = HashSet::new();
        visited.insert(start_id);

        let mut queue = VecDeque::new();
        queue.push_back(BfsState {
            node_id: start_id,
            path: Vec::new(),
            depth: 0,
        });

        Self {
            edge_store,
            queue,
            visited,
            config,
            yielded: 0,
            visited_overflow: false,
            pending_results: VecDeque::new(),
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
            // Get outgoing edges
            let edges = self.edge_store.get_outgoing(state.node_id);

            // Process ALL edges from this node, collecting results
            for edge in edges {
                // Filter by relationship type
                if !self.config.rel_types.is_empty()
                    && !self.config.rel_types.contains(&edge.label().to_string())
                {
                    continue;
                }

                let target = edge.target();
                let new_depth = state.depth + 1;

                // Skip if exceeds max depth
                if new_depth > self.config.max_depth {
                    continue;
                }

                // Check visited (with overflow handling)
                // When visited_overflow is true, cycle detection is disabled but traversal
                // is still bounded by max_depth, preventing infinite loops in cyclic graphs.
                if !self.visited_overflow && self.visited.contains(&target) {
                    continue;
                }

                // Track visited if not overflowed
                // Note: When overflow occurs, we trade cycle detection for memory efficiency.
                // The max_depth limit ensures termination even without visited tracking.
                if !self.visited_overflow {
                    if self.visited.len() >= self.config.max_visited_size {
                        self.visited_overflow = true;
                        self.visited.clear(); // Free memory
                    } else {
                        self.visited.insert(target);
                    }
                }

                // Build path
                let mut new_path = state.path.clone();
                new_path.push(edge.id());

                // Queue for further traversal
                if new_depth < self.config.max_depth {
                    self.queue.push_back(BfsState {
                        node_id: target,
                        path: new_path.clone(),
                        depth: new_depth,
                    });
                }

                // Buffer the result (don't return immediately - process all edges first)
                self.pending_results
                    .push_back(TraversalResult::new(target, new_path, new_depth));
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

// Tests moved to streaming_tests.rs per project rules
