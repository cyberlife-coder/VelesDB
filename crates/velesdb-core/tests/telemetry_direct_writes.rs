//! Core-invoked upsert telemetry on the programmatic collection API.
//!
//! A database opened with an observer reports every completed write through
//! `DatabaseObserver::on_upsert` — not only the `VelesQL` DML path, but every
//! write a binding or server handler makes on a collection handle it got from
//! that database. Before this, those writes reached the observer only if the
//! caller remembered the deprecated `Database::notify_upsert` shim, and the
//! shim's callers (REST handlers, the Python binding) each re-implemented the
//! count.
//!
//! Each case asserts exactly one `on_upsert` with the target collection and
//! the exact point count, so a path that fires twice (an entry point that
//! delegates to another entry point) fails as surely as one that never fires.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, Database, DatabaseObserver, DistanceMetric, GraphSchema, Point,
};

/// Records `(collection, point_count)` for every `on_upsert`.
#[derive(Default)]
struct UpsertSpy {
    upserts: Mutex<Vec<(String, usize)>>,
}

impl UpsertSpy {
    fn calls(&self) -> Vec<(String, usize)> {
        self.upserts.lock().clone()
    }

    /// Forgets what setup wrote, so a case observes only the write it tests.
    fn reset(&self) {
        self.upserts.lock().clear();
    }
}

impl DatabaseObserver for UpsertSpy {
    fn on_upsert(&self, collection: &str, point_count: usize) {
        self.upserts
            .lock()
            .push((collection.to_string(), point_count));
    }
}

fn open_db() -> (TempDir, Database, Arc<UpsertSpy>) {
    let spy = Arc::new(UpsertSpy::default());
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), spy.clone() as Arc<dyn DatabaseObserver>)
        .expect("test: open db with observer");
    (dir, db, spy)
}

fn vector_point(id: u64) -> Point {
    Point::new(id, vec![1.0, 0.5, 0.0, 0.0], Some(json!({"n": id})))
}

fn vector_db() -> (TempDir, Database, Arc<UpsertSpy>) {
    let (dir, db, spy) = open_db();
    db.create_vector_collection("docs", 4, DistanceMetric::Cosine)
        .expect("test: create docs");
    (dir, db, spy)
}

fn assert_single_upsert(spy: &UpsertSpy, collection: &str, point_count: usize) {
    assert_eq!(
        spy.calls(),
        vec![(collection.to_string(), point_count)],
        "exactly one on_upsert with the target collection and the exact count"
    );
}

#[test]
fn vector_upsert_reports_once() {
    let (_dir, db, spy) = vector_db();
    let docs = db.get_vector_collection("docs").expect("test: docs");
    docs.upsert((1..=3).map(vector_point))
        .expect("test: upsert");
    assert_single_upsert(&spy, "docs", 3);
}

#[test]
fn vector_upsert_bulk_reports_once() {
    let (_dir, db, spy) = vector_db();
    let docs = db.get_vector_collection("docs").expect("test: docs");
    let points: Vec<Point> = (1..=4).map(vector_point).collect();
    docs.upsert_bulk(&points).expect("test: upsert_bulk");
    assert_single_upsert(&spy, "docs", 4);
}

#[test]
fn vector_upsert_bulk_from_raw_reports_once() {
    let (_dir, db, spy) = vector_db();
    let docs = db.get_vector_collection("docs").expect("test: docs");
    let vectors = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    docs.upsert_bulk_from_raw(&vectors, &[1, 2], 4, None)
        .expect("test: upsert_bulk_from_raw");
    assert_single_upsert(&spy, "docs", 2);
}

#[test]
fn metadata_upsert_reports_once() {
    let (_dir, db, spy) = open_db();
    db.create_metadata_collection("notes")
        .expect("test: create notes");
    let notes = db.get_metadata_collection("notes").expect("test: notes");
    notes
        .upsert((1..=2).map(|id| Point::metadata_only(id, json!({"id": id}))))
        .expect("test: upsert");
    assert_single_upsert(&spy, "notes", 2);
}

#[test]
fn graph_node_payload_writes_report_once_each() {
    let (_dir, db, spy) = open_db();
    db.create_graph_collection("kg", GraphSchema::schemaless())
        .expect("test: create kg");
    let kg = db.get_graph_collection("kg").expect("test: kg");

    kg.upsert_node_payload(1, &json!({"name": "a"}))
        .expect("test: upsert_node_payload");
    assert_single_upsert(&spy, "kg", 1);

    spy.reset();
    let (b, c) = (json!({"name": "b"}), json!({"name": "c"}));
    kg.upsert_node_payloads(&[(2, &b), (3, &c)])
        .expect("test: upsert_node_payloads");
    assert_single_upsert(&spy, "kg", 2);

    spy.reset();
    kg.upsert_node(4, &json!({"name": "d"}), None)
        .expect("test: upsert_node");
    assert_single_upsert(&spy, "kg", 1);
}

#[test]
fn graph_node_with_embedding_reports_once() {
    let (_dir, db, spy) = open_db();
    db.create_graph_collection_with_embeddings(
        "kg",
        GraphSchema::schemaless(),
        4,
        DistanceMetric::Cosine,
    )
    .expect("test: create kg");
    let kg = db.get_graph_collection("kg").expect("test: kg");
    kg.upsert_node(1, &json!({"name": "a"}), Some(vec![1.0, 0.0, 0.0, 0.0]))
        .expect("test: upsert_node");
    assert_single_upsert(&spy, "kg", 1);
}

#[test]
fn velesql_insert_and_update_are_not_double_counted() {
    let (_dir, db, spy) = open_db();
    db.create_metadata_collection("sink")
        .expect("test: create sink");

    let insert = Parser::parse("INSERT INTO sink (id, name) VALUES (1, 'a'), (2, 'b')")
        .expect("test: parse INSERT");
    db.execute_query(&insert, &HashMap::new())
        .expect("test: INSERT");
    assert_single_upsert(&spy, "sink", 2);

    spy.reset();
    let update =
        Parser::parse("UPDATE sink SET name = 'z' WHERE id = 1").expect("test: parse UPDATE");
    db.execute_query(&update, &HashMap::new())
        .expect("test: UPDATE");
    assert_single_upsert(&spy, "sink", 1);
}

#[test]
fn an_empty_write_reports_nothing() {
    let (_dir, db, spy) = vector_db();
    let docs = db.get_vector_collection("docs").expect("test: docs");
    docs.upsert(Vec::new()).expect("test: empty upsert");
    docs.upsert_bulk(&[]).expect("test: empty upsert_bulk");
    assert!(spy.calls().is_empty(), "no point written, no on_upsert");
}

#[test]
fn a_rejected_write_reports_nothing() {
    let (_dir, db, spy) = vector_db();
    let docs = db.get_vector_collection("docs").expect("test: docs");
    let wrong_dimension = Point::new(1, vec![1.0, 0.0], None);
    assert!(docs.upsert([wrong_dimension]).is_err());
    assert!(
        spy.calls().is_empty(),
        "a failed write must not be reported"
    );
}
