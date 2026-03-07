//! Tests for plan cache types (CACHE-01).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use smallvec::smallvec;

use super::plan_cache::{CompiledPlan, CompiledPlanCache, PlanCacheMetrics, PlanKey};
use crate::velesql::QueryPlan;

/// Helper: build a minimal `QueryPlan` for testing.
fn dummy_query_plan() -> QueryPlan {
    use crate::velesql::{FilterStrategy, PlanNode, TableScanPlan};

    QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "test".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
    }
}

/// Helper: build a `CompiledPlan` wrapped in `Arc`.
fn dummy_compiled_plan() -> Arc<CompiledPlan> {
    Arc::new(CompiledPlan {
        plan: dummy_query_plan(),
        referenced_collections: vec!["test".to_string()],
        compiled_at: std::time::Instant::now(),
        reuse_count: std::sync::atomic::AtomicU64::new(0),
    })
}

// ---- PlanKey hash determinism ----

#[test]
fn plan_key_same_fields_same_hash() {
    use std::hash::{Hash, Hasher};

    let a = PlanKey {
        query_hash: 42,
        schema_version: 1,
        collection_generations: smallvec![10, 20],
    };
    let b = PlanKey {
        query_hash: 42,
        schema_version: 1,
        collection_generations: smallvec![10, 20],
    };
    assert_eq!(a, b);

    let hash_of = |key: &PlanKey| -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash_of(&a), hash_of(&b));
}

#[test]
fn plan_key_different_generations_different_hash() {
    use std::hash::{Hash, Hasher};

    let a = PlanKey {
        query_hash: 42,
        schema_version: 1,
        collection_generations: smallvec![10, 20],
    };
    let b = PlanKey {
        query_hash: 42,
        schema_version: 1,
        collection_generations: smallvec![10, 21],
    };
    assert_ne!(a, b);

    let hash_of = |key: &PlanKey| -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    };
    assert_ne!(hash_of(&a), hash_of(&b));
}

// ---- CompiledPlanCache insert + get ----

#[test]
fn plan_cache_insert_and_get() {
    let cache = CompiledPlanCache::new(100, 1_000);
    let key = PlanKey {
        query_hash: 1,
        schema_version: 0,
        collection_generations: smallvec![0],
    };
    let plan = dummy_compiled_plan();

    cache.insert(key.clone(), Arc::clone(&plan));
    let got = cache.get(&key);
    assert!(got.is_some(), "cached plan should be returned");
    assert_eq!(got.unwrap().plan, plan.plan);
}

#[test]
fn plan_cache_miss_on_different_key() {
    let cache = CompiledPlanCache::new(100, 1_000);
    let key = PlanKey {
        query_hash: 1,
        schema_version: 0,
        collection_generations: smallvec![0],
    };
    cache.insert(key, dummy_compiled_plan());

    let other = PlanKey {
        query_hash: 2,
        schema_version: 0,
        collection_generations: smallvec![0],
    };
    assert!(cache.get(&other).is_none(), "different key should miss");
}

// ---- PlanCacheMetrics ----

#[test]
fn plan_cache_metrics_hit_miss() {
    let metrics = PlanCacheMetrics::default();
    assert_eq!(metrics.hits(), 0);
    assert_eq!(metrics.misses(), 0);

    metrics.record_hit();
    metrics.record_hit();
    metrics.record_miss();

    assert_eq!(metrics.hits(), 2);
    assert_eq!(metrics.misses(), 1);
    // 2 / 3 ~= 0.666...
    let rate = metrics.hit_rate();
    assert!((rate - 2.0 / 3.0).abs() < 1e-9);
}

// ---- CompiledPlan is Send + Sync ----

#[test]
fn plan_cache_compiled_plan_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Arc<CompiledPlan>>();
    assert_send_sync::<CompiledPlanCache>();
}

// ---- Reuse count increments on get ----

#[test]
fn plan_cache_reuse_count_increments() {
    let cache = CompiledPlanCache::new(100, 1_000);
    let key = PlanKey {
        query_hash: 99,
        schema_version: 0,
        collection_generations: smallvec![0],
    };
    let plan = dummy_compiled_plan();
    assert_eq!(plan.reuse_count.load(Ordering::Relaxed), 0);

    cache.insert(key.clone(), Arc::clone(&plan));

    let _ = cache.get(&key);
    let _ = cache.get(&key);
    // The reuse_count on the *original* Arc should reflect two reuses
    assert_eq!(plan.reuse_count.load(Ordering::Relaxed), 2);
}
