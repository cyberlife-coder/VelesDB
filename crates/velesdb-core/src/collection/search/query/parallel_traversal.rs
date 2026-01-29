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
            (Some(sa), Some(sb)) => sb.total_cmp(&sa),
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

/// Frontier-parallel BFS for single-start traversals (EPIC-051 US-002).
///
/// This implementation parallelizes each level (frontier) of the BFS,
/// which is more efficient for traversals from a single start node
/// with large fanout at each level.
#[derive(Debug, Default)]
pub struct FrontierParallelBFS {
    config: ParallelConfig,
}

impl FrontierParallelBFS {
    /// Creates a new frontier-parallel BFS traverser.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates with custom configuration.
    #[must_use]
    pub fn with_config(config: ParallelConfig) -> Self {
        Self { config }
    }

    /// Performs frontier-parallel BFS from a single start node.
    ///
    /// Each BFS level is expanded in parallel using rayon.
    /// Uses a thread-safe visited set (DashSet-like behavior via atomic).
    ///
    /// # Arguments
    ///
    /// * `start` - Starting node ID
    /// * `get_neighbors` - Function returning (neighbor_id, edge_id) pairs
    ///
    /// # Returns
    ///
    /// Traversal results and statistics.
    pub fn traverse<F>(
        &self,
        start: u64,
        get_neighbors: F,
    ) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Sync,
    {
        use parking_lot::RwLock;
        use std::collections::HashSet;

        let mut stats = TraversalStats::new();
        stats.add_nodes_visited(1);

        // Thread-safe visited set using RwLock
        let visited: RwLock<HashSet<u64>> = RwLock::new(HashSet::new());
        visited.write().insert(start);

        let mut all_results: Vec<TraversalResult> = Vec::new();
        all_results.push(TraversalResult::new(start, start, Vec::new(), 0));

        // Current frontier: (node_id, path_to_node)
        let mut current_frontier: Vec<(u64, Vec<u64>)> = vec![(start, Vec::new())];
        let mut depth: u32 = 0;

        while !current_frontier.is_empty() && depth < self.config.max_depth {
            depth += 1;

            // BUG-5 FIX: Collect all candidates in parallel WITHOUT locking,
            // then deduplicate in a single pass to avoid lock serialization.
            let candidates: Vec<(u64, Vec<u64>, u64)> =
                if current_frontier.len() >= self.config.parallel_threshold {
                    current_frontier
                        .par_iter()
                        .flat_map(|(node, path)| {
                            let neighbors = get_neighbors(*node);
                            stats.add_edges_traversed(neighbors.len());

                            neighbors
                                .into_iter()
                                .map(|(neighbor, edge_id)| {
                                    let mut new_path = path.clone();
                                    new_path.push(edge_id);
                                    (neighbor, new_path, edge_id)
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect()
                } else {
                    // Sequential for small frontiers
                    current_frontier
                        .iter()
                        .flat_map(|(node, path)| {
                            let neighbors = get_neighbors(*node);
                            stats.add_edges_traversed(neighbors.len());

                            neighbors
                                .into_iter()
                                .map(|(neighbor, edge_id)| {
                                    let mut new_path = path.clone();
                                    new_path.push(edge_id);
                                    (neighbor, new_path, edge_id)
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect()
                };

            // Deduplicate in a single pass (no lock contention)
            let mut visited_guard = visited.write();
            let next_frontier: Vec<(u64, Vec<u64>, u64)> = candidates
                .into_iter()
                .filter(|(neighbor, _, _)| {
                    if visited_guard.insert(*neighbor) {
                        stats.add_nodes_visited(1);
                        true
                    } else {
                        false
                    }
                })
                .collect();
            drop(visited_guard);

            // Add results from this level
            for (neighbor, path, _) in &next_frontier {
                all_results.push(TraversalResult::new(start, *neighbor, path.clone(), depth));

                // Early exit if limit reached
                if all_results.len() >= self.config.limit {
                    // BUG-8 FIX: Use accumulated stats instead of creating new empty ones
                    stats.start_nodes_count = 1;
                    stats.raw_results = all_results.len();
                    stats.deduplicated_results = all_results.len();
                    return (all_results, stats);
                }
            }

            // Update frontier for next level
            current_frontier = next_frontier
                .into_iter()
                .map(|(node, path, _)| (node, path))
                .collect();
        }

        // BUG-8 FIX: Use accumulated stats (nodes_visited, edges_traversed already set)
        stats.start_nodes_count = 1;
        stats.raw_results = all_results.len();
        stats.deduplicated_results = all_results.len();

        (all_results, stats)
    }
}

/// Shard-parallel traverser for partitioned graphs (EPIC-051 US-003).
///
/// This traverser partitions nodes by shard and traverses each shard
/// in parallel, then handles cross-shard edges.
#[derive(Debug, Default)]
pub struct ShardedTraverser {
    /// Number of shards.
    num_shards: usize,
    /// Configuration for parallel traversal.
    config: ParallelConfig,
}

impl ShardedTraverser {
    /// Creates a new sharded traverser with the given number of shards.
    #[must_use]
    pub fn new(num_shards: usize) -> Self {
        Self {
            num_shards,
            config: ParallelConfig::default(),
        }
    }

    /// Creates with custom configuration.
    #[must_use]
    pub fn with_config(num_shards: usize, config: ParallelConfig) -> Self {
        Self { num_shards, config }
    }

    /// Determines which shard a node belongs to.
    #[must_use]
    pub fn shard_for_node(&self, node_id: u64) -> usize {
        (node_id as usize) % self.num_shards
    }

    /// Partitions nodes by their shard assignment.
    #[must_use]
    pub fn partition_by_shard(&self, nodes: &[u64]) -> Vec<Vec<u64>> {
        let mut shards: Vec<Vec<u64>> = vec![Vec::new(); self.num_shards];
        for &node in nodes {
            let shard_id = self.shard_for_node(node);
            shards[shard_id].push(node);
        }
        shards
    }

    /// Performs shard-parallel traversal from multiple start nodes.
    ///
    /// 1. Partitions start nodes by shard
    /// 2. Traverses each shard in parallel
    /// 3. Resolves cross-shard edges
    /// 4. Merges and deduplicates results
    pub fn traverse_parallel<F>(
        &self,
        start_nodes: &[u64],
        get_neighbors: F,
    ) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Sync,
    {
        let mut stats = TraversalStats::new();
        stats.start_nodes_count = start_nodes.len();

        // Step 1: Partition nodes by shard
        let nodes_by_shard = self.partition_by_shard(start_nodes);

        // Step 2: Traverse each shard in parallel
        let shard_results: Vec<Vec<TraversalResult>> =
            if self.config.should_parallelize(start_nodes.len()) {
                nodes_by_shard
                    .into_par_iter()
                    .enumerate()
                    .filter(|(_, nodes)| !nodes.is_empty())
                    .map(|(shard_id, nodes)| {
                        self.traverse_shard(shard_id, &nodes, &get_neighbors, &stats)
                    })
                    .collect()
            } else {
                nodes_by_shard
                    .into_iter()
                    .enumerate()
                    .filter(|(_, nodes)| !nodes.is_empty())
                    .map(|(shard_id, nodes)| {
                        self.traverse_shard(shard_id, &nodes, &get_neighbors, &stats)
                    })
                    .collect()
            };

        // Step 3: Flatten and collect results
        let mut all_results: Vec<TraversalResult> = shard_results.into_iter().flatten().collect();
        stats.raw_results = all_results.len();

        // Step 4: Deduplicate by path signature
        let mut seen: FxHashSet<u64> = FxHashSet::default();
        all_results.retain(|r| seen.insert(r.path_signature()));
        stats.deduplicated_results = all_results.len();

        // Sort by depth
        all_results.sort_by_key(|r| r.depth);

        // Apply limit
        all_results.truncate(self.config.limit);

        (all_results, stats)
    }

    /// Traverses a single shard using BFS.
    fn traverse_shard<F>(
        &self,
        _shard_id: usize,
        start_nodes: &[u64],
        get_neighbors: &F,
        stats: &TraversalStats,
    ) -> Vec<TraversalResult>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Sync,
    {
        use std::collections::VecDeque;

        let mut results = Vec::new();
        let mut visited: FxHashSet<u64> = FxHashSet::default();
        let mut queue: VecDeque<(u64, u64, Vec<u64>, u32)> = VecDeque::new();

        // Initialize with all start nodes
        for &start in start_nodes {
            queue.push_back((start, start, Vec::new(), 0));
            visited.insert(start);
            stats.add_nodes_visited(1);
            results.push(TraversalResult::new(start, start, Vec::new(), 0));
        }

        while let Some((start, current, path, depth)) = queue.pop_front() {
            if depth >= self.config.max_depth {
                continue;
            }

            let neighbors = get_neighbors(current);
            stats.add_edges_traversed(neighbors.len());

            for (neighbor, edge_id) in neighbors {
                // Follow edges even if they cross shards
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

                    queue.push_back((start, neighbor, new_path, depth + 1));

                    if results.len() >= self.config.limit {
                        return results;
                    }
                }
            }
        }

        results
    }

    /// Gets the number of shards.
    #[must_use]
    pub fn num_shards(&self) -> usize {
        self.num_shards
    }
}

// Tests moved to parallel_traversal_tests.rs per project rules
