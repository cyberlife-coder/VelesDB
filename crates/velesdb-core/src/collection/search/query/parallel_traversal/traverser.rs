//! ParallelTraverser: parallel BFS/DFS across multiple start nodes (EPIC-051 US-001).
//!
//! Uses rayon to parallelize traversal from independent start nodes.

use super::{ParallelConfig, TraversalResult, TraversalStats};
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;

/// Parallel traversal engine using rayon for start-node parallelism.
///
/// Each start node's traversal is independent, making it embarrassingly parallel.
/// Results are merged and deduplicated after all traversals complete.
///
/// The adjacency closure returns `Vec<(neighbor_id, edge_id)>` tuples.
#[derive(Debug, Default)]
pub struct ParallelTraverser {
    config: ParallelConfig,
}

impl ParallelTraverser {
    /// Creates a new parallel traverser with default config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: ParallelConfig::default(),
        }
    }

    /// Creates a traverser with the given config.
    #[must_use]
    pub fn with_config(config: ParallelConfig) -> Self {
        Self { config }
    }

    /// Executes BFS from multiple start nodes in parallel.
    ///
    /// Returns deduplicated results sorted by score (descending).
    /// The start node itself is included in results at depth 0.
    ///
    /// # Arguments
    /// * `start_nodes` - Node IDs to start BFS from
    /// * `adjacency` - Closure returning `Vec<(neighbor_id, edge_id)>` for a given node
    pub fn bfs_parallel<F>(
        &self,
        start_nodes: &[u64],
        adjacency: F,
    ) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let stats = TraversalStats::new();

        let results: Vec<Vec<TraversalResult>> =
            if self.config.should_parallelize(start_nodes.len()) {
                // Parallel: each start node gets its own BFS
                start_nodes
                    .par_iter()
                    .map(|&start| self.bfs_single(start, &adjacency, &stats))
                    .collect()
            } else {
                // Sequential for small inputs
                start_nodes
                    .iter()
                    .map(|&start| self.bfs_single(start, &adjacency, &stats))
                    .collect()
            };

        let all_results: Vec<TraversalResult> = results.into_iter().flatten().collect();
        let raw_count = all_results.len();

        let deduplicated = self.merge_and_deduplicate(all_results);

        let mut final_stats = stats;
        final_stats.start_nodes_count = start_nodes.len();
        final_stats.raw_results = raw_count;
        final_stats.deduplicated_results = deduplicated.len();

        (deduplicated, final_stats)
    }

    /// BFS from a single start node (includes start node at depth 0).
    fn bfs_single<F>(
        &self,
        start: u64,
        adjacency: &F,
        stats: &TraversalStats,
    ) -> Vec<TraversalResult>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let mut results = Vec::new();
        let mut visited = FxHashSet::default();
        let mut queue: VecDeque<(u64, Vec<u64>, u32)> = VecDeque::new();

        visited.insert(start);
        stats.add_nodes_visited(1); // Count start node
                                    // Include start node at depth 0
        results.push(TraversalResult::new(start, start, Vec::new(), 0));
        queue.push_back((start, Vec::new(), 0));

        while let Some((node, path, depth)) = queue.pop_front() {
            if depth >= self.config.max_depth || results.len() >= self.config.limit {
                break;
            }

            let neighbors = adjacency(node);
            stats.add_edges_traversed(neighbors.len());

            // Adjacency returns (neighbor_id, edge_id)
            for (neighbor, edge_id) in neighbors {
                if visited.insert(neighbor) {
                    stats.add_nodes_visited(1);
                    let mut new_path = path.clone();
                    new_path.push(edge_id);
                    let new_depth = depth + 1;
                    results.push(TraversalResult::new(
                        start,
                        neighbor,
                        new_path.clone(),
                        new_depth,
                    ));
                    queue.push_back((neighbor, new_path, new_depth));
                }
            }
        }

        results
    }

    /// Merge results from multiple traversals and deduplicate by path signature.
    ///
    /// Results are sorted by score descending (highest first).
    /// Applies the configured limit after deduplication.
    #[must_use]
    pub fn merge_and_deduplicate(&self, results: Vec<TraversalResult>) -> Vec<TraversalResult> {
        let mut seen = FxHashSet::default();
        let mut unique: Vec<TraversalResult> = results
            .into_iter()
            .filter(|r| seen.insert(r.path_signature()))
            .collect();

        // Sort by score descending (highest first), break ties by depth ascending
        unique.sort_by(|a, b| {
            let score_cmp = b
                .score
                .unwrap_or(f32::NEG_INFINITY)
                .total_cmp(&a.score.unwrap_or(f32::NEG_INFINITY));
            score_cmp.then_with(|| a.depth.cmp(&b.depth))
        });
        unique.truncate(self.config.limit);
        unique
    }

    /// Executes DFS from multiple start nodes in parallel.
    pub fn dfs_parallel<F>(
        &self,
        start_nodes: &[u64],
        adjacency: F,
    ) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let stats = TraversalStats::new();

        let results: Vec<Vec<TraversalResult>> =
            if self.config.should_parallelize(start_nodes.len()) {
                start_nodes
                    .par_iter()
                    .map(|&start| self.dfs_single(start, &adjacency, &stats))
                    .collect()
            } else {
                start_nodes
                    .iter()
                    .map(|&start| self.dfs_single(start, &adjacency, &stats))
                    .collect()
            };

        let all_results: Vec<TraversalResult> = results.into_iter().flatten().collect();
        let raw_count = all_results.len();
        let deduplicated = self.merge_and_deduplicate(all_results);

        let mut final_stats = stats;
        final_stats.start_nodes_count = start_nodes.len();
        final_stats.raw_results = raw_count;
        final_stats.deduplicated_results = deduplicated.len();

        (deduplicated, final_stats)
    }

    /// DFS from a single start node (includes start node at depth 0).
    fn dfs_single<F>(
        &self,
        start: u64,
        adjacency: &F,
        stats: &TraversalStats,
    ) -> Vec<TraversalResult>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let mut results = Vec::new();
        let mut visited = FxHashSet::default();
        let mut stack: Vec<(u64, Vec<u64>, u32)> = Vec::new();

        visited.insert(start);
        // Include start node at depth 0
        results.push(TraversalResult::new(start, start, Vec::new(), 0));
        stack.push((start, Vec::new(), 0));

        while let Some((node, path, depth)) = stack.pop() {
            if depth >= self.config.max_depth || results.len() >= self.config.limit {
                continue;
            }

            let neighbors = adjacency(node);
            stats.add_edges_traversed(neighbors.len());

            // Adjacency returns (neighbor_id, edge_id)
            for (neighbor, edge_id) in neighbors {
                if visited.insert(neighbor) {
                    stats.add_nodes_visited(1);
                    let mut new_path = path.clone();
                    new_path.push(edge_id);
                    let new_depth = depth + 1;
                    results.push(TraversalResult::new(
                        start,
                        neighbor,
                        new_path.clone(),
                        new_depth,
                    ));
                    stack.push((neighbor, new_path, new_depth));
                }
            }
        }

        results
    }
}
