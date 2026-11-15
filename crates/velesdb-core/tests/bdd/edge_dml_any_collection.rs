//! BDD scenarios: VelesQL edge statements (`INSERT EDGE`, `SELECT EDGES`,
//! `DELETE EDGE`) work on every collection type that holds edges.
//!
//! Vector, graph and metadata collections share one edge store, and the Rust
//! API, REST `/relations` and the agent-memory wedge already add edges to all
//! three. The VelesQL executor alone resolved only the graph registry, so
//! `INSERT EDGE INTO docs …` on a vector collection failed with
//! "Collection 'docs' not found" for a collection that existed.
//! `INSERT NODE` stays graph-only (it writes a payload with no vector), but
//! now says why instead of claiming the collection does not exist.

use serde_json::json;
use velesdb_core::{Database, Error, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, result_ids};

/// A 2-dim vector collection `docs` with points 1, 2 and 3 (all `:Doc`).
fn setup_vector_docs(db: &Database) {
    db.create_vector_collection("docs", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create docs");
    let vc = db.get_vector_collection("docs").expect("test: get docs");
    vc.upsert((1..=3).map(|id| {
        Point::new(
            id,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Doc"], "title": format!("doc {id}")})),
        )
    }))
    .expect("test: upsert docs");
}

/// GIVEN a vector collection with three points
/// WHEN `INSERT EDGE INTO docs` links 1 -> 2 and 1 -> 3
/// THEN `SELECT EDGES` lists both, and MATCH traverses them.
#[test]
fn scenario_insert_edge_into_a_vector_collection_is_traversable() {
    let (_dir, db) = create_test_db();
    setup_vector_docs(&db);

    execute_sql(
        &db,
        "INSERT EDGE INTO docs (id = 10, source = 1, target = 2, label = 'CITES');",
    )
    .expect("INSERT EDGE on a vector collection must succeed");
    execute_sql(
        &db,
        "INSERT EDGE INTO docs (id = 11, source = 1, target = 3, label = 'CITES');",
    )
    .expect("second INSERT EDGE");

    let edges = execute_sql(&db, "SELECT EDGES FROM docs WHERE source = 1;")
        .expect("SELECT EDGES on a vector collection must succeed");
    assert_eq!(result_ids(&edges), [10, 11].into_iter().collect());

    let mut params = std::collections::HashMap::new();
    params.insert("_collection".to_string(), json!("docs"));
    let rows = execute_sql_with_params(&db, "MATCH (a)-[:CITES]->(b) RETURN b.title;", &params)
        .expect("MATCH over the inserted edges");
    assert_eq!(result_ids(&rows), [2, 3].into_iter().collect());
}

/// GIVEN a vector collection with an edge inserted through VelesQL
/// WHEN `DELETE EDGE` removes it
/// THEN it reports `deleted: true` and `SELECT EDGES` no longer lists it.
#[test]
fn scenario_delete_edge_from_a_vector_collection() {
    let (_dir, db) = create_test_db();
    setup_vector_docs(&db);
    execute_sql(
        &db,
        "INSERT EDGE INTO docs (id = 10, source = 1, target = 2, label = 'CITES');",
    )
    .expect("INSERT EDGE");

    let deleted = execute_sql(&db, "DELETE EDGE 10 FROM docs;")
        .expect("DELETE EDGE on a vector collection must succeed");
    assert_eq!(
        deleted[0]
            .point
            .payload
            .as_ref()
            .map(|p| p["deleted"].clone()),
        Some(json!(true))
    );
    let edges = execute_sql(&db, "SELECT EDGES FROM docs;").expect("SELECT EDGES");
    assert!(edges.is_empty(), "the deleted edge must be gone");
}

/// GIVEN a metadata-only collection with two rows
/// WHEN `INSERT EDGE` links them
/// THEN `SELECT EDGES` lists the edge.
#[test]
fn scenario_insert_edge_into_a_metadata_collection() {
    let (_dir, db) = create_test_db();
    db.create_metadata_collection("people")
        .expect("test: create people");
    let mc = db
        .get_metadata_collection("people")
        .expect("test: get people");
    mc.upsert([
        Point::metadata_only(1, json!({"name": "Ada"})),
        Point::metadata_only(2, json!({"name": "Grace"})),
    ])
    .expect("test: upsert people");

    execute_sql(
        &db,
        "INSERT EDGE INTO people (id = 7, source = 1, target = 2, label = 'KNOWS');",
    )
    .expect("INSERT EDGE on a metadata collection must succeed");
    let edges = execute_sql(&db, "SELECT EDGES FROM people WHERE label = 'KNOWS';")
        .expect("SELECT EDGES on a metadata collection");
    assert_eq!(result_ids(&edges), [7].into_iter().collect());
}

/// GIVEN a vector collection
/// WHEN `INSERT NODE` targets it
/// THEN the error says a graph collection is required — not that `docs`
///      does not exist.
#[test]
fn scenario_insert_node_into_a_vector_collection_names_the_real_reason() {
    let (_dir, db) = create_test_db();
    setup_vector_docs(&db);

    let err = execute_sql(&db, "INSERT NODE INTO docs (id = 9, payload = '{}');")
        .expect_err("INSERT NODE stays graph-only");
    assert!(
        !matches!(err, Error::CollectionNotFound(_)),
        "docs exists; got {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains("docs") && message.contains("graph collection"),
        "the error must name the collection and the requirement: {message}"
    );
}

/// GIVEN no collection named `ghost`
/// WHEN an edge statement targets it
/// THEN it is still `CollectionNotFound`.
#[test]
fn scenario_edge_statements_on_a_missing_collection_are_not_found() {
    let (_dir, db) = create_test_db();

    for sql in [
        "INSERT EDGE INTO ghost (source = 1, target = 2, label = 'X');",
        "SELECT EDGES FROM ghost;",
        "DELETE EDGE 1 FROM ghost;",
        "INSERT NODE INTO ghost (id = 1, payload = '{}');",
    ] {
        let err = execute_sql(&db, sql).expect_err("missing collection");
        assert!(
            matches!(err, Error::CollectionNotFound(ref name) if name == "ghost"),
            "{sql}: expected CollectionNotFound(ghost), got {err:?}"
        );
    }
}
