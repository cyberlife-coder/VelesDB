//! Core-invoked telemetry — exactly-once per completed operation.
//!
//! Feature: core-control-plane-boundary, Property 5
//!
//! **Property 5: Telemetry fires exactly once per completed operation** —
//! for any completed query (including compound `UNION`/`INTERSECT`/`EXCEPT`
//! and `JOIN`) or successful upsert, with an observer present, the
//! corresponding telemetry hook is invoked exactly once *after* the
//! data-plane op completes: `on_query(collection, duration_us)` for a read, or
//! `on_upsert(collection, point_count)` for a write, carrying the target
//! collection and — for upserts — the exact number of points affected.
//!
//! **Validates: Requirements 2.1, 2.2, 2.3, 2.5**
//!
//! A counting spy observer, installed at open time via
//! `Database::open_with_observer`, tallies every `on_query` / `on_upsert`
//! invocation with the `(collection, value)` it observed. Because the
//! telemetry hook fires at the `Database` facade (`execute_query` wraps a
//! single top-level `execute_query_inner`, DML fires once at the DML entry),
//! compound and JOIN sub-executions that re-enter the collection executor must
//! NOT re-fire — so the exactly-once tally is the real guard against a
//! double-fire regression. A fresh temp-dir `Database` is built per case so
//! the tallies never bleed across iterations.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use proptest::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, Database, DatabaseObserver, DistanceMetric, Point, QueryAccessContext,
    Result as CoreResult,
};

/// Counting spy: records `(collection, duration_us)` for every `on_query` and
/// `(collection, point_count)` for every `on_upsert`, so a test can assert the
/// exact number of invocations and the exact captured values.
#[derive(Default)]
struct CountingSpy {
    queries: Mutex<Vec<(String, u64)>>,
    upserts: Mutex<Vec<(String, usize)>>,
}

impl CountingSpy {
    fn new() -> Self {
        Self::default()
    }

    fn query_calls(&self) -> Vec<(String, u64)> {
        self.queries.lock().clone()
    }

    fn upsert_calls(&self) -> Vec<(String, usize)> {
        self.upserts.lock().clone()
    }
}

impl DatabaseObserver for CountingSpy {
    // Allow-all read gate: the query genuinely executes so `on_query` fires
    // after real results are produced.
    fn on_query_request(
        &self,
        _ctx: &QueryAccessContext,
    ) -> CoreResult<velesdb_core::AccessDecision> {
        Ok(velesdb_core::AccessDecision::Allow)
    }

    fn on_query(&self, collection: &str, duration_us: u64) {
        self.queries
            .lock()
            .push((collection.to_string(), duration_us));
    }

    fn on_upsert(&self, collection: &str, point_count: usize) {
        self.upserts
            .lock()
            .push((collection.to_string(), point_count));
    }
}

/// Opens a temp-dir database wired with the counting spy.
fn open_db(spy: Arc<CountingSpy>) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), spy as Arc<dyn DatabaseObserver>)
        .expect("test: open db with observer");
    (dir, db)
}

/// Seeds the `items` vector collection used by plain-SELECT and compound
/// (single-collection `UNION`/`INTERSECT`/`EXCEPT`) reads.
fn seed_items(db: &Database) {
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create items");
    let items = db.get_vector_collection("items").expect("test: get items");
    items
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 100})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "science", "price": 25})),
            ),
            Point::new(
                3,
                vec![0.8, 0.2, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 50})),
            ),
        ])
        .expect("test: upsert items");
}

/// Seeds a `products` vector collection and matching `inventory` metadata
/// collection so a JOIN on the primary key yields rows.
fn seed_join(db: &Database) {
    db.create_vector_collection("products", 4, DistanceMetric::Cosine)
        .expect("test: create products");
    let products = db
        .get_vector_collection("products")
        .expect("test: get products");
    products
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"category": "audio"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"category": "input"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"category": "display"})),
            ),
        ])
        .expect("test: upsert products");

    db.create_metadata_collection("inventory")
        .expect("test: create inventory");
    let inventory = db
        .get_metadata_collection("inventory")
        .expect("test: get inventory");
    inventory
        .upsert(vec![
            Point::metadata_only(1, json!({"price": 99.99})),
            Point::metadata_only(2, json!({"price": 149.99})),
            Point::metadata_only(3, json!({"price": 399.99})),
        ])
        .expect("test: upsert inventory");
}

/// Creates the metadata-only `sink` collection that `VelesQL` `INSERT` writes to.
fn seed_sink(db: &Database) {
    db.create_metadata_collection("sink")
        .expect("test: create sink");
}

/// Builds a multi-row `VelesQL` `INSERT` writing `n` rows into `sink`.
fn build_insert_sql(n: u32) -> String {
    let rows: Vec<String> = (1..=n).map(|i| format!("({i}, 'r{i}')")).collect();
    format!("INSERT INTO sink (id, name) VALUES {}", rows.join(", "))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(120))]

    /// Property 5: telemetry fires exactly once per completed operation.
    ///
    /// * `variant` selects the operation shape — plain SELECT, each compound
    ///   set operator, a JOIN, or a multi-row INSERT.
    /// * Reads assert `on_query` fired exactly once (with the target
    ///   collection) and `on_upsert` never fired.
    /// * The INSERT asserts `on_upsert` fired exactly once with the exact
    ///   number of points affected.
    #[test]
    fn telemetry_fires_exactly_once_per_completed_operation(
        variant in 0u8..7,
        threshold in 0i64..=200,
        limit in 1u32..=10,
        rows in 1u32..=5,
    ) {
        let spy = Arc::new(CountingSpy::new());
        let (_dir, db) = open_db(spy.clone());

        // Each arm returns the SQL to run plus what the tally must look like:
        // Some((collection, point_count)) for an upsert, or None for a read
        // (whose expected collection is carried separately).
        match variant {
            6 => {
                // ---- Upsert path: VelesQL multi-row INSERT ----
                seed_sink(&db);
                let sql = build_insert_sql(rows);
                let query = Parser::parse(&sql).expect("test: parse INSERT");
                db.execute_query(&query, &HashMap::new())
                    .expect("test: execute INSERT");

                let upserts = spy.upsert_calls();
                prop_assert_eq!(
                    upserts.len(),
                    1_usize,
                    "on_upsert must fire exactly once, got {} for sql {}",
                    upserts.len(),
                    sql
                );
                let (collection, point_count) = &upserts[0];
                prop_assert_eq!(
                    collection.as_str(),
                    "sink",
                    "on_upsert must report the target collection for sql {}",
                    sql
                );
                prop_assert_eq!(
                    *point_count,
                    rows as usize,
                    "on_upsert must report the exact affected point count for sql {}",
                    sql
                );
            }
            5 => {
                // ---- Read path: JOIN across two collections ----
                seed_join(&db);
                let sql = String::from(
                    "SELECT * FROM products JOIN inventory ON products.id = inventory.id LIMIT 10",
                );
                let query = Parser::parse(&sql).expect("test: parse JOIN");
                db.execute_query(&query, &HashMap::new())
                    .expect("test: execute JOIN");
                assert_single_query(&spy, "products", &sql)?;
            }
            other => {
                // ---- Read path: plain SELECT or compound set operation ----
                seed_items(&db);
                let sql = build_read_sql(other, threshold, limit);
                let query = Parser::parse(&sql).expect("test: parse read query");
                db.execute_query(&query, &HashMap::new())
                    .expect("test: execute read query");
                assert_single_query(&spy, "items", &sql)?;
            }
        }
    }
}

/// Builds a read query with a KNOWN target collection (`items`).
///
/// * `0` — plain relational SELECT.
/// * `1` — `UNION` compound (single collection, two WHERE arms).
/// * `2` — `UNION ALL` compound.
/// * `3` — `INTERSECT` compound.
/// * `_` — `EXCEPT` compound.
fn build_read_sql(variant: u8, threshold: i64, limit: u32) -> String {
    let left = format!("SELECT * FROM items WHERE price > {threshold}");
    let right = String::from("SELECT * FROM items WHERE category = 'tech'");
    match variant {
        0 => format!("SELECT * FROM items WHERE price > {threshold} LIMIT {limit}"),
        1 => format!("{left} UNION {right}"),
        2 => format!("{left} UNION ALL {right}"),
        3 => format!("{left} INTERSECT {right}"),
        _ => format!("{left} EXCEPT {right}"),
    }
}

/// Asserts the read fired `on_query` exactly once (for `expected_collection`)
/// and never fired `on_upsert`.
fn assert_single_query(
    spy: &CountingSpy,
    expected_collection: &str,
    sql: &str,
) -> Result<(), TestCaseError> {
    let queries = spy.query_calls();
    prop_assert_eq!(
        queries.len(),
        1_usize,
        "on_query must fire exactly once, got {} for sql {}",
        queries.len(),
        sql
    );
    prop_assert_eq!(
        queries[0].0.as_str(),
        expected_collection,
        "on_query must report the target collection for sql {}",
        sql
    );
    // A read never triggers the write-path telemetry.
    let upserts = spy.upsert_calls();
    prop_assert!(
        upserts.is_empty(),
        "a read must not fire on_upsert, got {} for sql {}",
        upserts.len(),
        sql
    );
    Ok(())
}

// ===========================================================================
// Deterministic coverage pins: compound (UNION/INTERSECT/EXCEPT) + JOIN each
// yield exactly one on_query. These guarantee the property's compound/JOIN
// arms are exercised regardless of proptest sampling.
// ===========================================================================

/// Runs `sql` against a freshly-seeded `items` collection and returns the
/// recorded `on_query` collections.
fn run_items_read(sql: &str) -> Vec<(String, u64)> {
    let spy = Arc::new(CountingSpy::new());
    let (_dir, db) = open_db(spy.clone());
    seed_items(&db);
    let query = Parser::parse(sql).expect("test: parse read query");
    db.execute_query(&query, &HashMap::new())
        .expect("test: execute read query");
    spy.query_calls()
}

#[test]
fn compound_union_fires_on_query_exactly_once() {
    let calls = run_items_read(
        "SELECT * FROM items WHERE price > 10 UNION SELECT * FROM items WHERE category = 'tech'",
    );
    assert_eq!(calls.len(), 1, "compound UNION must fire on_query once");
    assert_eq!(calls[0].0, "items");
}

#[test]
fn compound_intersect_fires_on_query_exactly_once() {
    let calls = run_items_read(
        "SELECT * FROM items WHERE price > 10 INTERSECT SELECT * FROM items WHERE category = 'tech'",
    );
    assert_eq!(calls.len(), 1, "compound INTERSECT must fire on_query once");
    assert_eq!(calls[0].0, "items");
}

#[test]
fn compound_except_fires_on_query_exactly_once() {
    let calls = run_items_read(
        "SELECT * FROM items WHERE price > 10 EXCEPT SELECT * FROM items WHERE category = 'tech'",
    );
    assert_eq!(calls.len(), 1, "compound EXCEPT must fire on_query once");
    assert_eq!(calls[0].0, "items");
}

#[test]
fn join_fires_on_query_exactly_once() {
    let spy = Arc::new(CountingSpy::new());
    let (_dir, db) = open_db(spy.clone());
    seed_join(&db);
    let query = Parser::parse(
        "SELECT * FROM products JOIN inventory ON products.id = inventory.id LIMIT 10",
    )
    .expect("test: parse JOIN");
    db.execute_query(&query, &HashMap::new())
        .expect("test: execute JOIN");

    let calls = spy.query_calls();
    assert_eq!(calls.len(), 1, "JOIN must fire on_query exactly once");
    assert_eq!(calls[0].0, "products");
    assert!(
        spy.upsert_calls().is_empty(),
        "a JOIN read must not fire on_upsert"
    );
}
