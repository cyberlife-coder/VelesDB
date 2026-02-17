//! Tests for `planner` module - Query execution strategy planning.

use super::planner::*;

#[test]
fn test_choose_strategy_graph_first() {
    let planner = QueryPlanner::new();
    assert_eq!(
        planner.choose_strategy(Some(0.005)),
        ExecutionStrategy::GraphFirst
    );
}

#[test]
fn test_choose_strategy_vector_first() {
    let planner = QueryPlanner::new();
    assert_eq!(
        planner.choose_strategy(Some(0.8)),
        ExecutionStrategy::VectorFirst
    );
}

#[test]
fn test_choose_strategy_parallel() {
    let planner = QueryPlanner::new();
    assert_eq!(
        planner.choose_strategy(Some(0.2)),
        ExecutionStrategy::Parallel
    );
}

#[test]
fn test_estimate_selectivity() {
    let planner = QueryPlanner::new();
    let sel = planner.estimate_selectivity(10, 1000, 5, 100);
    assert!((sel - 0.0005).abs() < 0.0001);
}

#[test]
fn test_query_stats_update() {
    let stats = QueryStats::new();

    stats.update_graph_selectivity(10, 1000);
    assert!((stats.graph_selectivity() - 0.01).abs() < 0.0001);

    stats.update_vector_latency(100);
    stats.update_vector_latency(200);
    assert_eq!(stats.avg_vector_latency_us(), 110);
    assert_eq!(stats.vector_query_count(), 2);
}

#[test]
fn test_graph_latency_independent_count() {
    let stats = QueryStats::new();

    stats.update_vector_latency(100);
    stats.update_vector_latency(200);
    stats.update_vector_latency(300);

    stats.update_graph_latency(50);
    stats.update_graph_latency(150);

    assert_eq!(stats.avg_graph_latency_us(), 60);
    assert_eq!(stats.graph_query_count(), 2);
    assert_eq!(stats.vector_query_count(), 3);
}

#[test]
fn test_custom_thresholds() {
    let planner = QueryPlanner::with_thresholds(0.05, 0.30);

    assert_eq!(
        planner.choose_strategy(Some(0.03)),
        ExecutionStrategy::GraphFirst
    );
    assert_eq!(
        planner.choose_strategy(Some(0.15)),
        ExecutionStrategy::Parallel
    );
    assert_eq!(
        planner.choose_strategy(Some(0.40)),
        ExecutionStrategy::VectorFirst
    );
}

#[test]
fn test_estimate_selectivity_missing_rel_type_returns_zero() {
    let planner = QueryPlanner::new();
    let sel = planner.estimate_selectivity(10, 100, 0, 50);

    assert!(
        sel < 0.01,
        "Missing relationship type should give selectivity ~0.0, got {sel}"
    );
}

// =========================================================================
// Hybrid Query Planner Tests (US-009)
// =========================================================================

#[test]
fn test_hybrid_strategy_order_by_similarity_uses_vector_first() {
    let planner = QueryPlanner::new();
    let plan = planner.choose_hybrid_strategy(true, false, Some(10), None);

    assert_eq!(plan.strategy, ExecutionStrategy::VectorFirst);
    assert!(!plan.recompute_scores);
}

#[test]
fn test_hybrid_strategy_order_by_similarity_with_filter_over_fetches() {
    let planner = QueryPlanner::new();
    let plan = planner.choose_hybrid_strategy(true, true, Some(10), Some(0.5));

    assert_eq!(plan.strategy, ExecutionStrategy::VectorFirst);
    assert!(plan.over_fetch_factor >= 2.0);
}

#[test]
fn test_hybrid_strategy_low_selectivity_over_fetches_more() {
    let planner = QueryPlanner::new();
    let plan = planner.choose_hybrid_strategy(true, true, Some(10), Some(0.1));

    assert!((plan.over_fetch_factor - 10.0).abs() < 0.01);
}

#[test]
fn test_hybrid_strategy_no_order_by_uses_standard_planning() {
    let planner = QueryPlanner::new();
    let plan = planner.choose_hybrid_strategy(false, true, Some(10), Some(0.005));

    assert_eq!(plan.strategy, ExecutionStrategy::GraphFirst);
    assert!(plan.recompute_scores);
}

#[test]
fn test_hybrid_plan_default() {
    let plan = HybridExecutionPlan::default();

    assert_eq!(plan.strategy, ExecutionStrategy::VectorFirst);
    assert!((plan.over_fetch_factor - 1.0).abs() < 0.01);
    assert!(plan.use_early_termination);
    assert!(!plan.recompute_scores);
}

#[test]
fn test_estimate_cost_vector_first() {
    let planner = QueryPlanner::new();
    planner.stats().update_vector_latency(100);

    let plan = HybridExecutionPlan {
        strategy: ExecutionStrategy::VectorFirst,
        over_fetch_factor: 1.0,
        use_early_termination: true,
        recompute_scores: false,
    };

    let cost = planner.estimate_cost(&plan, 100);
    assert!(cost > 0);
}
