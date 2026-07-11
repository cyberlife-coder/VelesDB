//! Read-path control-plane gate — deny semantics.
//!
//! Feature: core-control-plane-boundary, Property 3
//!
//! **Property 3: Deny yields the supplied error and zero results** —
//! when the read-path observer hook returns `AccessDecision::Deny(err)`,
//! `Database::execute_query` returns exactly `Err(err)` (same variant and
//! message) and produces no results.
//!
//! **Validates: Requirements 1.4**
//!
//! The observer is installed at open time via `Database::open_with_observer`.
//! Data is inserted so that, absent the deny decision, the queries would
//! return rows — the companion allow-path test below pins that fact, so the
//! property genuinely demonstrates that deny *suppresses* otherwise-present
//! results rather than trivially matching on an empty collection.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use proptest::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, AccessDecision, Database, DatabaseObserver, DistanceMetric, Error, Point,
    QueryAccessContext,
};

/// Builds the error the observer denies with, from a proptest-generated
/// variant selector + message. Kept as a free function so the observer and the
/// assertion construct byte-identical errors.
fn make_error(variant: u8, message: &str) -> Error {
    match variant % 3 {
        0 => Error::Query(message.to_string()),
        1 => Error::Config(message.to_string()),
        _ => Error::SearchNotSupported(message.to_string()),
    }
}

/// Observer that denies every read with a configurable error. The current
/// `(variant, message)` pair is swapped between proptest iterations through a
/// lock so a single opened `Database` can exercise many deny errors.
struct DenyObserver {
    deny_with: RwLock<(u8, String)>,
}

impl DenyObserver {
    fn new() -> Self {
        Self {
            deny_with: RwLock::new((0, String::from("denied"))),
        }
    }

    fn set(&self, variant: u8, message: &str) {
        *self.deny_with.write() = (variant, message.to_string());
    }
}

impl DatabaseObserver for DenyObserver {
    fn on_query_request(&self, _ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        let (variant, message) = {
            let guard = self.deny_with.read();
            (guard.0, guard.1.clone())
        };
        Ok(AccessDecision::Deny(make_error(variant, &message)))
    }
}

/// Opens a temp-dir database wired with `observer`, creates the `items`
/// collection, and inserts rows that a read would otherwise return.
fn setup(observer: Arc<dyn DatabaseObserver>) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), observer).expect("test: open db");
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create collection");
    let collection = db
        .get_vector_collection("items")
        .expect("test: get collection");
    collection
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
    (dir, db)
}

/// Builds a valid `VelesQL` SELECT over `items` from proptest inputs, so both
/// the query text and the deny error vary across iterations.
fn build_query(where_kind: u8, threshold: i64, limit: u32) -> String {
    let predicate = match where_kind % 3 {
        0 => String::new(),
        1 => format!(" WHERE price > {threshold}"),
        _ => String::from(" WHERE category = 'tech'"),
    };
    format!("SELECT * FROM items{predicate} LIMIT {limit}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 3: `Deny(err)` ⇒ `execute_query` returns exactly `Err(err)`
    /// (same variant + message) and no results, for any valid read query.
    #[test]
    fn deny_yields_supplied_error_and_zero_results(
        variant in any::<u8>(),
        message in "[a-zA-Z0-9 _.,:-]{1,60}",
        where_kind in any::<u8>(),
        threshold in 0i64..=200,
        limit in 1u32..=10,
    ) {
        let observer = Arc::new(DenyObserver::new());
        observer.set(variant, &message);
        let (_dir, db) = setup(observer.clone() as Arc<dyn DatabaseObserver>);

        let sql = build_query(where_kind, threshold, limit);
        let query = Parser::parse(&sql).expect("test: parse SELECT");

        let result = db.execute_query(&query, &HashMap::new());

        // Deny aborts before execution: the call must be Err (no Vec of rows
        // is ever produced), and the error must be exactly what was supplied.
        let expected = make_error(variant, &message);
        match result {
            Ok(rows) => prop_assert!(
                false,
                "deny must abort the read, got {} result rows",
                rows.len()
            ),
            Err(err) => {
                prop_assert_eq!(
                    err.code(),
                    expected.code(),
                    "deny must surface the supplied error variant"
                );
                prop_assert_eq!(
                    err.to_string(),
                    expected.to_string(),
                    "deny must surface the supplied error message verbatim"
                );
            }
        }
    }
}

/// Companion pin: the same query the property denies returns rows when no
/// observer denies it. This guarantees the property above is meaningful —
/// deny is suppressing results that genuinely exist, not matching an empty set.
#[test]
fn allow_path_returns_the_rows_that_deny_suppresses() {
    // No observer: the read path is unmodified and returns the inserted rows.
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open db");
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create collection");
    let collection = db
        .get_vector_collection("items")
        .expect("test: get collection");
    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 100})),
            ),
            Point::new(
                3,
                vec![0.8, 0.2, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 50})),
            ),
        ])
        .expect("test: upsert items");

    let query = Parser::parse("SELECT * FROM items WHERE category = 'tech' LIMIT 10")
        .expect("test: parse SELECT");
    let rows = db
        .execute_query(&query, &HashMap::new())
        .expect("test: allow path must succeed");

    assert!(
        !rows.is_empty(),
        "allow path must return the rows that the deny property suppresses"
    );
}
