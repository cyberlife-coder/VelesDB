#![allow(deprecated)] // Tests use legacy Collection.
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
        cache_hit: None,
        plan_reuse_count: None,
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

// ---- PlanKey structural equality ----

/// Two `PlanKey` values with identical fields must compare equal.
///
/// `DefaultHasher` is intentionally avoided here: its output is not stable
/// across Rust versions, so asserting `hash_of(a) == hash_of(b)` would be a
/// tautology for equal values anyway (required by the `Hash` contract) and
/// is not meaningful as a regression test. Structural equality via `PartialEq`
/// is what the cache actually uses for key lookup.
#[test]
fn plan_key_equal_fields_are_equal() {
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
}

/// Two `PlanKey` values that differ in `collection_generations` must compare unequal.
///
/// This verifies that a write to a collection (which advances `write_generation`)
/// produces a different key and therefore results in a cache miss, which is the
/// core cache invalidation invariant (CACHE-01).
#[test]
fn plan_key_different_generations_are_not_equal() {
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

// ---- write_generation on Collection ----

#[cfg(feature = "persistence")]
#[test]
fn write_generation_starts_at_zero_and_increments() {
    let dir = tempfile::tempdir().unwrap();
    let coll =
        crate::Collection::create(dir.path().to_path_buf(), 4, crate::DistanceMetric::Cosine)
            .unwrap();

    assert_eq!(coll.write_generation(), 0, "should start at 0");

    // Upsert a point to bump write_generation.
    coll.upsert(vec![crate::Point {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    assert_eq!(coll.write_generation(), 1, "should be 1 after upsert");

    // Delete to bump again.
    coll.delete(&[1]).unwrap();
    assert_eq!(coll.write_generation(), 2, "should be 2 after delete");
}

// ---- schema_version on Database ----

#[cfg(feature = "persistence")]
#[test]
fn schema_version_increments_on_ddl() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    assert_eq!(db.schema_version(), 0, "should start at 0");

    db.create_collection("test_sv", 4, crate::DistanceMetric::Cosine)
        .unwrap();
    assert_eq!(db.schema_version(), 1, "should be 1 after create");

    db.delete_collection("test_sv").unwrap();
    assert_eq!(db.schema_version(), 2, "should be 2 after delete");
}

// ---- collection_write_generation on Database ----

#[cfg(feature = "persistence")]
#[test]
fn write_generation_accessible_from_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    db.create_collection("wg_test", 4, crate::DistanceMetric::Cosine)
        .unwrap();
    assert_eq!(
        db.collection_write_generation("wg_test"),
        Some(0),
        "new collection starts at 0"
    );

    let coll = db.get_collection("wg_test").unwrap();
    coll.upsert(vec![crate::Point {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    assert_eq!(
        db.collection_write_generation("wg_test"),
        Some(1),
        "should reflect upsert"
    );
    assert_eq!(
        db.collection_write_generation("nonexistent"),
        None,
        "missing collection returns None"
    );
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

// ---- Integration tests: Database execute_query cache wiring (CACHE-02) ----

/// Helper: build a simple SELECT query for a given collection.
#[cfg(feature = "persistence")]
fn select_query(collection: &str) -> crate::velesql::Query {
    use crate::velesql::{Condition, SelectColumns, SelectStatement, VectorExpr, VectorSearch};

    crate::velesql::Query {
        select: SelectStatement {
            distinct: crate::velesql::DistinctMode::default(),
            columns: SelectColumns::All,
            from: collection.to_string(),
            from_alias: Vec::new(),
            where_clause: Some(Condition::VectorSearch(VectorSearch {
                vector: VectorExpr::Literal(vec![1.0, 0.0, 0.0, 0.0]),
            })),
            limit: Some(5),
            offset: None,
            order_by: None,
            joins: Vec::new(),
            group_by: None,
            having: None,
            with_clause: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: None,
        dml: None,
        train: None,
    }
}

#[cfg(feature = "persistence")]
#[test]
fn test_plan_cache_hit() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    db.create_collection("cache_hit", 4, crate::DistanceMetric::Cosine)
        .unwrap();

    // Insert some data so the query has results.
    let coll = db.get_collection("cache_hit").unwrap();
    coll.upsert(vec![crate::Point {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    let query = select_query("cache_hit");
    let params = std::collections::HashMap::new();

    // Exact execution flow (important for understanding reuse_count):
    //
    // 1. execute_query #1: calls contains() -> false (no metric recorded).
    //    Executes the query, compiles plan, inserts into cache (reuse_count=0).
    // 2. explain_query #1: calls get() -> HIT, increments reuse_count to 1,
    //    returns cache_hit=true and plan_reuse_count=Some(1).
    // 3. execute_query #2: calls contains() -> true (no metric recorded).
    //    Executes the query, skips cache insert (is_cached=true).
    //    reuse_count remains 1.
    // 4. explain_query #2: calls get() -> HIT, increments reuse_count to 2,
    //    returns cache_hit=true and plan_reuse_count=Some(2).
    //
    // NOTE: execute_query uses contains() (no metric side-effect) rather than
    // get() (which would increment reuse_count). Only explain_query calls get(),
    // keeping cache metrics accurate and reuse_count meaningful.

    // Step 1: first execute_query — cache miss, populates cache.
    let _ = db.execute_query(&query, &params).unwrap();
    // Step 2: explain_query — hits the cache via get(), reuse_count -> 1.
    let explain1 = db.explain_query(&query).unwrap();
    assert_eq!(
        explain1.cache_hit,
        Some(true),
        "explain after first execute should be cache hit"
    );

    // Step 3: second execute_query — cache hit via contains(), reuse_count unchanged.
    let _ = db.execute_query(&query, &params).unwrap();
    // Step 4: explain_query — hits the cache via get(), reuse_count -> 2.
    let explain2 = db.explain_query(&query).unwrap();
    assert_eq!(explain2.cache_hit, Some(true));
    assert!(
        explain2.plan_reuse_count.unwrap_or(0) >= 2,
        "plan should have been reused at least twice (once per explain_query call)"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn test_plan_invalidation_on_write() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    db.create_collection("cache_write", 4, crate::DistanceMetric::Cosine)
        .unwrap();

    let coll = db.get_collection("cache_write").unwrap();
    coll.upsert(vec![crate::Point {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    let query = select_query("cache_write");
    let params = std::collections::HashMap::new();

    // Populate cache.
    let _ = db.execute_query(&query, &params).unwrap();

    // Upsert invalidates via write_generation change.
    coll.upsert(vec![crate::Point {
        id: 2,
        vector: vec![0.0, 1.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    // Cache should miss because write_generation changed.
    let explain = db.explain_query(&query).unwrap();
    assert_eq!(explain.cache_hit, Some(false), "should miss after upsert");
}

#[cfg(feature = "persistence")]
#[test]
fn test_plan_invalidation_on_delete() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    db.create_collection("cache_del", 4, crate::DistanceMetric::Cosine)
        .unwrap();

    let coll = db.get_collection("cache_del").unwrap();
    coll.upsert(vec![crate::Point {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    let query = select_query("cache_del");
    let params = std::collections::HashMap::new();

    // Populate cache.
    let _ = db.execute_query(&query, &params).unwrap();

    // Delete invalidates via write_generation change.
    coll.delete(&[1]).unwrap();

    let explain = db.explain_query(&query).unwrap();
    assert_eq!(explain.cache_hit, Some(false), "should miss after delete");
}

#[cfg(feature = "persistence")]
#[test]
fn test_plan_invalidation_on_drop_recreate() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    db.create_collection("cache_drop", 4, crate::DistanceMetric::Cosine)
        .unwrap();

    let coll = db.get_collection("cache_drop").unwrap();
    coll.upsert(vec![crate::Point {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    let query = select_query("cache_drop");
    let params = std::collections::HashMap::new();

    // Populate cache.
    let _ = db.execute_query(&query, &params).unwrap();

    // Drop + recreate invalidates via schema_version change.
    db.delete_collection("cache_drop").unwrap();
    db.create_collection("cache_drop", 4, crate::DistanceMetric::Cosine)
        .unwrap();

    let explain = db.explain_query(&query).unwrap();
    assert_eq!(
        explain.cache_hit,
        Some(false),
        "should miss after drop + recreate"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn test_explain_first_query_cache_miss() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    db.create_collection("cache_miss", 4, crate::DistanceMetric::Cosine)
        .unwrap();

    let query = select_query("cache_miss");

    // EXPLAIN on a never-executed query should show cache_hit: false.
    let explain = db.explain_query(&query).unwrap();
    assert_eq!(explain.cache_hit, Some(false));
    assert_eq!(explain.plan_reuse_count, Some(0));
}

/// Verifies that calling `add_edge` on a collection increments `write_generation`
/// and therefore invalidates any cached query plan for that collection (CACHE-01).
///
/// `add_edge` is a write operation on the graph sub-layer of a `Collection`.
/// The cache invalidation contract requires that **every** write operation — vector
/// upserts, deletes, *and* graph edge mutations — advances `write_generation`.
/// This test asserts that contract for graph mutations specifically.
#[cfg(feature = "persistence")]
#[test]
fn test_plan_invalidation_on_graph_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::Database::open(dir.path()).unwrap();

    db.create_collection("cache_graph", 4, crate::DistanceMetric::Cosine)
        .unwrap();

    let coll = db.get_collection("cache_graph").unwrap();

    // Seed with one vector point so the query returns results and the
    // planner can produce a stable plan to cache.
    coll.upsert(vec![crate::Point {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
        sparse_vectors: None,
    }])
    .unwrap();

    let query = select_query("cache_graph");
    let params = std::collections::HashMap::new();

    // Populate the plan cache via execute_query.
    let _ = db.execute_query(&query, &params).unwrap();

    // Confirm the plan is cached before the graph mutation.
    let explain_before = db.explain_query(&query).unwrap();
    assert_eq!(
        explain_before.cache_hit,
        Some(true),
        "plan should be cached before graph mutation"
    );

    // Graph mutation: add_edge bumps write_generation, invalidating the cached plan.
    let edge = crate::GraphEdge::new(1, 1, 2, "KNOWS").expect("edge should be valid");
    coll.add_edge(edge).unwrap();

    // The cache key now encodes a newer write_generation — it must miss.
    let explain_after = db.explain_query(&query).unwrap();
    assert_eq!(
        explain_after.cache_hit,
        Some(false),
        "plan should be invalidated after add_edge"
    );
}

// ---- schema_version bumped by load_collections (C-3) ----

/// Verifies that `schema_version` is incremented when `load_collections`
/// loads at least one collection from disk on re-open.
///
/// This ensures that any plan key built before the re-open (with the old
/// schema_version) cannot match a key built after, preventing stale-plan
/// cache hits across process restarts (CACHE-01).
#[cfg(feature = "persistence")]
#[test]
fn test_schema_version_after_load_collections() {
    let dir = tempfile::tempdir().unwrap();

    // Phase 1: create a collection and close the DB.
    {
        let db = crate::Database::open(dir.path()).unwrap();
        // Fresh empty DB: no collections loaded, version stays 0.
        assert_eq!(db.schema_version(), 0, "empty DB starts at 0");
        db.create_collection("reload_test", 4, crate::DistanceMetric::Cosine)
            .unwrap();
        // After create_collection: version = 1.
        assert_eq!(db.schema_version(), 1, "after create = 1");
        // DB drops here, persisting the collection to disk.
    }

    // Phase 2: re-open the DB. load_collections loads "reload_test" from disk,
    // bumping schema_version by 1 (from 0 to 1).
    {
        let db2 = crate::Database::open(dir.path()).unwrap();
        assert_eq!(
            db2.schema_version(),
            1,
            "schema_version should be 1 after loading one collection from disk"
        );
    }
}
