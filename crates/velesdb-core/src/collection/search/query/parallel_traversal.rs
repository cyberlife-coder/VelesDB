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
        self.nodes_visited
            .fetch_add(count, AtomicOrdering::Relaxed);
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
        let results: Vec<Vec<TraversalResult>> = if start_nodes.len() >= self.config.parallel_threshold
        {
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
    fn merge_and_deduplicate(&self, results: Vec<TraversalResult>) -> Vec<TraversalResult> {
        let mut seen: FxHashSet<u64> = FxHashSet::default();
        let mut merged: Vec<TraversalResult> = Vec::new();

        for result in results {
            let signature = result.path_signature();
            if seen.insert(signature) {
                merged.push(result);
            }
        }

        // Sort by score (if present) then by depth
        merged.sort_by(|a, b| {
            match (a.score, b.score) {
                (Some(sa), Some(sb)) => sb.partial_cmp(&sa).unwrap_or(Ordering::Equal),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => a.depth.cmp(&b.depth),
            }
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

        let results: Vec<Vec<TraversalResult>> = if start_nodes.len() >= self.config.parallel_threshold
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Creates a simple test graph for testing.
    fn create_test_graph() -> HashMap<u64, Vec<(u64, u64)>> {
        let mut graph = HashMap::new();

        // Graph structure:
        //   1 -> 2 -> 4
        //   |    |
        //   v    v
        //   3 -> 5
        //   |
        //   v
        //   6

        graph.insert(1, vec![(2, 100), (3, 101)]);
        graph.insert(2, vec![(4, 102), (5, 103)]);
        graph.insert(3, vec![(5, 104), (6, 105)]);
        graph.insert(4, vec![]);
        graph.insert(5, vec![]);
        graph.insert(6, vec![]);

        graph
    }

    #[test]
    fn test_traversal_result_new() {
        let result = TraversalResult::new(1, 5, vec![100, 103], 2);
        assert_eq!(result.start_node, 1);
        assert_eq!(result.end_node, 5);
        assert_eq!(result.depth, 2);
        assert!(result.score.is_none());
    }

    #[test]
    fn test_traversal_result_with_score() {
        let result = TraversalResult::new(1, 5, vec![100], 1).with_score(0.9);
        assert_eq!(result.score, Some(0.9));
    }

    #[test]
    fn test_path_signature_uniqueness() {
        let r1 = TraversalResult::new(1, 5, vec![100, 101], 2);
        let r2 = TraversalResult::new(1, 5, vec![100, 102], 2);
        let r3 = TraversalResult::new(1, 5, vec![100, 101], 2);

        assert_ne!(r1.path_signature(), r2.path_signature());
        assert_eq!(r1.path_signature(), r3.path_signature());
    }

    #[test]
    fn test_parallel_config_default() {
        let config = ParallelConfig::default();
        assert_eq!(config.max_depth, 5);
        assert_eq!(config.parallel_threshold, 100);
        assert_eq!(config.limit, 1000);
    }

    #[test]
    fn test_traversal_stats() {
        let stats = TraversalStats::new();
        stats.add_nodes_visited(10);
        stats.add_edges_traversed(20);

        assert_eq!(stats.total_nodes_visited(), 10);
        assert_eq!(stats.total_edges_traversed(), 20);
    }

    #[test]
    fn test_bfs_single_start() {
        let graph = create_test_graph();
        let traverser = ParallelTraverser::with_config(ParallelConfig {
            max_depth: 3,
            parallel_threshold: 1,
            limit: 100,
            relationship_types: vec![],
        });

        let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
            graph.get(&node).cloned().unwrap_or_default()
        };

        let (results, stats) = traverser.bfs_parallel(&[1], get_neighbors);

        // Should find all 6 nodes
        assert_eq!(results.len(), 6);
        assert_eq!(stats.start_nodes_count, 1);
        assert!(stats.total_nodes_visited() >= 6);
    }

    #[test]
    fn test_bfs_multiple_starts() {
        let graph = create_test_graph();
        let traverser = ParallelTraverser::with_config(ParallelConfig {
            max_depth: 2,
            parallel_threshold: 1, // Force parallel even for small input
            limit: 100,
            relationship_types: vec![],
        });

        let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
            graph.get(&node).cloned().unwrap_or_default()
        };

        let (results, stats) = traverser.bfs_parallel(&[1, 3], get_neighbors);

        assert_eq!(stats.start_nodes_count, 2);
        // Results should be deduplicated
        assert!(results.len() >= 2);
    }

    #[test]
    fn test_bfs_depth_limit() {
        let graph = create_test_graph();
        let traverser = ParallelTraverser::with_config(ParallelConfig {
            max_depth: 1,
            parallel_threshold: 1,
            limit: 100,
            relationship_types: vec![],
        });

        let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
            graph.get(&node).cloned().unwrap_or_default()
        };

        let (results, _) = traverser.bfs_parallel(&[1], get_neighbors);

        // At depth 1, should only reach immediate neighbors (1, 2, 3)
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.depth <= 1));
    }

    #[test]
    fn test_dfs_single_start() {
        let graph = create_test_graph();
        let traverser = ParallelTraverser::with_config(ParallelConfig {
            max_depth: 3,
            parallel_threshold: 1,
            limit: 100,
            relationship_types: vec![],
        });

        let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
            graph.get(&node).cloned().unwrap_or_default()
        };

        let (results, stats) = traverser.dfs_parallel(&[1], get_neighbors);

        // Should find all 6 nodes
        assert_eq!(results.len(), 6);
        assert_eq!(stats.start_nodes_count, 1);
    }

    #[test]
    fn test_merge_deduplication() {
        let traverser = ParallelTraverser::new();

        let results = vec![
            TraversalResult::new(1, 2, vec![100], 1),
            TraversalResult::new(1, 2, vec![100], 1), // Duplicate
            TraversalResult::new(1, 3, vec![101], 1),
        ];

        let merged = traverser.merge_and_deduplicate(results);
        assert_eq!(merged.len(), 2); // One duplicate removed
    }

    #[test]
    fn test_merge_sorting_by_score() {
        let traverser = ParallelTraverser::new();

        let results = vec![
            TraversalResult::new(1, 2, vec![100], 1).with_score(0.5),
            TraversalResult::new(1, 3, vec![101], 1).with_score(0.9),
            TraversalResult::new(1, 4, vec![102], 1).with_score(0.7),
        ];

        let merged = traverser.merge_and_deduplicate(results);

        // Should be sorted by score descending
        assert_eq!(merged[0].score, Some(0.9));
        assert_eq!(merged[1].score, Some(0.7));
        assert_eq!(merged[2].score, Some(0.5));
    }

    #[test]
    fn test_result_limit() {
        let traverser = ParallelTraverser::with_config(ParallelConfig {
            max_depth: 10,
            parallel_threshold: 1,
            limit: 3,
            relationship_types: vec![],
        });

        // Create a larger graph
        let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
            if node < 100 {
                vec![(node + 1, node * 10), (node + 2, node * 10 + 1)]
            } else {
                vec![]
            }
        };

        let (results, _) = traverser.bfs_parallel(&[1], get_neighbors);

        // Should be limited to 3 results
        assert!(results.len() <= 3);
    }
}
