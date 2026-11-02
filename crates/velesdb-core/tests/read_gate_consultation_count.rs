//! Read-path control-plane gate — one observer consultation per collection
//! per statement (issue #2588).
//!
//! A statement can fan out internally — a JOIN reaching a secondary
//! collection, `UNION`/`INTERSECT`/`EXCEPT` operands, a scalar subquery, a
//! `MATCH` with `@collection` enrichment, `SELECT EDGES`, or a plan read
//! preceding the row read (`EXPLAIN`, `EXPLAIN ANALYZE`). Whatever the shape,
//! the observer is consulted **once per (collection, operation kind) the
//! statement reads** — a logging/metering observer must never see the same
//! collection decided twice for one logical statement.
//!
//! The spy below records every `(collection, operation)` the gate asks
//! about; each scenario asserts the per-collection multiplicity is exactly
//! one.

#![cfg(feature = "persistence")]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{
    AccessDecision, Database, DatabaseObserver, DistanceMetric, GraphEdge, Point,
    QueryAccessContext, QueryOperationKind,
};

/// Counts every `on_query_request` consultation as `(collection, operation)`.
#[derive(Default)]
struct CountingObserver {
    consultations: Mutex<Vec<(String, QueryOperationKind)>>,
}

impl CountingObserver {
    /// Consultations per collection name, folded across operation kinds.
    fn per_collection(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for (collection, _) in self.consultations.lock().iter() {
            *counts.entry(collection.clone()).or_insert(0) += 1;
        }
        counts
    }

    /// Consultations per `(collection, operation)` pair (`QueryOperationKind`
    /// is not `Ord`, so the key carries its debug name).
    fn per_collection_operation(&self) -> BTreeMap<(String, String), usize> {
        let mut counts = BTreeMap::new();
        for (collection, operation) in self.consultations.lock().iter() {
            *counts
                .entry((collection.clone(), format!("{operation:?}")))
                .or_insert(0) += 1;
        }
        counts
    }
}

impl DatabaseObserver for CountingObserver {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        self.consultations
            .lock()
            .push((ctx.collection.to_string(), ctx.operation));
        Ok(AccessDecision::Allow)
    }
}

/// Opens a database under a counting observer with two vector collections
/// (`c`, `d`) and a graph collection (`g`, two Person nodes, one KNOWS edge).
fn setup() -> (TempDir, Database, Arc<CountingObserver>) {
    let observer = Arc::new(CountingObserver::default());
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), observer.clone()).expect("test: open db");
    for name in ["c", "d"] {
        db.create_vector_collection(name, 4, DistanceMetric::Cosine)
            .expect("test: create vector collection");
        let coll = db
            .get_vector_collection(name)
            .expect("test: get collection");
        coll.upsert(vec![
            Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"amount": 10}))),
            Point::new(2, vec![0.9, 0.1, 0.0, 0.0], Some(json!({"amount": 20}))),
        ])
        .expect("test: seed collection");
    }
    let ddl =
        Parser::parse("CREATE GRAPH COLLECTION g (dimension = 4, metric = 'cosine') SCHEMALESS")
            .expect("test: parse CREATE GRAPH");
    db.execute_query(&ddl, &HashMap::new())
        .expect("test: create g");
    let g = db.get_graph_collection("g").expect("test: get g");
    g.upsert_node_payload(1, &json!({"_labels": ["Person"], "name": "Ann"}))
        .expect("test: node 1");
    g.upsert_node_payload(2, &json!({"_labels": ["Person"], "name": "Bo"}))
        .expect("test: node 2");
    g.add_edge(GraphEdge::new(1, 1, 2, "KNOWS").expect("test: edge"))
        .expect("test: add edge");
    (dir, db, observer)
}

fn run(db: &Database, sql: &str) {
    let query = Parser::parse(sql).expect("test: parse");
    db.execute_query(&query, &HashMap::new())
        .expect("test: statement runs");
}

fn run_on(db: &Database, sql: &str, collection: &str) {
    let query = Parser::parse(sql).expect("test: parse");
    let params = HashMap::from([("_collection".to_string(), json!(collection))]);
    db.execute_query(&query, &params)
        .expect("test: statement runs");
}

/// Asserts every collection was consulted exactly once per operation kind.
fn assert_once_each(observer: &CountingObserver, context: &str) {
    let counts = observer.per_collection_operation();
    for ((collection, operation), n) in &counts {
        assert_eq!(
            *n, 1,
            "{context}: ({collection}, {operation:?}) consulted {n} times, want exactly 1 \
             (all consultations: {counts:?})"
        );
    }
}

#[test]
fn a_join_consults_each_collection_once() {
    let (_dir, db, observer) = setup();
    run(&db, "SELECT * FROM c JOIN d ON c.id = d.id LIMIT 10");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("c".to_string(), 1), ("d".to_string(), 1)]),
        "JOIN must consult its base and secondary collection once each"
    );
}

#[test]
fn a_self_join_consults_its_collection_once_per_kind() {
    let (_dir, db, observer) = setup();
    run(&db, "SELECT * FROM c JOIN c AS x ON c.id = x.id LIMIT 10");
    assert_once_each(&observer, "self-JOIN");
}

#[test]
fn a_union_consults_each_collection_once() {
    let (_dir, db, observer) = setup();
    run(&db, "SELECT * FROM c UNION SELECT * FROM d LIMIT 10");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("c".to_string(), 1), ("d".to_string(), 1)]),
        "UNION must consult each operand's collection once"
    );
}

#[test]
fn a_union_of_the_same_collection_consults_it_once() {
    let (_dir, db, observer) = setup();
    run(&db, "SELECT * FROM c UNION SELECT * FROM c LIMIT 10");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("c".to_string(), 1)]),
        "a repeated collection must still be decided once per statement"
    );
}

#[test]
fn a_scalar_subquery_on_another_collection_consults_each_once() {
    let (_dir, db, observer) = setup();
    run(
        &db,
        "SELECT * FROM c WHERE amount > (SELECT MAX(amount) AS m FROM d) LIMIT 10",
    );
    assert_once_each(&observer, "scalar subquery across collections");
}

#[test]
fn a_scalar_subquery_on_the_same_collection_consults_it_once() {
    let (_dir, db, observer) = setup();
    run(
        &db,
        "SELECT * FROM c WHERE amount > (SELECT MAX(amount) AS m FROM c) LIMIT 10",
    );
    assert_once_each(&observer, "same-collection scalar subquery");
}

#[test]
fn an_aggregate_scalar_subquery_consults_each_collection_once() {
    let (_dir, db, observer) = setup();
    let query = Parser::parse(
        "SELECT COUNT(*) AS n FROM c WHERE amount > (SELECT MAX(amount) AS m FROM d)",
    )
    .expect("test: parse aggregate");
    db.execute_aggregate(&query, &HashMap::new())
        .expect("test: aggregate runs");
    assert_once_each(&observer, "aggregate with scalar subquery");
}

#[test]
fn a_match_consults_target_and_each_annotation_once() {
    let (_dir, db, observer) = setup();
    run_on(
        &db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person@c) RETURN a, b LIMIT 10",
        "g",
    );
    assert_once_each(&observer, "MATCH with @collection");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("c".to_string(), 1), ("g".to_string(), 1)]),
    );
}

#[test]
fn a_match_naming_the_same_collection_twice_consults_it_once() {
    let (_dir, db, observer) = setup();
    run_on(
        &db,
        "MATCH (a:Person@c)-[:KNOWS]->(b:Person@c) RETURN a, b LIMIT 10",
        "g",
    );
    assert_once_each(&observer, "MATCH with a repeated @collection");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("c".to_string(), 1), ("g".to_string(), 1)]),
    );
}

#[test]
fn a_match_enriched_from_its_own_target_consults_it_once() {
    let (_dir, db, observer) = setup();
    run_on(
        &db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person@g) RETURN a, b LIMIT 10",
        "g",
    );
    assert_once_each(&observer, "MATCH enriched from its own target");
}

#[test]
fn select_edges_consults_its_collection_once() {
    let (_dir, db, observer) = setup();
    run(&db, "SELECT EDGES FROM g");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("g".to_string(), 1)]),
    );
}

#[test]
fn describe_collection_consults_it_once() {
    let (_dir, db, observer) = setup();
    run(&db, "DESCRIBE COLLECTION c");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("c".to_string(), 1)]),
    );
}

#[test]
fn an_explain_statement_consults_each_collection_once() {
    let (_dir, db, observer) = setup();
    run(&db, "EXPLAIN SELECT * FROM c WHERE amount > 0 LIMIT 10");
    assert_once_each(&observer, "EXPLAIN statement");
}

#[test]
fn an_explain_analyze_consults_each_collection_once() {
    let (_dir, db, observer) = setup();
    let query = Parser::parse("SELECT * FROM c WHERE amount > 0 LIMIT 10").expect("test: parse");
    db.explain_analyze_query(&query, &HashMap::new())
        .expect("test: explain analyze runs");
    assert_once_each(&observer, "EXPLAIN ANALYZE");
}

#[test]
fn an_explain_analyze_over_a_compound_consults_each_collection_once() {
    let (_dir, db, observer) = setup();
    let query =
        Parser::parse("SELECT * FROM c UNION SELECT * FROM d LIMIT 10").expect("test: parse");
    db.explain_analyze_query(&query, &HashMap::new())
        .expect("test: explain analyze runs");
    assert_once_each(&observer, "EXPLAIN ANALYZE over UNION");
}

#[test]
fn a_plain_select_consults_its_collection_once() {
    let (_dir, db, observer) = setup();
    run(&db, "SELECT * FROM c LIMIT 10");
    assert_eq!(
        observer.per_collection(),
        BTreeMap::from([("c".to_string(), 1)]),
    );
}
