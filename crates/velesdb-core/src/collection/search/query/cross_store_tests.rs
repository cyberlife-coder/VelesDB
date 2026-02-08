//! Tests for cross-store query planner integration (VP-010, Plan 06-03).
//!
//! Tests verify that the QueryPlanner selects correct strategies
//! and that hybrid.rs fusion functions are actively wired.

use crate::velesql::{
    fuse_maximum, fuse_rrf, fuse_weighted, intersect_results, normalize_scores, ExecutionStrategy,
    HybridExecutionPlan, QueryPlanner, RrfConfig, ScoredResult, WeightedConfig,
};

// ============================================================================
// QueryPlanner strategy selection tests
// ============================================================================

#[test]
fn test_planner_selects_vector_first_for_dense_graph() {
    let planner = QueryPlanner::new();
    // High selectivity (80% of data matches) → VectorFirst
    let strategy = planner.choose_strategy(Some(0.80));
    assert_eq!(strategy, ExecutionStrategy::VectorFirst);
}

#[test]
fn test_planner_selects_graph_first_for_selective_labels() {
    let planner = QueryPlanner::new();
    // Low selectivity (<1% of data matches) → GraphFirst
    let strategy = planner.choose_strategy(Some(0.005));
    assert_eq!(strategy, ExecutionStrategy::GraphFirst);
}

#[test]
fn test_planner_selects_parallel_for_medium_selectivity() {
    let planner = QueryPlanner::new();
    // Medium selectivity (10% of data) → Parallel
    let strategy = planner.choose_strategy(Some(0.10));
    assert_eq!(strategy, ExecutionStrategy::Parallel);
}

#[test]
fn test_planner_hybrid_strategy_with_order_by_similarity() {
    let planner = QueryPlanner::new();
    let plan = planner.choose_hybrid_strategy(true, true, Some(10), Some(0.3));
    // ORDER BY similarity() always chooses VectorFirst
    assert_eq!(plan.strategy, ExecutionStrategy::VectorFirst);
    assert!(
        plan.over_fetch_factor > 1.0,
        "Should over-fetch when filtering"
    );
}

#[test]
fn test_planner_hybrid_strategy_no_order_by() {
    let planner = QueryPlanner::new();
    let plan = planner.choose_hybrid_strategy(false, true, Some(10), Some(0.005));
    // Low selectivity without ORDER BY → GraphFirst
    assert_eq!(plan.strategy, ExecutionStrategy::GraphFirst);
}

#[test]
fn test_planner_estimate_selectivity() {
    let planner = QueryPlanner::new();

    // 100 "Person" labels out of 10000 nodes, 50 "KNOWS" out of 5000 edges
    let sel = planner.estimate_selectivity(100, 10_000, 50, 5000);
    // (100/10000) * (50/5000) = 0.01 * 0.01 = 0.0001
    assert!(
        (sel - 0.0001).abs() < 0.0001,
        "Selectivity should be ~0.0001, got {sel}"
    );
}

#[test]
fn test_planner_estimate_cost() {
    let planner = QueryPlanner::new();
    // Update stats with some latencies
    planner.stats().update_vector_latency(100);
    planner.stats().update_graph_latency(200);

    let plan = HybridExecutionPlan::default();
    let cost = planner.estimate_cost(&plan, 1000);
    assert!(cost > 0, "Cost should be positive");
}

#[test]
fn test_planner_stats_update() {
    let planner = QueryPlanner::new();

    planner.stats().update_vector_latency(100);
    planner.stats().update_vector_latency(200);
    assert_eq!(planner.stats().vector_query_count(), 2);
    assert!(planner.stats().avg_vector_latency_us() > 0);

    planner.stats().update_graph_latency(300);
    assert_eq!(planner.stats().graph_query_count(), 1);

    planner.stats().update_graph_selectivity(50, 1000);
    let sel = planner.stats().graph_selectivity();
    assert!(
        (sel - 0.05).abs() < 0.001,
        "Selectivity should be ~0.05, got {sel}"
    );
}

// ============================================================================
// hybrid.rs fusion function tests (wiring validation)
// ============================================================================

#[test]
fn test_cross_store_rrf_fusion() {
    let vector_results = vec![
        ScoredResult::new(1, 0.95),
        ScoredResult::new(2, 0.85),
        ScoredResult::new(3, 0.75),
    ];
    let graph_results = vec![
        ScoredResult::new(2, 0.90),
        ScoredResult::new(4, 0.80),
        ScoredResult::new(1, 0.70),
    ];

    let fused = fuse_rrf(&vector_results, &graph_results, &RrfConfig::default(), 5);
    assert!(!fused.is_empty(), "RRF fusion should return results");

    // IDs 1 and 2 appear in both → should have highest RRF scores
    let top_ids: Vec<u64> = fused.iter().take(2).map(|r| r.id).collect();
    assert!(
        top_ids.contains(&1) && top_ids.contains(&2),
        "IDs in both sources should rank highest: {:?}",
        top_ids
    );
}

#[test]
fn test_cross_store_weighted_fusion() {
    let vector_results = vec![ScoredResult::new(1, 0.95), ScoredResult::new(2, 0.60)];
    let graph_results = vec![ScoredResult::new(2, 0.90), ScoredResult::new(3, 0.80)];

    let config = WeightedConfig::new(0.7, 0.3);
    let fused = fuse_weighted(&vector_results, &graph_results, &config, 5);
    assert!(!fused.is_empty(), "Weighted fusion should return results");
}

#[test]
fn test_cross_store_maximum_fusion() {
    let vector_results = vec![ScoredResult::new(1, 0.5), ScoredResult::new(2, 0.3)];
    let graph_results = vec![ScoredResult::new(1, 0.8), ScoredResult::new(3, 0.9)];

    let fused = fuse_maximum(&vector_results, &graph_results, 5);
    assert!(!fused.is_empty(), "Maximum fusion should return results");
}

#[test]
fn test_cross_store_intersect_results() {
    let vector_results = vec![
        ScoredResult::new(1, 0.9),
        ScoredResult::new(2, 0.8),
        ScoredResult::new(3, 0.7),
    ];
    let graph_results = vec![ScoredResult::new(2, 0.85), ScoredResult::new(4, 0.75)];

    let (filtered_vector, filtered_graph) = intersect_results(&vector_results, &graph_results);

    // Only ID=2 is in both
    assert_eq!(
        filtered_vector.len(),
        1,
        "Only shared IDs in vector results"
    );
    assert_eq!(filtered_vector[0].id, 2);
    assert_eq!(filtered_graph.len(), 1, "Only shared IDs in graph results");
    assert_eq!(filtered_graph[0].id, 2);
}

#[test]
fn test_normalize_scores_basic() {
    let results = vec![
        ScoredResult::new(1, 10.0),
        ScoredResult::new(2, 5.0),
        ScoredResult::new(3, 0.0),
    ];
    let normalized = normalize_scores(&results);

    assert_eq!(normalized.len(), 3);
    assert!(
        (normalized[0].score - 1.0).abs() < 0.01,
        "Max should normalize to 1.0"
    );
    assert!(
        (normalized[2].score - 0.0).abs() < 0.01,
        "Min should normalize to 0.0"
    );
}

#[test]
fn test_normalize_scores_empty() {
    let normalized = normalize_scores(&[]);
    assert!(normalized.is_empty());
}

#[test]
fn test_normalize_scores_equal() {
    let results = vec![ScoredResult::new(1, 5.0), ScoredResult::new(2, 5.0)];
    let normalized = normalize_scores(&results);
    // All same → all normalize to 1.0
    for r in &normalized {
        assert!((r.score - 1.0).abs() < 0.01);
    }
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_rrf_fusion_empty_sources() {
    let fused = fuse_rrf(&[], &[], &RrfConfig::default(), 10);
    assert!(fused.is_empty(), "Empty sources → empty results");
}

#[test]
fn test_rrf_fusion_one_empty_source() {
    let vector_results = vec![ScoredResult::new(1, 0.9)];
    let fused = fuse_rrf(&vector_results, &[], &RrfConfig::default(), 10);
    assert_eq!(
        fused.len(),
        1,
        "Should still return results from non-empty source"
    );
    assert_eq!(fused[0].id, 1);
}

#[test]
fn test_planner_custom_thresholds() {
    let planner = QueryPlanner::with_thresholds(0.05, 0.80);
    // 3% selectivity with threshold 5% → GraphFirst
    assert_eq!(
        planner.choose_strategy(Some(0.03)),
        ExecutionStrategy::GraphFirst
    );
    // 90% selectivity → VectorFirst
    assert_eq!(
        planner.choose_strategy(Some(0.90)),
        ExecutionStrategy::VectorFirst
    );
    // 50% → Parallel (between thresholds)
    assert_eq!(
        planner.choose_strategy(Some(0.50)),
        ExecutionStrategy::Parallel
    );
}
