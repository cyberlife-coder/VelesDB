//! Parallel Graph Traversal for MATCH queries (EPIC-051).
//!
//! This module provides parallel BFS/DFS traversal using rayon for
//! efficient execution on multi-core systems.

// SAFETY: Numeric casts in parallel traversal are intentional:
// - u64->usize for node ID hashing: Node IDs are generated sequentially and fit in usize
// - Used for sharding only, actual storage uses u64 for persistence
#![allow(clippy::cast_possible_truncation)]

mod frontier;
mod sharded;
mod traverser;

pub use frontier::FrontierParallelBFS;
pub use sharded::ShardedTraverser;
pub use traverser::ParallelTraverser;

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

/// Result of a parallel traversal operation.
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// Starting node ID.
    pub start_node: u64,
    /// Final node ID reached.
    pub end_node: u64,
    /// Path from start to end (edge IDs).
    pub path: Vec<u64>,
    /// Depth at which `end_node` was found.
    pub depth: u32,
    /// Optional score for ranking.
    pub score: Option<f32>,
}

impl TraversalResult {
    /// Creates a new traversal result.
    #[must_use]
    pub fn new(start_node: u64, end_node: u64, path: Vec<u64>, depth: u32) -> Self {
        Self {
            start_node,
            end_node,
            path,
            depth,
            score: None,
        }
    }

    /// Builder: set score.
    #[must_use]
    pub fn with_score(mut self, score: f32) -> Self {
        self.score = Some(score);
        self
    }

    /// Generates a unique signature for deduplication.
    #[must_use]
    pub fn path_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        self.start_node.hash(&mut hasher);
        self.end_node.hash(&mut hasher);
        self.path.hash(&mut hasher);
        hasher.finish()
    }
}

/// Thread configuration for parallel traversal (EPIC-051 US-006).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ThreadConfig {
    /// Automatically detect optimal thread count based on CPU.
    #[default]
    Auto,
    /// Use a fixed number of threads.
    Fixed(usize),
}

impl ThreadConfig {
    /// Returns the effective number of threads to use.
    #[must_use]
    pub fn effective_threads(&self) -> usize {
        match self {
            ThreadConfig::Auto => {
                // Use std::thread::available_parallelism (same as rayon default)
                let cpus = std::thread::available_parallelism()
                    .map(std::num::NonZeroUsize::get)
                    .unwrap_or(1);
                // Leave 1 core for other work, minimum 1 thread
                (cpus.saturating_sub(1)).max(1)
            }
            ThreadConfig::Fixed(n) => *n,
        }
    }
}

/// Configuration for parallel traversal (EPIC-051 US-006).
#[derive(Debug, Clone)]
pub struct ParallelConfig {
    /// Maximum traversal depth.
    pub max_depth: u32,
    /// Minimum nodes to trigger parallel start-node traversal.
    pub parallel_threshold: usize,
    /// Minimum frontier size to trigger parallel expansion.
    pub min_frontier_for_parallel: usize,
    /// Maximum results to return.
    pub limit: usize,
    /// Relationship types to follow (empty = all).
    pub relationship_types: Vec<String>,
    /// Thread configuration (auto or fixed).
    pub threads: ThreadConfig,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            parallel_threshold: 100,
            min_frontier_for_parallel: 50,
            limit: 1000,
            relationship_types: Vec::new(),
            threads: ThreadConfig::Auto,
        }
    }
}

impl ParallelConfig {
    /// Creates a new config with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: set max depth.
    #[must_use]
    pub fn with_max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }

    /// Builder: set parallel threshold.
    #[must_use]
    pub fn with_parallel_threshold(mut self, threshold: usize) -> Self {
        self.parallel_threshold = threshold;
        self
    }

    /// Builder: set minimum frontier for parallel.
    #[must_use]
    pub fn with_min_frontier(mut self, min_frontier: usize) -> Self {
        self.min_frontier_for_parallel = min_frontier;
        self
    }

    /// Builder: set result limit.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Builder: set thread config.
    #[must_use]
    pub fn with_threads(mut self, threads: ThreadConfig) -> Self {
        self.threads = threads;
        self
    }

    /// Builder: set fixed thread count.
    #[must_use]
    pub fn with_fixed_threads(mut self, count: usize) -> Self {
        self.threads = ThreadConfig::Fixed(count);
        self
    }

    /// Determines if parallelism should be used based on node count.
    #[must_use]
    pub fn should_parallelize(&self, node_count: usize) -> bool {
        node_count >= self.parallel_threshold
    }

    /// Determines if frontier should be expanded in parallel.
    #[must_use]
    pub fn should_parallelize_frontier(&self, frontier_size: usize) -> bool {
        frontier_size >= self.min_frontier_for_parallel
    }

    /// Gets effective thread count for this config.
    #[must_use]
    pub fn effective_threads(&self) -> usize {
        self.threads.effective_threads()
    }
}

/// Statistics from a parallel traversal.
#[derive(Debug, Default)]
pub struct TraversalStats {
    /// Number of start nodes processed.
    pub start_nodes_count: usize,
    /// Total nodes visited across all traversals.
    pub nodes_visited: AtomicUsize,
    /// Total edges traversed.
    pub edges_traversed: AtomicUsize,
    /// Number of results before deduplication.
    pub raw_results: usize,
    /// Number of results after deduplication.
    pub deduplicated_results: usize,
}

impl TraversalStats {
    /// Creates new empty stats.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Increments nodes visited (thread-safe).
    pub fn add_nodes_visited(&self, count: usize) {
        self.nodes_visited.fetch_add(count, AtomicOrdering::Relaxed);
    }

    /// Increments edges traversed (thread-safe).
    pub fn add_edges_traversed(&self, count: usize) {
        self.edges_traversed
            .fetch_add(count, AtomicOrdering::Relaxed);
    }

    /// Gets total nodes visited.
    #[must_use]
    pub fn total_nodes_visited(&self) -> usize {
        self.nodes_visited.load(AtomicOrdering::Relaxed)
    }

    /// Gets total edges traversed.
    #[must_use]
    pub fn total_edges_traversed(&self) -> usize {
        self.edges_traversed.load(AtomicOrdering::Relaxed)
    }
}

// Tests moved to parallel_traversal_tests.rs per project rules
