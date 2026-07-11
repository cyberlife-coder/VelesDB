//! Read-path control-plane gate — observer-transparent baseline equivalence.
//!
//! Feature: core-control-plane-boundary, Property 1
//!
//! **Property 1: Observer-transparent baseline equivalence** — for any database
//! state and valid read query, executing with **no observer**, with the
//! **default `DatabaseObserver`**, or with an observer whose `on_query_request`
//! returns `AccessDecision::Allow` (no scope) produces identical results —
//! same rows, same order — as the pre-hook data-plane baseline.
//!
//! **Validates: Requirements 1.6, 1.8, 3.1, 3.2**
//!
//! Three databases are seeded with byte-identical points and run the same
//! query: (a) opened with `Database::open` (no observer / the pre-hook
//! baseline), (b) opened with a `DefaultObserver` relying on trait defaults,
//! and (c) opened with a spy `AllowObserver` that records the call and returns
//! `Allow`. The property asserts the ordered list of result ids is identical
//! across all three, and the spy confirms the allow path genuinely fired
//! (so the equivalence is not an artifact of the observer being bypassed).

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use proptest::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, AccessDecision, Database, DatabaseObserver, DistanceMetric, Point,
    QueryAccessContext,
};

/// Observer that overrides nothing — every hook uses the trait default
/// (no-op / allow-all). Proves the default port implementation is transparent
/// (Requirement 3.1).
struct DefaultObserver;
impl DatabaseObserver for DefaultObserver {}

/// Spy observer whose `on_query_request` returns `Allow` (no scope) and counts
/// invocations, so the test can confirm the allow path actually fired
/// (Requirement 1.6).
struct AllowObserver {
    calls: AtomicUsize,
}

impl AllowObserver {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl DatabaseObserver for AllowObserver {
    fn on_query_request(&self, _ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AccessDecision::Allow)
    }
}

/// Creates the `items` collection on `db` and upserts one point per
/// `(price, is_tech)` input, assigning ids `1..=n`. Identical inputs yield
/// byte-identical collections across every database, so any output difference
/// is attributable to the observer, not the data.
fn seed(db: &Database, rows: &[(i64, bool)]) {
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create collection");
    let collection = db
        .get_vector_collection("items")
        .expect("test: get collection");
    let points: Vec<Point> = rows
        .iter()
        .enumerate()
        .map(|(i, (price, is_tech))| {
            let id = i as u64 + 1;
            let category = if *is_tech { "tech" } else { "science" };
            // Deterministic, cast-free vector variety so vector-search shapes
            // produce a meaningful (yet reproducible) ordering.
            let second = match i % 3 {
                0 => 0.0_f32,
                1 => 0.1,
                _ => 0.2,
            };
            Point::new(
                id,
                vec![1.0, second, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": category, "price": price})),
            )
        })
        .collect();
    collection.upsert(points).expect("test: upsert items");
}

/// Runs `sql` against `db` and returns the ordered list of result ids — the
/// canonical observable used to compare rows *and* order.
fn run(db: &Database, sql: &str) -> Vec<u64> {
    let query = Parser::parse(sql).expect("test: parse query");
    let results = db
        .execute_query(&query, &HashMap::new())
        .expect("test: execute query");
    results.iter().map(|r| r.point.id).collect()
}

/// Builds a valid `VelesQL` read query with a deterministic ordering, so the
/// result row order is stable and any divergence is a genuine observer effect.
fn build_query(shape: u8, threshold: i64, limit: u32) -> String {
    match shape % 3 {
        0 => format!("SELECT * FROM items WHERE price > {threshold} ORDER BY price DESC LIMIT {limit}"),
        1 => format!("SELECT * FROM items WHERE category = 'tech' ORDER BY price ASC LIMIT {limit}"),
        // Vector search top-k: `> -2.0` selects every row (cosine ∈ [-1, 1]),
        // then a scalar ORDER BY pins a deterministic order.
        _ => format!(
            "SELECT * FROM items WHERE similarity(vector, [1.0, 0.0, 0.0, 0.0]) > -2.0 ORDER BY price DESC LIMIT {limit}"
        ),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 1: no observer, the default observer, and an `Allow`-returning
    /// observer all produce identical result rows and order for the same query
    /// over identical data.
    #[test]
    fn observer_transparent_baseline_equivalence(
        rows in prop::collection::vec((0i64..=200, any::<bool>()), 1..=15),
        shape in any::<u8>(),
        threshold in 0i64..=200,
        limit in 1u32..=20,
    ) {
        let sql = build_query(shape, threshold, limit);

        // (a) Baseline: no observer — the unmodified pre-hook read path.
        let baseline_dir = TempDir::new().expect("test: tempdir");
        let baseline = Database::open(baseline_dir.path()).expect("test: open baseline");
        seed(&baseline, &rows);
        let baseline_ids = run(&baseline, &sql);

        // (b) Default observer: relies entirely on trait defaults.
        let default_dir = TempDir::new().expect("test: tempdir");
        let default_db = Database::open_with_observer(
            default_dir.path(),
            Arc::new(DefaultObserver) as Arc<dyn DatabaseObserver>,
        )
        .expect("test: open default-observer db");
        seed(&default_db, &rows);
        let default_ids = run(&default_db, &sql);

        // (c) Allow spy observer: returns `Allow` and records the call.
        let spy = Arc::new(AllowObserver::new());
        let spy_dir = TempDir::new().expect("test: tempdir");
        let spy_db = Database::open_with_observer(
            spy_dir.path(),
            spy.clone() as Arc<dyn DatabaseObserver>,
        )
        .expect("test: open allow-observer db");
        seed(&spy_db, &rows);
        let spy_ids = run(&spy_db, &sql);

        // Same rows, same order across all three.
        prop_assert_eq!(&baseline_ids, &default_ids, "default observer must match the no-observer baseline");
        prop_assert_eq!(&baseline_ids, &spy_ids, "allow observer must match the no-observer baseline");

        // The spy proves the allow path genuinely fired: equivalence is not an
        // artifact of the read gate being skipped.
        prop_assert!(spy.calls() >= 1, "allow observer's read hook must have fired");
    }
}
