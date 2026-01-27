//! Parallel Graph Traversal for MATCH queries (EPIC-051).
//!
//! This module provides parallel BFS/DFS traversal using rayon for
//! efficient execution on multi-core systems.

use rayon::prelude::*;
use rustc_hash::FxHashSet;
use std::cmp::Ordering;
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
    /// Depth at which end_node was found.
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

/// Configuration for parallel traversal.
#[derive(Debug, Clone)]
pub struct ParallelConfig {
    /// Maximum traversal depth.
    pub max_depth: u32,
    /// Minimum nodes to trigger parallelism.
    pub parallel_threshold: usize,
    /// Maximum results to return.
    pub limit: usize,
    /// Relationship types to follow (empty = all).
    pub relationship_types: Vec<String>,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            parallel_threshold: 100,
            limit: 1000,
            relationship_types: Vec::new(),
        }
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

/// Parallel traverser for graph queries (EPIC-051 US-001).
#[derive(Debug, Default)]
pub struct ParallelTraverser {
    config: ParallelConfig,
}

impl ParallelTraverser {
    /// Creates a new parallel traverser with default config.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a parallel traverser with custom config.
    #[must_use]
    pub fn with_config(config: ParallelConfig) -> Self {
        Self { config }
    }

    /// Performs parallel BFS from multiple start nodes (EPIC-051 US-001).
    ///
    /// This method parallelizes traversal across start nodes using rayon,
    /// then merges and deduplicates results.
    ///
    /// # Arguments
    ///
    /// * `start_nodes` - Vector of node IDs to start traversal from
    /// * `get_neighbors` - Function to get neighbors of a node: (node_id) -> Vec<(neighbor_id, edge_id)>
    ///
    /// # Returns
    ///
    /// Merged and deduplicated traversal results.
    pub fn bfs_parallel<F>(
        &self,
        start_nodes: &[u64],
        get_neighbors: F,
    ) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Sync,
    {
        let mut stats = TraversalStats::new();
        stats.start_nodes_count = start_nodes.len();

        // Decide whether to parallelize based on threshold
        let results: Vec<Vec<TraversalResult>> =
            if start_nodes.len() >= self.config.parallel_threshold {
                // Parallel execution
                start_nodes
                    .par_iter()
                    .map(|&start| self.bfs_single(start, &get_neighbors, &stats))
                    .collect()
            } else {
                // Sequential execution for small inputs
                start_nodes
                    .iter()
                    .map(|&start| self.bfs_single(start, &get_neighbors, &stats))
                    .collect()
            };

        // Flatten results
        let flat_results: Vec<TraversalResult> = results.into_iter().flatten().collect();
        stats.raw_results = flat_results.len();

        // Deduplicate and merge (EPIC-051 US-004)
        let merged = self.merge_and_deduplicate(flat_results);
        stats.deduplicated_results = merged.len();

        (merged, stats)
    }

    /// Performs BFS from a single start node.
    fn bfs_single<F>(
        &self,
        start: u64,
        get_neighbors: &F,
        stats: &TraversalStats,
    ) -> Vec<TraversalResult>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Sync,
    {
        use std::collections::VecDeque;

        let mut results = Vec::new();
        let mut visited: FxHashSet<u64> = FxHashSet::default();
        let mut queue: VecDeque<(u64, Vec<u64>, u32)> = VecDeque::new();

        // Start node
        queue.push_back((start, Vec::new(), 0));
        visited.insert(start);
        stats.add_nodes_visited(1);

        // Add start node as a result (depth 0)
        results.push(TraversalResult::new(start, start, Vec::new(), 0));

        while let Some((current, path, depth)) = queue.pop_front() {
            // Check depth limit
            if depth >= self.config.max_depth {
                continue;
            }

            // Get neighbors
            let neighbors = get_neighbors(current);
            stats.add_edges_traversed(neighbors.len());

            for (neighbor, edge_id) in neighbors {
                if visited.insert(neighbor) {
                    stats.add_nodes_visited(1);

                    let mut new_path = path.clone();
                    new_path.push(edge_id);

                    // Add result
                    results.push(TraversalResult::new(
                        start,
                        neighbor,
                        new_path.clone(),
                        depth + 1,
                    ));

                    // Continue traversal
                    queue.push_back((neighbor, new_path, depth + 1));

                    // Early exit if we have enough results
                    if results.len() >= self.config.limit {
                        return results;
                    }
                }
            }
        }

        results
    }

    /// Merges and deduplicates results from multiple traversals (EPIC-051 US-004).
    pub(crate) fn merge_and_deduplicate(
        &self,
        results: Vec<TraversalResult>,
    ) -> Vec<TraversalResult> {
        let mut seen: FxHashSet<u64> = FxHashSet::default();
        let mut merged: Vec<TraversalResult> = Vec::new();

        for result in results {
            let signature = result.path_signature();
            if seen.insert(signature) {
                merged.push(result);
            }
        }

        // Sort by score (if present) then by depth
        merged.sort_by(|a, b| match (a.score, b.score) {
            (Some(sa), Some(sb)) => sb.partial_cmp(&sa).unwrap_or(Ordering::Equal),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.depth.cmp(&b.depth),
        });

        // Apply limit
        merged.truncate(self.config.limit);
        merged
    }

    /// Performs parallel DFS from multiple start nodes.
    ///
    /// Similar to `bfs_parallel` but uses depth-first search.
    pub fn dfs_parallel<F>(
        &self,
        start_nodes: &[u64],
        get_neighbors: F,
    ) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Sync,
    {
        let mut stats = TraversalStats::new();
        stats.start_nodes_count = start_nodes.len();

        let results: Vec<Vec<TraversalResult>> =
            if start_nodes.len() >= self.config.parallel_threshold {
                start_nodes
                    .par_iter()
                    .map(|&start| self.dfs_single(start, &get_neighbors, &stats))
                    .collect()
            } else {
                start_nodes
                    .iter()
                    .map(|&start| self.dfs_single(start, &get_neighbors, &stats))
                    .collect()
            };

        let flat_results: Vec<TraversalResult> = results.into_iter().flatten().collect();
        stats.raw_results = flat_results.len();

        let merged = self.merge_and_deduplicate(flat_results);
        stats.deduplicated_results = merged.len();

        (merged, stats)
    }

    /// Performs DFS from a single start node.
    fn dfs_single<F>(
        &self,
        start: u64,
        get_neighbors: &F,
        stats: &TraversalStats,
    ) -> Vec<TraversalResult>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Sync,
    {
        let mut results = Vec::new();
        let mut visited: FxHashSet<u64> = FxHashSet::default();
        let mut stack: Vec<(u64, Vec<u64>, u32)> = Vec::new();

        stack.push((start, Vec::new(), 0));
        visited.insert(start);
        stats.add_nodes_visited(1);

        results.push(TraversalResult::new(start, start, Vec::new(), 0));

        while let Some((current, path, depth)) = stack.pop() {
            if depth >= self.config.max_depth {
                continue;
            }

            let neighbors = get_neighbors(current);
            stats.add_edges_traversed(neighbors.len());

            for (neighbor, edge_id) in neighbors {
                if visited.insert(neighbor) {
                    stats.add_nodes_visited(1);

                    let mut new_path = path.clone();
                    new_path.push(edge_id);

                    results.push(TraversalResult::new(
                        start,
                        neighbor,
                        new_path.clone(),
                        depth + 1,
                    ));

                    stack.push((neighbor, new_path, depth + 1));

                    if results.len() >= self.config.limit {
                        return results;
                    }
                }
            }
        }

        results
    }
}

// Tests moved to parallel_traversal_tests.rs per project rules
