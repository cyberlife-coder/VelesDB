//! Core-invoked read telemetry on the reads that build no `VelesQL` query.
//!
//! `execute_query` always reported its read through
//! `DatabaseObserver::on_query`; the governed reads that bypass it —
//! `gated_search`, `execute_aggregate`, and the searches a caller runs after
//! the gate admitted them (sparse, batch, multi-query) — reported nothing,
//! so the REST server measured them itself and called the deprecated
//! `Database::notify_query` shim. Each case here asserts exactly one
//! `on_query` naming the collection for a completed read, and none for a
//! refused or failed one. A statement is one read however many scalar
//! subqueries it resolves: the subqueries are part of it, not reads of their
//! own.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, AccessDecision, Database, DatabaseObserver, DistanceMetric, Error, GatedRead,
    Point, QueryAccessContext, QueryOperationKind, Result as CoreResult,
};

/// Records the collection of every `on_query`; refuses reads of `secret`.
#[derive(Default)]
struct QuerySpy {
    queries: Mutex<Vec<String>>,
}

impl QuerySpy {
    fn calls(&self) -> Vec<String> {
        self.queries.lock().clone()
    }
}

impl DatabaseObserver for QuerySpy {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> CoreResult<AccessDecision> {
        if ctx.collection == "secret" {
            return Ok(AccessDecision::Deny(Error::Config("denied".to_string())));
        }
        Ok(AccessDecision::Allow)
    }

    fn on_query(&self, collection: &str, _duration_us: u64) {
        self.queries.lock().push(collection.to_string());
    }
}

fn seeded_db() -> (TempDir, Database, Arc<QuerySpy>) {
    let spy = Arc::new(QuerySpy::default());
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), spy.clone() as Arc<dyn DatabaseObserver>)
        .expect("test: open db with observer");
    for name in ["docs", "secret"] {
        db.create_vector_collection(name, 4, DistanceMetric::Cosine)
            .expect("test: create collection");
        db.get_vector_collection(name)
            .expect("test: collection")
            .upsert([
                Point::new(
                    1,
                    vec![1.0, 0.0, 0.0, 0.0],
                    Some(json!({"tag": "a", "n": 1})),
                ),
                Point::new(
                    2,
                    vec![0.0, 1.0, 0.0, 0.0],
                    Some(json!({"tag": "b", "n": 3})),
                ),
            ])
            .expect("test: seed");
    }
    (dir, db, spy)
}

const QUERY: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

fn dense() -> GatedRead<'static> {
    GatedRead::Dense {
        query: &QUERY,
        k: 2,
        ef: None,
        quality: None,
        filter: None,
    }
}

#[test]
fn gated_search_reports_once() {
    let (_dir, db, spy) = seeded_db();
    let results = db
        .gated_search("docs", None, None, dense())
        .expect("test: search");
    assert_eq!(results.len(), 2);
    assert_eq!(spy.calls(), vec!["docs".to_string()]);
}

#[test]
fn a_refused_gated_search_reports_nothing() {
    let (_dir, db, spy) = seeded_db();
    assert!(db.gated_search("secret", None, None, dense()).is_err());
    assert!(spy.calls().is_empty());
}

#[test]
fn execute_aggregate_reports_once() {
    let (_dir, db, spy) = seeded_db();
    let query = parse("SELECT COUNT(*) FROM docs");
    db.execute_aggregate(&query, &HashMap::new())
        .expect("test: aggregate");
    assert_eq!(spy.calls(), vec!["docs".to_string()]);
}

#[test]
fn run_gated_read_reports_a_completed_read_once() {
    let (_dir, db, spy) = seeded_db();
    let outcome = db
        .run_gated_read(
            "docs",
            QueryOperationKind::VectorSearch,
            None,
            None,
            |scope| -> CoreResult<i32> {
                assert!(scope.is_none(), "an allowed read carries no scope");
                Ok(7)
            },
        )
        .expect("test: admitted");
    assert_eq!(outcome.ok(), Some(7));
    assert_eq!(spy.calls(), vec!["docs".to_string()]);
}

#[test]
fn run_gated_read_reports_neither_a_refusal_nor_a_failure() {
    let (_dir, db, spy) = seeded_db();

    let mut ran = false;
    let refused = db.run_gated_read(
        "secret",
        QueryOperationKind::VectorSearch,
        None,
        None,
        |_| -> CoreResult<()> {
            ran = true;
            Ok(())
        },
    );
    assert!(refused.is_err(), "the gate's refusal is the outer error");
    assert!(!ran, "a refused read never runs");

    let failed = db
        .run_gated_read(
            "docs",
            QueryOperationKind::VectorSearch,
            None,
            None,
            |_| -> CoreResult<()> { Err(Error::Config("boom".to_string())) },
        )
        .expect("test: admitted");
    assert!(failed.is_err(), "the read's own failure is the inner error");

    assert!(spy.calls().is_empty());
}

fn parse(sql: &str) -> velesdb_core::velesql::Query {
    Parser::parse(sql).expect("test: parse")
}

#[test]
fn a_select_reports_once_however_many_subqueries_it_resolves() {
    let (_dir, db, spy) = seeded_db();
    let params = HashMap::new();
    let aggregate_sub = parse("SELECT * FROM docs WHERE n > (SELECT AVG(n) FROM docs)");
    let rows = db
        .execute_query(&aggregate_sub, &params)
        .expect("test: aggregate subquery");
    assert_eq!(rows.len(), 1);
    assert_eq!(spy.calls(), vec!["docs".to_string()]);

    let scalar_sub = parse("SELECT * FROM docs WHERE n > (SELECT n FROM docs WHERE tag = 'a')");
    db.execute_query(&scalar_sub, &params)
        .expect("test: scalar subquery");
    assert_eq!(spy.calls(), vec!["docs".to_string(); 2]);
}

#[test]
fn an_aggregate_reports_once_however_many_subqueries_it_resolves() {
    let (_dir, db, spy) = seeded_db();
    let query = parse("SELECT COUNT(*) FROM docs WHERE n > (SELECT AVG(n) FROM docs)");
    db.execute_aggregate(&query, &HashMap::new())
        .expect("test: aggregate");
    assert_eq!(spy.calls(), vec!["docs".to_string()]);
}

#[test]
fn explain_analyze_reports_its_executed_read_once() {
    let (_dir, db, spy) = seeded_db();
    let query = parse("SELECT * FROM docs WHERE n > (SELECT AVG(n) FROM docs)");
    db.explain_analyze_query(&query, &HashMap::new())
        .expect("test: explain analyze");
    assert_eq!(spy.calls(), vec!["docs".to_string()]);
}
