//! ShardedTraverser: shard-parallel traversal for partitioned graphs (EPIC-051 US-003).
//!
//! Designed for graphs that are logically partitioned into shards,
//! handling cross-shard edges transparently.

// SAFETY: Numeric casts in sharded traversal are intentional:
// - u64->usize for node ID hashing: Node IDs are generated sequentially and fit in usize
// - Used for sharding only, actual storage uses u64 for persistence
#![allow(clippy::cast_possible_truncation)]

use super::{ParallelConfig, TraversalResult, TraversalStats};
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;

/// Shard-parallel traversal for partitioned graphs.
///
/// Splits the graph into logical shards and traverses each shard in parallel.
/// Cross-shard edges are collected and processed in subsequent rounds.
///
/// The adjacency closure returns `Vec<(neighbor_id, edge_id)>` tuples.
#[derive(Debug)]
pub struct ShardedTraverser {
    config: ParallelConfig,
    /// Number of shards to use.
    num_shards: usize,
}

impl ShardedTraverser {
    /// Creates a new sharded traverser.
    #[must_use]
    pub fn new(num_shards: usize) -> Self {
        Self {
            config: ParallelConfig::default(),
            num_shards: num_shards.max(1),
        }
    }

    /// Creates with custom config.
    #[must_use]
    pub fn with_config(num_shards: usize, config: ParallelConfig) -> Self {
        Self {
            config,
            num_shards: num_shards.max(1),
        }
    }

    /// Returns the number of shards.
    #[must_use]
    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    /// Determines which shard a node belongs to.
    #[must_use]
    pub fn shard_for_node(&self, node_id: u64) -> usize {
        (node_id as usize) % self.num_shards
    }

    /// Partitions a list of node IDs into shards.
    #[must_use]
    pub fn partition_by_shard(&self, nodes: &[u64]) -> Vec<Vec<u64>> {
        let mut partitions = vec![Vec::new(); self.num_shards];
        for &node in nodes {
            let shard = self.shard_for_node(node);
            partitions[shard].push(node);
        }
        partitions
    }

    /// Executes sharded BFS from multiple start nodes.
    ///
    /// The start nodes themselves are included in results at depth 0.
    ///
    /// Strategy:
    /// 1. Assign start nodes to shards
    /// 2. Run BFS within each shard in parallel
    /// 3. Collect cross-shard edges
    /// 4. Continue BFS from cross-shard frontier
    /// 5. Repeat until max_depth or limit reached
    pub fn traverse_parallel<F>(
        &self,
        start_nodes: &[u64],
        adjacency: F,
    ) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let stats = TraversalStats::new();
        let mut all_results = Vec::new();
        let mut global_visited = FxHashSet::default();

        // Include start nodes at depth 0
        for &start in start_nodes {
            global_visited.insert(start);
            stats.add_nodes_visited(1); // Count start node
            all_results.push(TraversalResult::new(start, start, Vec::new(), 0));
        }

        // Initialize: assign start nodes to shards
        let mut shard_frontiers: Vec<Vec<(u64, u64, Vec<u64>)>> = vec![Vec::new(); self.num_shards];

        for &start in start_nodes {
            let shard = self.shard_for_node(start);
            shard_frontiers[shard].push((start, start, Vec::new()));
        }

        // Iterative BFS with shard parallelism
        for depth in 1..=self.config.max_depth {
            if all_results.len() >= self.config.limit {
                break;
            }

            // Check if any shard has work
            let has_work = shard_frontiers.iter().any(|f| !f.is_empty());
            if !has_work {
                break;
            }

            // Process each shard in parallel
            #[allow(clippy::type_complexity)]
            let shard_results: Vec<(Vec<TraversalResult>, Vec<(u64, u64, Vec<u64>)>)> =
                shard_frontiers
                    .par_iter()
                    .map(|frontier| {
                        let mut results = Vec::new();
                        let mut next_frontier = Vec::new();

                        for (start_node, current_node, path) in frontier {
                            let neighbors = adjacency(*current_node);
                            stats.add_edges_traversed(neighbors.len());

                            // Adjacency returns (neighbor_id, edge_id)
                            for (neighbor, edge_id) in neighbors {
                                let mut new_path = path.clone();
                                new_path.push(edge_id);

                                results.push(TraversalResult::new(
                                    *start_node,
                                    neighbor,
                                    new_path.clone(),
                                    depth,
                                ));

                                next_frontier.push((*start_node, neighbor, new_path));
                            }
                        }

                        (results, next_frontier)
                    })
                    .collect();

            // Merge results and build next frontiers
            let mut new_shard_frontiers: Vec<Vec<(u64, u64, Vec<u64>)>> =
                vec![Vec::new(); self.num_shards];

            // Track nodes newly discovered in this round
            let mut newly_visited = FxHashSet::default();

            for (results, next_frontier) in shard_results {
                for result in results {
                    if global_visited.insert(result.end_node) {
                        stats.add_nodes_visited(1);
                        newly_visited.insert(result.end_node);
                        all_results.push(result);

                        if all_results.len() >= self.config.limit {
                            break;
                        }
                    }
                }

                // Distribute next frontier nodes to their shards
                // Include nodes that were newly discovered (they need expansion)
                for (start, node, path) in next_frontier {
                    if !newly_visited.contains(&node) {
                        // Node was already visited in a previous round
                        continue;
                    }
                    let shard = self.shard_for_node(node);
                    new_shard_frontiers[shard].push((start, node, path));
                }
            }

            shard_frontiers = new_shard_frontiers;
        }

        let mut final_stats = stats;
        final_stats.start_nodes_count = start_nodes.len();
        final_stats.raw_results = all_results.len();
        final_stats.deduplicated_results = all_results.len();

        (all_results, final_stats)
    }

    /// Executes BFS within a single shard (for testing/debugging).
    pub fn bfs_single_shard<F>(
        &self,
        start: u64,
        adjacency: &F,
        stats: &TraversalStats,
    ) -> Vec<TraversalResult>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let target_shard = self.shard_for_node(start);
        let mut results = Vec::new();
        let mut visited = FxHashSet::default();
        let mut queue: VecDeque<(u64, Vec<u64>, u32)> = VecDeque::new();

        visited.insert(start);
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
                // Only follow edges within the same shard
                if self.shard_for_node(neighbor) == target_shard && visited.insert(neighbor) {
                    stats.add_nodes_visited(1);
                    let mut new_path = path.clone();
                    new_path.push(edge_id);
                    results.push(TraversalResult::new(
                        start,
                        neighbor,
                        new_path.clone(),
                        depth + 1,
                    ));
                    queue.push_back((neighbor, new_path, depth + 1));
                }
            }
        }

        results
    }
}
