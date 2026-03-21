//! ShardedTraverser: shard-parallel traversal for partitioned graphs (EPIC-051 US-003).
//!
//! Designed for graphs that are logically partitioned into shards,
//! handling cross-shard edges transparently.

// SAFETY: Numeric casts in sharded traversal are intentional:
// - u64->usize for node ID hashing: Node IDs are generated sequentially and fit in usize
// - Used for sharding only, actual storage uses u64 for persistence
#![allow(clippy::cast_possible_truncation)]

use super::{bfs_core, ParallelConfig, TraversalResult, TraversalStats};
use rayon::prelude::*;
use rustc_hash::FxHashSet;

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

        for &start in start_nodes {
            global_visited.insert(start);
            stats.add_nodes_visited(1);
            all_results.push(TraversalResult::new(start, start, Vec::new(), 0));
        }

        let mut shard_frontiers = self.initialize_frontiers(start_nodes);

        for depth in 1..=self.config.max_depth {
            if all_results.len() >= self.config.limit {
                break;
            }
            if shard_frontiers.iter().all(Vec::is_empty) {
                break;
            }

            let shard_results = self.expand_shards(&shard_frontiers, &adjacency, &stats, depth);

            shard_frontiers = self.merge_shard_results(
                shard_results,
                &mut global_visited,
                &stats,
                &mut all_results,
            );
        }

        let mut final_stats = stats;
        final_stats.start_nodes_count = start_nodes.len();
        final_stats.raw_results = all_results.len();
        final_stats.deduplicated_results = all_results.len();
        (all_results, final_stats)
    }

    /// Initializes shard frontiers from start nodes.
    fn initialize_frontiers(&self, start_nodes: &[u64]) -> Vec<Vec<(u64, u64, Vec<u64>)>> {
        let mut frontiers = vec![Vec::new(); self.num_shards];
        for &start in start_nodes {
            frontiers[self.shard_for_node(start)].push((start, start, Vec::new()));
        }
        frontiers
    }

    /// Expands all shard frontiers in parallel.
    #[allow(clippy::type_complexity, clippy::unused_self)]
    fn expand_shards<F>(
        &self,
        shard_frontiers: &[Vec<(u64, u64, Vec<u64>)>],
        adjacency: &F,
        stats: &TraversalStats,
        depth: u32,
    ) -> Vec<(Vec<TraversalResult>, Vec<(u64, u64, Vec<u64>)>)>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        shard_frontiers
            .par_iter()
            .map(|frontier| {
                let mut results = Vec::new();
                let mut next_frontier = Vec::new();
                for (start_node, current_node, path) in frontier {
                    let neighbors = adjacency(*current_node);
                    stats.add_edges_traversed(neighbors.len());
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
            .collect()
    }

    /// Merges shard results, deduplicates, and builds next frontiers.
    #[allow(clippy::type_complexity)]
    fn merge_shard_results(
        &self,
        shard_results: Vec<(Vec<TraversalResult>, Vec<(u64, u64, Vec<u64>)>)>,
        global_visited: &mut FxHashSet<u64>,
        stats: &TraversalStats,
        all_results: &mut Vec<TraversalResult>,
    ) -> Vec<Vec<(u64, u64, Vec<u64>)>> {
        let mut new_frontiers = vec![Vec::new(); self.num_shards];
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
            for (start, node, path) in next_frontier {
                if newly_visited.contains(&node) {
                    new_frontiers[self.shard_for_node(node)].push((start, node, path));
                }
            }
        }
        new_frontiers
    }

    /// Executes BFS within a single shard (for testing/debugging).
    ///
    /// Only follows edges whose target belongs to the same shard as `start`.
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
        let shard_filter = |neighbor: u64| self.shard_for_node(neighbor) == target_shard;
        bfs_core(start, adjacency, stats, &self.config, Some(&shard_filter))
    }
}
