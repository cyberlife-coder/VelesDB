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
    let traverser = ParallelTraverser::with_config(ParallelConfig {
        max_depth: 3,
        parallel_threshold: 1,
        limit: 100,
        relationship_types: vec![],
    });

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
    let traverser = ParallelTraverser::with_config(ParallelConfig {
        max_depth: 2,
        parallel_threshold: 1,
        limit: 100,
        relationship_types: vec![],
    });

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, stats) = traverser.bfs_parallel(&[1, 3], get_neighbors);

    assert_eq!(stats.start_nodes_count, 2);
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

    let get_neighbors =
        |node: u64| -> Vec<(u64, u64)> { graph.get(&node).cloned().unwrap_or_default() };

    let (results, _) = traverser.bfs_parallel(&[1], get_neighbors);

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
    let traverser = ParallelTraverser::with_config(ParallelConfig {
        max_depth: 10,
        parallel_threshold: 1,
        limit: 3,
        relationship_types: vec![],
    });

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
