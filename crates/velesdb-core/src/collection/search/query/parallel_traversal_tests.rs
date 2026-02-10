//! Tests for `parallel_traversal` module - Parallel graph traversal.

use super::parallel_traversal::*;
use std::collections::HashMap;

fn create_test_graph() -> HashMap<u64, Vec<(u64, u64)>> {
    let mut graph = HashMap::new();
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
    let traverser = ParallelTraverser::with_config(
        ParallelConfig::new()
            .with_max_depth(3)
            .with_parallel_threshold(1)
            .with_limit(100),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, stats) = traverser.bfs_parallel(&[1], get_neighbors);

    assert_eq!(results.len(), 6);
    assert_eq!(stats.start_nodes_count, 1);
    assert!(stats.total_nodes_visited() >= 6);
}

#[test]
fn test_bfs_multiple_starts() {
    let graph = create_test_graph();
    let traverser = ParallelTraverser::with_config(
        ParallelConfig::new()
            .with_max_depth(2)
            .with_parallel_threshold(1)
            .with_limit(100),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, stats) = traverser.bfs_parallel(&[1, 3], get_neighbors);

    assert_eq!(stats.start_nodes_count, 2);
    assert!(results.len() >= 2);
}

#[test]
fn test_bfs_depth_limit() {
    let graph = create_test_graph();
    let traverser = ParallelTraverser::with_config(
        ParallelConfig::new()
            .with_max_depth(1)
            .with_parallel_threshold(1)
            .with_limit(100),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, _) = traverser.bfs_parallel(&[1], get_neighbors);

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.depth <= 1));
}

#[test]
fn test_dfs_single_start() {
    let graph = create_test_graph();
    let traverser = ParallelTraverser::with_config(
        ParallelConfig::new()
            .with_max_depth(3)
            .with_parallel_threshold(1)
            .with_limit(100),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, stats) = traverser.dfs_parallel(&[1], get_neighbors);

    assert_eq!(results.len(), 6);
    assert_eq!(stats.start_nodes_count, 1);
}

#[test]
fn test_merge_deduplication() {
    let traverser = ParallelTraverser::new();

    let results = vec![
        TraversalResult::new(1, 2, vec![100], 1),
        TraversalResult::new(1, 2, vec![100], 1),
        TraversalResult::new(1, 3, vec![101], 1),
    ];

    let merged = traverser.merge_and_deduplicate(results);
    assert_eq!(merged.len(), 2);
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

    assert_eq!(merged[0].score, Some(0.9));
    assert_eq!(merged[1].score, Some(0.7));
    assert_eq!(merged[2].score, Some(0.5));
}

#[test]
fn test_result_limit() {
    let traverser = ParallelTraverser::with_config(
        ParallelConfig::new()
            .with_max_depth(10)
            .with_parallel_threshold(1)
            .with_limit(3),
    );

    let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
        if node < 100 {
            vec![(node + 1, node * 10), (node + 2, node * 10 + 1)]
        } else {
            vec![]
        }
    };

    let (results, _) = traverser.bfs_parallel(&[1], get_neighbors);

    assert!(results.len() <= 3);
}

// ============================================================================
// EPIC-051 US-002: FrontierParallelBFS Tests
// ============================================================================

#[test]
fn test_frontier_parallel_bfs_basic() {
    let graph = create_test_graph();
    let bfs = FrontierParallelBFS::new();

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, stats) = bfs.traverse(1, get_neighbors);

    // Should visit all reachable nodes: 1, 2, 3, 4, 5, 6
    assert!(results.len() >= 6);
    assert_eq!(stats.start_nodes_count, 1);
}

#[test]
fn test_frontier_parallel_bfs_no_duplicates() {
    let graph = create_test_graph();
    let bfs = FrontierParallelBFS::new();

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, _) = bfs.traverse(1, get_neighbors);

    // Check no duplicate end nodes
    let mut seen_ends: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for result in &results {
        assert!(
            seen_ends.insert(result.end_node),
            "Duplicate end node: {}",
            result.end_node
        );
    }
}

#[test]
fn test_frontier_parallel_bfs_depth_order() {
    let graph = create_test_graph();
    let bfs = FrontierParallelBFS::new();

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, _) = bfs.traverse(1, get_neighbors);

    // Verify results are ordered by depth (BFS property)
    let mut last_depth = 0;
    for result in &results {
        assert!(
            result.depth >= last_depth || result.depth == 0,
            "Results not in BFS order"
        );
        last_depth = result.depth;
    }
}

#[test]
fn test_frontier_parallel_bfs_with_limit() {
    let bfs = FrontierParallelBFS::with_config(
        ParallelConfig::new()
            .with_max_depth(10)
            .with_parallel_threshold(1)
            .with_limit(3),
    );

    let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
        if node < 100 {
            vec![(node + 1, node * 10), (node + 2, node * 10 + 1)]
        } else {
            vec![]
        }
    };

    let (results, _) = bfs.traverse(1, get_neighbors);

    assert!(results.len() <= 3);
}

#[test]
fn test_frontier_parallel_bfs_empty_graph() {
    let bfs = FrontierParallelBFS::new();

    let get_neighbors = |_node: u64| -> Vec<(u64, u64)> { vec![] };

    let (results, stats) = bfs.traverse(1, get_neighbors);

    // Only start node should be in results
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].end_node, 1);
    assert_eq!(stats.start_nodes_count, 1);
}

// =============================================================================
// US-006: Configuration & Auto-Tuning Tests
// =============================================================================

#[test]
fn test_thread_config_auto() {
    let config = ThreadConfig::Auto;
    let threads = config.effective_threads();
    // Should be at least 1 thread
    assert!(threads >= 1);
    // Should be less than or equal to CPU count
    let cpu_count = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    assert!(threads <= cpu_count);
}

#[test]
fn test_thread_config_fixed() {
    let config = ThreadConfig::Fixed(4);
    assert_eq!(config.effective_threads(), 4);
}

#[test]
fn test_parallel_config_builder() {
    let config = ParallelConfig::new()
        .with_max_depth(10)
        .with_parallel_threshold(50)
        .with_min_frontier(25)
        .with_limit(500)
        .with_fixed_threads(8);

    assert_eq!(config.max_depth, 10);
    assert_eq!(config.parallel_threshold, 50);
    assert_eq!(config.min_frontier_for_parallel, 25);
    assert_eq!(config.limit, 500);
    assert_eq!(config.threads, ThreadConfig::Fixed(8));
}

#[test]
fn test_should_parallelize() {
    let config = ParallelConfig::new().with_parallel_threshold(100);

    assert!(!config.should_parallelize(50));
    assert!(!config.should_parallelize(99));
    assert!(config.should_parallelize(100));
    assert!(config.should_parallelize(200));
}

#[test]
fn test_should_parallelize_frontier() {
    let config = ParallelConfig::new().with_min_frontier(50);

    assert!(!config.should_parallelize_frontier(25));
    assert!(!config.should_parallelize_frontier(49));
    assert!(config.should_parallelize_frontier(50));
    assert!(config.should_parallelize_frontier(100));
}

#[test]
fn test_effective_threads_from_config() {
    let config_auto = ParallelConfig::new();
    assert!(config_auto.effective_threads() >= 1);

    let config_fixed = ParallelConfig::new().with_fixed_threads(16);
    assert_eq!(config_fixed.effective_threads(), 16);
}

// =============================================================================
// US-003: Shard-Parallel Traversal Tests
// =============================================================================

#[test]
fn test_sharded_traverser_shard_assignment() {
    let traverser = ShardedTraverser::new(4);

    // Nodes should be evenly distributed across shards
    assert_eq!(traverser.shard_for_node(0), 0);
    assert_eq!(traverser.shard_for_node(1), 1);
    assert_eq!(traverser.shard_for_node(2), 2);
    assert_eq!(traverser.shard_for_node(3), 3);
    assert_eq!(traverser.shard_for_node(4), 0); // Wraps around
    assert_eq!(traverser.shard_for_node(100), 0);
}

#[test]
fn test_sharded_traverser_partition() {
    let traverser = ShardedTraverser::new(4);

    let nodes = vec![0, 1, 2, 3, 4, 5, 6, 7, 8];
    let partitions = traverser.partition_by_shard(&nodes);

    assert_eq!(partitions.len(), 4);
    assert_eq!(partitions[0], vec![0, 4, 8]); // Shard 0
    assert_eq!(partitions[1], vec![1, 5]); // Shard 1
    assert_eq!(partitions[2], vec![2, 6]); // Shard 2
    assert_eq!(partitions[3], vec![3, 7]); // Shard 3
}

#[test]
fn test_sharded_traverser_basic() {
    let graph = create_test_graph();
    let traverser = ShardedTraverser::with_config(
        2,
        ParallelConfig::new()
            .with_max_depth(3)
            .with_parallel_threshold(1),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, stats) = traverser.traverse_parallel(&[1], get_neighbors);

    // Should visit all reachable nodes
    assert!(!results.is_empty());
    assert_eq!(stats.start_nodes_count, 1);
}

#[test]
fn test_sharded_traverser_cross_shard_edges() {
    // Graph where edges cross shard boundaries
    let mut graph: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
    // Node 0 (shard 0) -> Node 1 (shard 1)
    graph.insert(0, vec![(1, 100)]);
    // Node 1 (shard 1) -> Node 2 (shard 0)
    graph.insert(1, vec![(2, 101)]);
    graph.insert(2, vec![]);

    let traverser = ShardedTraverser::with_config(
        2, // 2 shards
        ParallelConfig::new()
            .with_max_depth(5)
            .with_parallel_threshold(1),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, _) = traverser.traverse_parallel(&[0], get_neighbors);

    // Should follow cross-shard edges: 0 -> 1 -> 2
    assert!(results.len() >= 3);
    let end_nodes: std::collections::HashSet<u64> = results.iter().map(|r| r.end_node).collect();
    assert!(end_nodes.contains(&0));
    assert!(end_nodes.contains(&1));
    assert!(end_nodes.contains(&2));
}

#[test]
fn test_sharded_traverser_multiple_start_nodes() {
    let graph = create_test_graph();
    let traverser = ShardedTraverser::with_config(
        4,
        ParallelConfig::new()
            .with_max_depth(2)
            .with_parallel_threshold(1),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    // Start from multiple nodes in different shards
    let (results, stats) = traverser.traverse_parallel(&[1, 2, 3], get_neighbors);

    assert_eq!(stats.start_nodes_count, 3);
    assert!(results.len() >= 3);
}

#[test]
fn test_sharded_traverser_num_shards() {
    let traverser = ShardedTraverser::new(8);
    assert_eq!(traverser.num_shards(), 8);
}

// --- M-03 Regression Test: DFS stops immediately at limit ---

#[test]
fn test_dfs_stops_at_limit() {
    // Deep linear graph: 1→2→3→4→5→6→7→8→9→10
    let mut graph: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
    for i in 1..=9 {
        graph.insert(i, vec![(i + 1, i * 10)]);
    }
    graph.insert(10, vec![]);

    let traverser = ParallelTraverser::with_config(
        ParallelConfig::new()
            .with_max_depth(20)
            .with_parallel_threshold(1000) // force sequential
            .with_limit(4),
    );

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, _) = traverser.dfs_parallel(&[1], get_neighbors);

    // DFS should stop at exactly 4 results (start node + 3 neighbors)
    assert_eq!(
        results.len(),
        4,
        "DFS should produce exactly 4 results with limit=4, got {}. \
         M-03 regression: break instead of continue when limit reached.",
        results.len()
    );
}
