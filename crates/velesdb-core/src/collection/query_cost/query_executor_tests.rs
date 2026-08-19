use super::*;
use crate::collection::stats::IndexStats;

fn test_stats() -> CollectionStats {
    let mut stats = CollectionStats::with_counts(100_000, 0);
    stats.total_size_bytes = 100_000 * 256;
    stats.index_stats.insert(
        "hnsw_primary".to_string(),
        IndexStats::new("hnsw_primary", "HNSW").with_entry_count(100_000),
    );
    stats
}

#[test]
fn test_plan_cache_basic() {
    let cache = PlanCache::new(10);
    assert!(cache.is_empty());

    let plan = CandidatePlan::new(
        super::super::plan_generator::PhysicalPlan::SeqScan {
            collection: "test".to_string(),
            estimated_rows: 100,
        },
        super::super::cost_model::OperationCost::new(0.0, 10.0, 100),
        "Test plan",
    );

    cache.insert(123, plan.clone());
    assert_eq!(cache.len(), 1);

    let cached = cache.get(123);
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().cost.rows, 100);
}

#[test]
fn test_plan_cache_eviction() {
    let cache = PlanCache::new(2);

    for i in 0..5 {
        let plan = CandidatePlan::new(
            super::super::plan_generator::PhysicalPlan::SeqScan {
                collection: format!("test_{i}"),
                estimated_rows: i as u64,
            },
            super::super::cost_model::OperationCost::new(0.0, 10.0, i as u64),
            format!("Plan {i}"),
        );
        cache.insert(i, plan);
    }

    // Should only have 2 entries
    assert_eq!(cache.len(), 2);
}

#[test]
fn test_optimizer_caching() {
    let optimizer = QueryOptimizer::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_similarity: true,
        top_k: Some(10),
        ..Default::default()
    };

    // First call - generates plan
    let plan1 = optimizer.optimize(&query, &stats);
    assert!(plan1.is_some());
    assert_eq!(optimizer.cache_size(), 1);

    // Second call - uses cache
    let plan2 = optimizer.optimize(&query, &stats);
    assert!(plan2.is_some());
    assert_eq!(optimizer.cache_size(), 1); // Still 1

    // Plans should be equivalent
    assert_eq!(plan1.unwrap().cost.rows, plan2.unwrap().cost.rows);
}

#[test]
fn test_cache_invalidation() {
    let optimizer = QueryOptimizer::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "users".to_string(),
        ..Default::default()
    };

    let _ = optimizer.optimize(&query, &stats);
    assert_eq!(optimizer.cache_size(), 1);

    optimizer.invalidate("users");
    assert_eq!(optimizer.cache_size(), 0);
}

#[test]
fn test_execution_context_explain() {
    let ctx = ExecutionContext::new();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_similarity: true,
        has_match: true,
        top_k: Some(10),
        max_depth: Some(2),
        ..Default::default()
    };

    let explain = ctx.explain(&query, &stats);

    assert!(explain.contains("Query Plan Analysis"));
    assert!(explain.contains("Selected:"));
}

#[test]
fn test_cache_key_stability() {
    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_similarity: true,
        top_k: Some(10),
        ..Default::default()
    };

    let key1 = QueryOptimizer::compute_cache_key(&query);
    let key2 = QueryOptimizer::compute_cache_key(&query);

    assert_eq!(key1, key2);
}

#[test]
fn test_invalidate_nested_filter_plans() {
    // Regression test for PR #152: nested Filter/Limit plans must be invalidated
    use crate::collection::query_cost::plan_generator::PhysicalPlan;

    // Test that plan_references_collection correctly identifies nested plans
    let nested_plan = PhysicalPlan::Filter {
        input: Box::new(PhysicalPlan::VectorSearch {
            collection: "docs".to_string(),
            k: 10,
            ef_search: 100,
        }),
        selectivity: 0.5,
    };

    assert!(
        PlanCache::plan_references_collection(&nested_plan, "docs"),
        "Should find collection in nested Filter plan"
    );

    let double_nested = PhysicalPlan::Limit {
        input: Box::new(nested_plan),
        limit: 5,
        offset: 0,
    };

    assert!(
        PlanCache::plan_references_collection(&double_nested, "docs"),
        "Should find collection in double-nested Limit->Filter plan"
    );

    // Ensure it returns false for different collection
    assert!(
        !PlanCache::plan_references_collection(&double_nested, "other"),
        "Should NOT find unrelated collection"
    );
}
