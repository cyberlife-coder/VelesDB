//! Core reports every completed delete to the observer, as it reports writes.
//!
//! `on_upsert` reached the observer from every surface, but a delete reached
//! nothing: a host mirroring the data (the Tauri plugin's
//! `collection-updated` event, a cache, an audit trail) had to hook each
//! delete command by hand, and missed `VelesQL` `DELETE` entirely.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, AccessDecision, Database, DatabaseObserver, DistanceMetric, Point,
    QueryAccessContext, Result as CoreResult,
};

#[derive(Default)]
struct DeleteSpy {
    deletes: Mutex<Vec<(String, usize)>>,
}

impl DatabaseObserver for DeleteSpy {
    fn on_query_request(&self, _ctx: &QueryAccessContext) -> CoreResult<AccessDecision> {
        Ok(AccessDecision::Allow)
    }

    fn on_delete(&self, collection: &str, point_count: usize) {
        self.deletes
            .lock()
            .push((collection.to_owned(), point_count));
    }
}

fn seeded() -> (TempDir, Database, Arc<DeleteSpy>) {
    let dir = TempDir::new().expect("test: tempdir");
    let spy = Arc::new(DeleteSpy::default());
    let db = Database::open_with_observer(dir.path(), spy.clone()).expect("test: open");
    db.create_vector_collection("docs", 2, DistanceMetric::Cosine)
        .expect("test: create");
    let docs = db.get_vector_collection("docs").expect("test: docs");
    docs.upsert((1..=3).map(|id| Point::new(id, vec![1.0, 0.0], None)))
        .expect("test: seed");
    (dir, db, spy)
}

#[test]
fn a_delete_through_the_collection_is_reported_once() {
    let (_dir, db, spy) = seeded();
    db.get_vector_collection("docs")
        .expect("test: docs")
        .delete(&[1, 2])
        .expect("test: delete");
    assert_eq!(*spy.deletes.lock(), vec![("docs".to_owned(), 2)]);
}

#[test]
fn a_velesql_delete_is_reported_once() {
    let (_dir, db, spy) = seeded();
    let query = Parser::parse("DELETE FROM docs WHERE id = 3").expect("test: parse");
    db.execute_query(&query, &HashMap::new())
        .expect("test: delete");
    assert_eq!(*spy.deletes.lock(), vec![("docs".to_owned(), 1)]);
}

#[test]
fn an_empty_delete_is_not_reported() {
    let (_dir, db, spy) = seeded();
    db.get_vector_collection("docs")
        .expect("test: docs")
        .delete(&[])
        .expect("test: delete");
    assert!(spy.deletes.lock().is_empty());
}
