//! FrontierParallelBFS: single-start, frontier-parallel BFS (EPIC-051 US-002).
//!
//! Instead of parallelizing across start nodes, this parallelizes the
//! expansion of each BFS level's frontier using rayon.

use super::{ParallelConfig, TraversalResult, TraversalStats};
use rayon::prelude::*;
use rustc_hash::FxHashSet;

/// Frontier-parallel BFS: parallelizes each level expansion.
///
/// Best for single-start-node queries with wide graphs (high branching factor).
/// Each BFS level's frontier is expanded in parallel using rayon.
///
/// The adjacency closure returns `Vec<(neighbor_id, edge_id)>` tuples.
#[derive(Debug)]
pub struct FrontierParallelBFS {
    config: ParallelConfig,
}

impl FrontierParallelBFS {
    /// Creates a new frontier-parallel BFS with default config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: ParallelConfig::default(),
        }
    }

    /// Creates with the given config.
    #[must_use]
    pub fn with_config(config: ParallelConfig) -> Self {
        Self { config }
    }

    /// Executes frontier-parallel BFS from a single start node.
    ///
    /// The start node itself is included in results at depth 0.
    /// Each level of the BFS is expanded in parallel when the frontier
    /// exceeds `min_frontier_for_parallel`.
    pub fn traverse<F>(&self, start: u64, adjacency: F) -> (Vec<TraversalResult>, TraversalStats)
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let stats = TraversalStats::new();
        let mut results = Vec::new();
        let mut visited = FxHashSet::default();
        visited.insert(start);

        results.push(TraversalResult::new(start, start, Vec::new(), 0));

        let mut frontier: Vec<(u64, Vec<u64>)> = vec![(start, Vec::new())];
        let mut depth = 0u32;

        while !frontier.is_empty() && depth < self.config.max_depth {
            depth += 1;

            // Bound expansion to the remaining result budget so a wide level
            // never materializes more neighbors than can still be returned. The
            // cap counts only NEW unique nodes (candidates already in `visited`
            // or duplicated within the level are filtered before the cap), so it
            // bounds new results -- never raw candidates -- and cannot under-fill
            // below the limit.
            let remaining = self.config.limit.saturating_sub(results.len());
            if remaining == 0 {
                break;
            }
            let next_frontier =
                self.expand_frontier(&frontier, &adjacency, &stats, remaining, &visited);

            frontier = Vec::new();
            for (neighbor, path, _edge_id) in next_frontier {
                if visited.insert(neighbor) {
                    stats.add_nodes_visited(1);
                    results.push(TraversalResult::new(start, neighbor, path.clone(), depth));
                    frontier.push((neighbor, path));

                    if results.len() >= self.config.limit {
                        let count = results.len();
                        return (results, Self::finalize_stats(stats, count));
                    }
                }
            }
        }

        let count = results.len();
        (results, Self::finalize_stats(stats, count))
    }

    /// Expands the frontier (parallel or sequential based on size).
    ///
    /// Candidate neighbors are produced (in parallel for wide levels), then
    /// deterministically filtered against `visited` and deduplicated within the
    /// level so that *at most* `remaining` genuinely-new unique nodes are kept.
    /// Because the cap counts only new unique nodes -- not raw (possibly
    /// already-visited or within-level duplicate) candidates -- it bounds peak
    /// buffering without ever dropping a node that would fit inside the limit.
    /// The caller's `visited.insert` remains the authoritative dedup; here it is
    /// a no-op given the pre-filter, preserving BFS level ordering.
    fn expand_frontier<F>(
        &self,
        frontier: &[(u64, Vec<u64>)],
        adjacency: &F,
        stats: &TraversalStats,
        remaining: usize,
        visited: &FxHashSet<u64>,
    ) -> Vec<(u64, Vec<u64>, u64)>
    where
        F: Fn(u64) -> Vec<(u64, u64)> + Send + Sync,
    {
        let expand_node = |node: &u64, path: &Vec<u64>| {
            let neighbors = adjacency(*node);
            stats.add_edges_traversed(neighbors.len());
            neighbors
                .into_iter()
                .map(|(neighbor, edge_id)| {
                    let mut new_path = path.clone();
                    new_path.push(edge_id);
                    (neighbor, new_path, edge_id)
                })
                .collect::<Vec<_>>()
        };

        // Keep only genuinely-new unique nodes, in frontier order, capped at the
        // remaining budget. The wide-level parallel branch expands neighbors in
        // parallel (`collect` preserves order) and the cheap dedup runs after;
        // the sequential branch keeps the lazy `take` so a deep narrow frontier
        // never materializes neighbors past the budget. Both are deterministic.
        let mut level_seen = FxHashSet::default();
        let mut keep = |neighbor: u64| !visited.contains(&neighbor) && level_seen.insert(neighbor);

        if self.config.should_parallelize_frontier(frontier.len()) {
            let candidates: Vec<(u64, Vec<u64>, u64)> = frontier
                .par_iter()
                .flat_map(|(node, path)| expand_node(node, path))
                .collect();
            candidates
                .into_iter()
                .filter(|(neighbor, _, _)| keep(*neighbor))
                .take(remaining)
                .collect()
        } else {
            frontier
                .iter()
                .flat_map(|(node, path)| expand_node(node, path))
                .filter(|(neighbor, _, _)| keep(*neighbor))
                .take(remaining)
                .collect()
        }
    }

    /// Finalizes traversal stats.
    fn finalize_stats(mut stats: TraversalStats, result_count: usize) -> TraversalStats {
        stats.start_nodes_count = 1;
        stats.raw_results = result_count;
        stats.deduplicated_results = result_count;
        stats
    }
}

impl Default for FrontierParallelBFS {
    fn default() -> Self {
        Self::new()
    }
}
