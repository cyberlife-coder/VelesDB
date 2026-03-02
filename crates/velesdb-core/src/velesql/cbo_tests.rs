use super::ast::{CompareOp, Comparison, Condition, Value};
use super::planner::{ExecutionStrategy, QueryPlanner};
use crate::collection::stats::{CollectionStats, ColumnStats};

#[test]
fn test_cbo_prefers_graph_first_for_selective_filter() {
    let planner = QueryPlanner::new();
    let mut stats = CollectionStats::new();
    stats.total_points = 100_000;
    stats.field_stats.insert(
        "tenant_id".to_string(),
        ColumnStats::new("tenant_id").with_distinct_count(50_000),
    );

    let filter = Condition::Comparison(Comparison {
        column: "tenant_id".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(42),
    });

    let strategy = planner.choose_strategy_with_cbo(&stats, Some(&filter), 20);
    assert_eq!(strategy, ExecutionStrategy::GraphFirst);
}

#[test]
fn test_cbo_prefers_vector_first_for_non_selective_filter() {
    let planner = QueryPlanner::new();
    let mut stats = CollectionStats::new();
    stats.total_points = 100_000;
    stats.field_stats.insert(
        "status".to_string(),
        ColumnStats::new("status").with_distinct_count(2),
    );

    let filter = Condition::Comparison(Comparison {
        column: "status".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("active".to_string()),
    });

    let strategy = planner.choose_strategy_with_cbo(&stats, Some(&filter), 20);
    assert_eq!(strategy, ExecutionStrategy::VectorFirst);
}

#[test]
fn test_cbo_vector_first_cost_scales_with_selectivity() {
    let planner = QueryPlanner::new();
    let mut stats = CollectionStats::new();
    stats.total_points = 100_000;
    stats.field_stats.insert(
        "tenant_id".to_string(),
        ColumnStats::new("tenant_id").with_distinct_count(100_000),
    );

    let selective_filter = Condition::Comparison(Comparison {
        column: "tenant_id".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(7),
    });

    // With an extremely selective predicate and large k, graph-first should remain preferred.
    let strategy = planner.choose_strategy_with_cbo(&stats, Some(&selective_filter), 500);
    assert_eq!(strategy, ExecutionStrategy::GraphFirst);
}

#[test]
fn test_cbo_without_filter_always_uses_vector_first() {
    let planner = QueryPlanner::new();
    let mut stats = CollectionStats::new();
    stats.total_points = 100_000;

    let strategy = planner.choose_strategy_with_cbo(&stats, None, 20);
    assert_eq!(strategy, ExecutionStrategy::VectorFirst);
}
