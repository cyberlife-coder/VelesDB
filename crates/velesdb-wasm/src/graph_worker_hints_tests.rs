use super::*;

#[test]
fn test_default_config() {
    let config = GraphWorkerConfig::default();
    assert_eq!(config.node_threshold, 5_000);
    assert_eq!(config.depth_threshold, 4);
    assert_eq!(config.progress_interval_ms, 100);
}

#[test]
fn test_should_use_worker_by_nodes() {
    let config = GraphWorkerConfig {
        node_threshold: 1000,
        depth_threshold: 10,
        ..Default::default()
    };

    assert!(!should_use_worker(500, 2, Some(config.clone())));
    assert!(should_use_worker(1500, 2, Some(config)));
}

#[test]
fn test_should_use_worker_by_depth() {
    let config = GraphWorkerConfig {
        node_threshold: 10_000,
        depth_threshold: 5,
        ..Default::default()
    };

    assert!(!should_use_worker(100, 3, Some(config.clone())));
    assert!(should_use_worker(100, 6, Some(config)));
}

#[test]
fn test_progress_percentage() {
    let progress = TraversalProgress::new(50, 100, 2);
    assert!((progress.percentage() - 50.0).abs() < 0.01);

    let progress_zero = TraversalProgress::new(0, 0, 0);
    assert!((progress_zero.percentage() - 0.0).abs() < 0.01);
}

#[test]
fn test_estimate_traversal_size() {
    // Empty graph
    assert_eq!(estimate_traversal_size(0, 0, 5), 0);

    // Single node
    assert_eq!(estimate_traversal_size(1, 0, 5), 1);

    // Small graph with depth 1
    let estimate = estimate_traversal_size(100, 200, 1);
    assert!(estimate > 0 && estimate <= 100);

    // Larger depth
    let estimate_deep = estimate_traversal_size(1000, 3000, 5);
    assert!(estimate_deep <= 1000);
}
