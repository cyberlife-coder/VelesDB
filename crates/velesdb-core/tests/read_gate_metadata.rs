//! Read-path control-plane gate — collection metadata reads.
//!
//! `DESCRIBE COLLECTION` returns a collection's `point_count`, and a
//! plan-only `EXPLAIN` returns statistics calibrated from the store — both
//! are reads of collection data the observer decides like any other: a
//! denial refuses them, and a scope — there is no row to narrow — refuses
//! them too rather than leak the unscoped count.
//!
//! Memory reads through `AgentMemory` are collection
//! reads on `_semantic_memory` & co., so they pass the same gate.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::Arc;

use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{
    AccessDecision, AccessScope, Database, DatabaseObserver, DistanceMetric, Error, Point,
    QueryAccessContext,
};

/// What the observer answers for the named collection.
enum Rule {
    Deny(&'static str),
    Scope(&'static str),
}

/// Observer whose single rule applies to every collection it is asked about.
struct UniformObserver(Rule);

impl DatabaseObserver for UniformObserver {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        let _ = ctx;
        Ok(match &self.0 {
            Rule::Deny(message) => AccessDecision::Deny(Error::Query((*message).to_string())),
            Rule::Scope(predicate) => {
                let filter = Parser::parse(&format!("SELECT * FROM c WHERE {predicate}"))
                    .expect("test: parse scope")
                    .select
                    .where_clause;
                #[allow(clippy::field_reassign_with_default)] // non_exhaustive
                let mut scope = AccessScope::default();
                scope.filter = filter;
                AccessDecision::AllowWithScope(scope)
            }
        })
    }
}

fn setup(rule: Rule) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), Arc::new(UniformObserver(rule)))
        .expect("test: open db");
    db.create_vector_collection("c", 4, DistanceMetric::Cosine)
        .expect("test: create c");
    let c = db.get_vector_collection("c").expect("test: get c");
    c.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
    ])
    .expect("test: seed c");
    (dir, db)
}

fn run(db: &Database, sql: &str) -> velesdb_core::Result<usize> {
    let query = Parser::parse(sql).expect("test: parse");
    db.execute_query(&query, &HashMap::new()).map(|v| v.len())
}

/// GIVEN the observer denies `c`
/// WHEN `DESCRIBE COLLECTION c` runs — a metadata read carrying the
///      collection's `point_count`
/// THEN it fails with the observer's error instead of leaking the count.
#[test]
fn describe_collection_is_gated() {
    let (_dir, db) = setup(Rule::Deny("no metadata reads on c"));
    let err = run(&db, "DESCRIBE COLLECTION c").expect_err("test: denied");
    assert!(err.to_string().contains("no metadata reads on c"), "{err}");
}

/// GIVEN the observer scopes `c`
/// WHEN `DESCRIBE COLLECTION c` runs
/// THEN it is refused (VELES-009) rather than returning an unscoped count —
///      the same fail-closed answer every uncomposable read gives.
#[test]
fn describe_collection_refuses_a_scope() {
    let (_dir, db) = setup(Rule::Scope("cat = 'a'"));
    let err = run(&db, "DESCRIBE COLLECTION c").expect_err("test: refused");
    assert!(matches!(err, Error::Config(_)), "VELES-009, got {err:?}");
}

/// GIVEN the observer denies `c`
/// WHEN a plan-only `EXPLAIN` over `c` runs — it exposes statistics the
///      planner calibrates from the store
/// THEN it fails with the observer's error.
#[test]
fn explain_is_gated() {
    let (_dir, db) = setup(Rule::Deny("no stats on c"));
    let err = run(&db, "EXPLAIN SELECT * FROM c WHERE cat = 'a'").expect_err("test: denied");
    assert!(err.to_string().contains("no stats on c"), "{err}");
}

/// GIVEN the observer scopes `c`
/// WHEN a plan-only `EXPLAIN` over `c` runs
/// THEN it is refused rather than leak unscoped statistics.
#[test]
fn explain_refuses_a_scope() {
    let (_dir, db) = setup(Rule::Scope("cat = 'a'"));
    let err = run(&db, "EXPLAIN SELECT * FROM c WHERE cat = 'a'").expect_err("test: refused");
    assert!(matches!(err, Error::Config(_)), "VELES-009, got {err:?}");
}

/// GIVEN `SHOW COLLECTIONS` names collections but reads no row data
/// WHEN the observer denies every collection
/// THEN the listing still answers — the gate is about reads of collection
///      data, and a name is not one.
#[test]
fn show_collections_is_not_a_collection_read() {
    let (_dir, db) = setup(Rule::Deny("deny everything"));
    assert_eq!(run(&db, "SHOW COLLECTIONS").expect("test: SHOW runs"), 1);
}

/// GIVEN an `AgentMemory` on a database whose observer denies reads
/// WHEN its `VelesQL` bridge, a native semantic query, or `list_all` runs
/// THEN each fails — the memory collections are collection reads like any
///      other.
#[test]
fn agent_memory_reads_are_gated() {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Arc::new(
        Database::open_with_observer(dir.path(), Arc::new(UniformObserver(Rule::Deny("deny"))))
            .expect("test: open db"),
    );
    let memory = velesdb_core::agent::AgentMemory::new(Arc::clone(&db)).expect("test: memory");

    let params = HashMap::new();
    assert!(
        memory
            .query_semantic("SELECT * FROM _semantic_memory", &params)
            .is_err(),
        "memory VelesQL bridge must be gated"
    );
    assert!(
        memory.semantic().query(&[1.0, 0.0, 0.0, 0.0], 4).is_err(),
        "semantic query must be gated"
    );
    assert!(
        memory.semantic().list_all().is_err(),
        "semantic list_all must be gated"
    );
}
