//! Graph traversal algorithms for multi-hop queries.
//!
//! This module provides BFS-based traversal for variable-length path patterns
//! like `(a)-[*1..3]->(b)` in MATCH clauses.
//!
//! # Streaming Mode (EPIC-019 US-005)
//!
//! For large graphs, the module provides streaming iterators that yield results
//! lazily without loading all visited nodes into memory at once.

#![allow(dead_code)] // WIP: Will be used by MATCH clause execution

use super::EdgeStore;
use std::collections::{HashSet, VecDeque};

/// Default maximum depth for unbounded traversals.
pub const DEFAULT_MAX_DEPTH: u32 = 3;

/// Safety cap for maximum depth to prevent runaway traversals.
/// Only applied when user requests unbounded traversal (*).
///
/// Note: Neo4j and ArangoDB do NOT impose hard limits.
/// 100 is chosen to cover most real-world use cases:
/// - Social networks (6 degrees of separation)
/// - Dependency graphs (deep npm/cargo trees)
/// - Organizational hierarchies
/// - Knowledge graphs
pub const SAFETY_MAX_DEPTH: u32 = 100;

/// Result of a graph traversal operation.
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// The target node ID reached.
    pub target_id: u64,
    /// The path taken (list of edge IDs).
    pub path: Vec<u64>,
    /// Depth of the traversal (number of hops).
    pub depth: u32,
}

impl TraversalResult {
    /// Creates a new traversal result.
    #[must_use]
    pub fn new(target_id: u64, path: Vec<u64>, depth: u32) -> Self {
        Self {
            target_id,
            path,
            depth,
        }
    }
}

/// Configuration for graph traversal.
#[derive(Debug, Clone)]
pub struct TraversalConfig {
    /// Minimum number of hops (inclusive).
    pub min_depth: u32,
    /// Maximum number of hops (inclusive).
    pub max_depth: u32,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Filter by relationship types (empty = all types).
    pub rel_types: Vec<String>,
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            min_depth: 1,
            max_depth: DEFAULT_MAX_DEPTH,
            limit: 100,
            rel_types: Vec::new(),
        }
    }
}

impl TraversalConfig {
    /// Creates a config for a specific range (e.g., *1..3).
    ///
    /// Respects the caller's max_depth without artificial capping.
    /// For unbounded traversals, use `with_unbounded_range()` instead.
    #[must_use]
    pub fn with_range(min: u32, max: u32) -> Self {
        Self {
            min_depth: min,
            max_depth: max,
            ..Self::default()
        }
    }

    /// Creates a config for unbounded traversal (e.g., *1..).
    ///
    /// Applies SAFETY_MAX_DEPTH cap to prevent runaway traversals.
    #[must_use]
    pub fn with_unbounded_range(min: u32) -> Self {
        Self {
            min_depth: min,
            max_depth: SAFETY_MAX_DEPTH,
            ..Self::default()
        }
    }

    /// Sets the result limit.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Filters by relationship types.
    #[must_use]
    pub fn with_rel_types(mut self, types: Vec<String>) -> Self {
        self.rel_types = types;
        self
    }

    /// Sets a custom max depth (for advanced use cases).
    #[must_use]
    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }
}

/// BFS state for traversal.
#[derive(Debug)]
struct BfsState {
    /// Current node ID.
    node_id: u64,
    /// Path taken to reach this node (edge IDs).
    path: Vec<u64>,
    /// Current depth.
    depth: u32,
}

/// Performs BFS traversal from a source node.
///
/// Finds all paths from `source_id` within the configured depth range.
/// Uses iterative BFS with `VecDeque` for better cache locality.
///
/// # Arguments
///
/// * `edge_store` - The edge storage to traverse.
/// * `source_id` - Starting node ID.
/// * `config` - Traversal configuration.
///
/// # Returns
///
/// Vector of traversal results, limited by `config.limit`.
#[must_use]
pub fn bfs_traverse(
    edge_store: &EdgeStore,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // CRITICAL FIX: Mark source node as visited before traversal
    // to prevent cycles back to source causing duplicate work
    visited.insert(source_id);

    // Initialize with source node
    queue.push_back(BfsState {
        node_id: source_id,
        path: Vec::new(),
        depth: 0,
    });

    while let Some(state) = queue.pop_front() {
        // Check if we've collected enough results
        if results.len() >= config.limit {
            break;
        }

        // Get outgoing edges from current node
        let edges = edge_store.get_outgoing(state.node_id);

        for edge in edges {
            // Filter by relationship type if specified
            if !config.rel_types.is_empty() && !config.rel_types.contains(&edge.label().to_string())
            {
                continue;
            }

            let target = edge.target();
            let new_depth = state.depth + 1;

            // Skip if exceeds max depth
            if new_depth > config.max_depth {
                continue;
            }

            // Build new path
            let mut new_path = state.path.clone();
            new_path.push(edge.id());

            // Add to results if within valid depth range
            if new_depth >= config.min_depth {
                results.push(TraversalResult::new(target, new_path.clone(), new_depth));

                if results.len() >= config.limit {
                    break;
                }
            }

            // Continue traversal if not at max depth and not visited
            if new_depth < config.max_depth && !visited.contains(&target) {
                visited.insert(target);
                queue.push_back(BfsState {
                    node_id: target,
                    path: new_path,
                    depth: new_depth,
                });
            }
        }
    }

    results
}

/// Performs BFS traversal in the reverse direction (following incoming edges).
#[must_use]
pub fn bfs_traverse_reverse(
    edge_store: &EdgeStore,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // CRITICAL FIX: Mark source node as visited before traversal
    visited.insert(source_id);

    queue.push_back(BfsState {
        node_id: source_id,
        path: Vec::new(),
        depth: 0,
    });

    while let Some(state) = queue.pop_front() {
        if results.len() >= config.limit {
            break;
        }

        let edges = edge_store.get_incoming(state.node_id);

        for edge in edges {
            if !config.rel_types.is_empty() {
                let label = edge.label().to_string();
                if !config.rel_types.contains(&label) {
                    continue;
                }
            }

            let source = edge.source();
            let new_depth = state.depth + 1;

            if new_depth > config.max_depth {
                continue;
            }

            let mut new_path = state.path.clone();
            new_path.push(edge.id());

            if new_depth >= config.min_depth {
                results.push(TraversalResult::new(source, new_path.clone(), new_depth));

                if results.len() >= config.limit {
                    break;
                }
            }

            if new_depth < config.max_depth && !visited.contains(&source) {
                visited.insert(source);
                queue.push_back(BfsState {
                    node_id: source,
                    path: new_path,
                    depth: new_depth,
                });
            }
        }
    }

    results
}

/// Performs bidirectional BFS (follows both directions).
#[must_use]
pub fn bfs_traverse_both(
    edge_store: &EdgeStore,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    let mut results = Vec::new();
    let half_limit = config.limit / 2 + 1;

    let config_half = TraversalConfig {
        limit: half_limit,
        ..config.clone()
    };

    // Forward traversal
    let forward = bfs_traverse(edge_store, source_id, &config_half);
    results.extend(forward);

    // Reverse traversal
    if results.len() < config.limit {
        let reverse = bfs_traverse_reverse(edge_store, source_id, &config_half);
        for r in reverse {
            if results.len() >= config.limit {
                break;
            }
            // Avoid duplicates
            if !results
                .iter()
                .any(|existing| existing.target_id == r.target_id && existing.path == r.path)
            {
                results.push(r);
            }
        }
    }

    results.truncate(config.limit);
    results
}

// =============================================================================
// EPIC-019 US-005: Streaming BFS Iterator
// =============================================================================

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

        while let Some(state) = self.queue.pop_front() {
            // Get outgoing edges
            let edges = self.edge_store.get_outgoing(state.node_id);

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
                if !self.visited_overflow && self.visited.contains(&target) {
                    continue;
                }

                // Track visited if not overflowed
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

                // Yield result
                self.yielded += 1;
                return Some(TraversalResult::new(target, new_path, new_depth));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::graph::GraphEdge;

    fn create_test_edge_store() -> EdgeStore {
        let mut store = EdgeStore::new();
        // Create a simple graph:
        // 1 --KNOWS--> 2 --KNOWS--> 3 --KNOWS--> 4
        //              |
        //              +--WROTE--> 5
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(102, 3, 4, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(103, 2, 5, "WROTE").unwrap())
            .unwrap();
        store
    }

    fn create_cyclic_edge_store() -> EdgeStore {
        let mut store = EdgeStore::new();
        // Create a cyclic graph:
        // 1 --KNOWS--> 2 --KNOWS--> 3
        // ^                         |
        // +-------KNOWS-------------+
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(102, 3, 1, "KNOWS").unwrap())
            .unwrap(); // Cycle back to 1
        store
    }

    #[test]
    fn test_bfs_single_hop() {
        let store = create_test_edge_store();
        let config = TraversalConfig::with_range(1, 1);

        let results = bfs_traverse(&store, 1, &config);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, 2);
        assert_eq!(results[0].depth, 1);
    }

    #[test]
    fn test_bfs_multi_hop() {
        let store = create_test_edge_store();
        let config = TraversalConfig::with_range(1, 3);

        let results = bfs_traverse(&store, 1, &config);

        // Should find: 1->2, 1->2->3, 1->2->5, 1->2->3->4
        assert!(results.len() >= 4);

        // Verify we can reach node 4 at depth 3
        assert!(results.iter().any(|r| r.target_id == 4 && r.depth == 3));
    }

    #[test]
    fn test_bfs_with_rel_type_filter() {
        let store = create_test_edge_store();
        let config = TraversalConfig::with_range(1, 3).with_rel_types(vec!["KNOWS".to_string()]);

        let results = bfs_traverse(&store, 1, &config);

        // Should only follow KNOWS edges: 1->2, 1->2->3, 1->2->3->4
        // Should NOT include 1->2->5 (WROTE)
        assert!(!results.iter().any(|r| r.target_id == 5));
        assert!(results.iter().any(|r| r.target_id == 4));
    }

    #[test]
    fn test_bfs_min_depth() {
        let store = create_test_edge_store();
        let config = TraversalConfig::with_range(2, 3);

        let results = bfs_traverse(&store, 1, &config);

        // Should NOT include depth 1 results
        assert!(!results.iter().any(|r| r.depth == 1));
        // Should include depth 2 and 3
        assert!(results.iter().any(|r| r.depth == 2));
        assert!(results.iter().any(|r| r.depth == 3));
    }

    #[test]
    fn test_bfs_limit() {
        let store = create_test_edge_store();
        let config = TraversalConfig::with_range(1, 3).with_limit(2);

        let results = bfs_traverse(&store, 1, &config);

        assert!(results.len() <= 2);
    }

    #[test]
    fn test_bfs_reverse() {
        let store = create_test_edge_store();
        let config = TraversalConfig::with_range(1, 2);

        let results = bfs_traverse_reverse(&store, 4, &config);

        // From 4, going backwards: 4<-3, 4<-3<-2
        assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 1));
        assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 2));
    }

    #[test]
    fn test_default_max_depth() {
        assert_eq!(DEFAULT_MAX_DEPTH, 3);

        let config = TraversalConfig::default();
        assert_eq!(config.min_depth, 1);
        assert_eq!(config.max_depth, 3);
    }

    #[test]
    fn test_path_tracking() {
        let store = create_test_edge_store();
        let config = TraversalConfig::with_range(1, 2);

        let results = bfs_traverse(&store, 1, &config);

        // Find the result that reached node 3 (2 hops)
        let to_node_3 = results.iter().find(|r| r.target_id == 3 && r.depth == 2);
        assert!(to_node_3.is_some());

        let path = &to_node_3.unwrap().path;
        assert_eq!(path.len(), 2);
        assert_eq!(path[0], 100); // Edge 1->2
        assert_eq!(path[1], 101); // Edge 2->3
    }

    #[test]
    fn test_with_range_respects_max_depth() {
        // FIX: with_range should NOT cap max_depth artificially
        let config = TraversalConfig::with_range(1, 5);
        assert_eq!(config.max_depth, 5);

        let config = TraversalConfig::with_range(1, 10);
        assert_eq!(config.max_depth, 10);
    }

    #[test]
    fn test_unbounded_range_applies_safety_cap() {
        let config = TraversalConfig::with_unbounded_range(1);
        assert_eq!(config.max_depth, SAFETY_MAX_DEPTH);
        // SAFETY_MAX_DEPTH should be 100 (industry standard, no arbitrary low limit)
        assert_eq!(SAFETY_MAX_DEPTH, 100);
    }

    #[test]
    fn test_bfs_cyclic_graph_no_infinite_loop() {
        let store = create_cyclic_edge_store();
        let config = TraversalConfig::with_range(1, 5).with_limit(100);

        let results = bfs_traverse(&store, 1, &config);

        // Results should be finite (not infinite loop)
        assert!(results.len() < 100);

        // Count how many times each target appears
        let mut target_counts = std::collections::HashMap::new();
        for r in &results {
            *target_counts.entry(r.target_id).or_insert(0) += 1;
        }

        // Each node should appear at most once in results
        // (BFS with visited tracking prevents re-expansion)
        // Node 1 CAN appear as a result (via 3->1 edge) but only once
        for (node_id, count) in &target_counts {
            assert_eq!(
                *count, 1,
                "Node {} appeared {} times, expected 1",
                node_id, count
            );
        }

        // Verify we found the expected nodes: 2, 3, and 1 (via cycle)
        assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
        assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 2));
        // Node 1 is reachable via 1->2->3->1 at depth 3
        assert!(results.iter().any(|r| r.target_id == 1 && r.depth == 3));
    }

    #[test]
    fn test_with_max_depth_custom() {
        let config = TraversalConfig::default().with_max_depth(7);
        assert_eq!(config.max_depth, 7);
    }

    // =========================================================================
    // EPIC-019 US-005: Streaming BFS Iterator Tests
    // =========================================================================

    #[test]
    fn test_bfs_iterator_basic() {
        let store = create_test_edge_store();
        let config = StreamingConfig::default().with_max_depth(3);

        let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

        // Should find same nodes as regular BFS
        assert!(results.iter().any(|r| r.target_id == 2 && r.depth == 1));
        assert!(results.iter().any(|r| r.target_id == 3 && r.depth == 2));
        assert!(results.iter().any(|r| r.target_id == 4 && r.depth == 3));
    }

    #[test]
    fn test_bfs_iterator_with_limit() {
        let store = create_test_edge_store();
        let config = StreamingConfig::default().with_max_depth(5).with_limit(2);

        let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_bfs_iterator_early_exit() {
        let store = create_test_edge_store();
        let config = StreamingConfig::default().with_max_depth(5).with_limit(1);

        let mut iter = BfsIterator::new(&store, 1, config);

        // Take only first result
        let first = iter.next();
        assert!(first.is_some());
        assert_eq!(iter.yielded_count(), 1);

        // Should return None after limit reached
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_bfs_iterator_rel_type_filter() {
        let store = create_test_edge_store();
        let config = StreamingConfig::default()
            .with_max_depth(5)
            .with_rel_types(vec!["KNOWS".to_string()]);

        let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

        // Should only follow KNOWS edges, not WROTE
        assert!(!results.iter().any(|r| r.target_id == 5));
        assert!(results.iter().any(|r| r.target_id == 4));
    }

    #[test]
    fn test_bfs_iterator_visited_overflow() {
        let store = create_test_edge_store();
        // Set very small max_visited to trigger overflow
        let config = StreamingConfig::default()
            .with_max_depth(5)
            .with_max_visited(2);

        let mut iter = BfsIterator::new(&store, 1, config);

        // Consume results until overflow
        let mut count = 0;
        while iter.next().is_some() {
            count += 1;
            if count > 10 {
                break; // Safety limit
            }
        }

        // Should have triggered overflow
        assert!(iter.is_visited_overflow() || count <= 2);
    }

    #[test]
    fn test_bfs_iterator_cyclic_graph() {
        let store = create_cyclic_edge_store();
        let config = StreamingConfig::default().with_max_depth(5).with_limit(10);

        let results: Vec<_> = BfsIterator::new(&store, 1, config).collect();

        // Should not infinite loop
        assert!(results.len() <= 10);

        // Should visit nodes in cycle
        assert!(results.iter().any(|r| r.target_id == 2));
        assert!(results.iter().any(|r| r.target_id == 3));
    }

    #[test]
    fn test_bfs_stream_convenience_function() {
        let store = create_test_edge_store();
        let config = StreamingConfig::default().with_max_depth(2);

        let results: Vec<_> = bfs_stream(&store, 1, config).collect();

        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.depth <= 2));
    }

    #[test]
    fn test_streaming_config_defaults() {
        let config = StreamingConfig::default();

        assert_eq!(config.max_depth, DEFAULT_MAX_DEPTH);
        assert!(config.limit.is_none());
        assert_eq!(config.max_visited_size, 100_000);
        assert!(config.rel_types.is_empty());
    }
}
