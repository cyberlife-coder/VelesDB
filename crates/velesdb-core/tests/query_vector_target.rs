#![cfg(feature = "persistence")]
//! A statement's query vectors are checked against its collection once,
//! before any plan is chosen or any row is read (`velesql::QueryVectorTarget`):
//! a collection that stores no vector refuses with `SearchNotSupported`
//! (VELES-015), one of another dimension with `DimensionMismatch`
//! (VELES-004). Whether a statement fails used to depend on the planner's
//! strategy, the operator, the leaf's place under `NOT`/`OR`, or whether the
//! walk found a row. The wrong-dimension SELECT and MATCH shapes are pinned
//! for both engines in `conformance/velesql_executor_cases.json` (R022-R029).

use std::collections::HashMap;

use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, MatchScoring};

/// `docs` (2-d, holding a point), `tags` (metadata), `kg` (a graph without
/// embeddings).
fn db() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    for sql in [
        "CREATE COLLECTION docs (dimension = 2)",
        "CREATE METADATA COLLECTION tags",
        "INSERT INTO tags (id, name) VALUES (1, 'x')",
        "CREATE GRAPH COLLECTION kg",
        "INSERT NODE INTO kg (id = 1, payload = '{\"name\":\"x\"}')",
    ] {
        execute(&db, sql, &HashMap::new()).expect("test: setup");
    }
    execute(
        &db,
        "INSERT INTO docs (id, vector, name) VALUES (1, $v, 'a')",
        &HashMap::from([("v".to_string(), json!([1.0, 0.0]))]),
    )
    .expect("test: setup");
    (dir, db)
}

fn execute(
    db: &Database,
    sql: &str,
    params: &HashMap<String, Value>,
) -> velesdb_core::Result<Vec<velesdb_core::SearchResult>> {
    db.execute_query(&Parser::parse(sql).expect("test: parse"), params)
}

fn params(collection: Option<&str>, q: &[f32]) -> HashMap<String, Value> {
    let mut params = HashMap::from([("q".to_string(), json!(q))]);
    if let Some(name) = collection {
        params.insert("_collection".to_string(), json!(name));
    }
    params
}

fn assert_code(result: velesdb_core::Result<impl std::fmt::Debug>, code: &str, context: &str) {
    let err = result.expect_err(context);
    assert_eq!(err.code(), code, "{context}: {err}");
}

#[test]
fn a_collection_without_vectors_refuses_every_select_query_vector() {
    let (_dir, db) = db();
    for collection in ["tags", "kg"] {
        for shape in [
            "WHERE similarity(vector, $q) > 0.1 LIMIT 5",
            "WHERE similarity(vector, $q) < 0.1 LIMIT 5",
            "WHERE NOT similarity(vector, $q) > 0.1 LIMIT 5",
            "WHERE similarity(vector, $q) > 0.1 OR name = 'x' LIMIT 5",
            "ORDER BY similarity(vector, $q) DESC LIMIT 5",
            "WHERE vector NEAR $q LIMIT 5",
            "WHERE vector NEAR_FUSED [$q, $q] LIMIT 5",
        ] {
            let sql = format!("SELECT * FROM {collection} {shape}");
            assert_code(
                execute(&db, &sql, &params(None, &[1.0, 0.0])),
                "VELES-015",
                &sql,
            );
        }
    }
}

#[test]
fn a_named_vector_is_read_row_by_row_not_against_the_collection() {
    let (_dir, db) = db();
    // `embedding` is a payload array: a row without one is UNKNOWN for the
    // leaf, so the metadata collection answers with no row, not a refusal.
    let rows = execute(
        &db,
        "SELECT * FROM tags WHERE similarity(embedding, $q) > 0.1 LIMIT 5",
        &params(None, &[1.0, 0.0]),
    )
    .expect("test: a named vector is no collection query vector");
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn an_aggregate_checks_its_query_vectors_too() {
    let (_dir, db) = db();
    for (sql, q, code) in [
        (
            "SELECT COUNT(*) FROM docs WHERE similarity(vector, $q) < 0.1",
            &[1.0_f32, 0.0, 0.0][..],
            "VELES-004",
        ),
        (
            "SELECT COUNT(*) FROM tags WHERE NOT similarity(vector, $q) > 0.1",
            &[1.0, 0.0][..],
            "VELES-015",
        ),
    ] {
        let query = Parser::parse(sql).expect("test: parse");
        assert_code(db.execute_aggregate(&query, &params(None, q)), code, sql);
    }
}

#[test]
fn a_match_on_a_collection_without_vectors_refuses_whatever_the_plan() {
    let (_dir, db) = db();
    for collection in ["tags", "kg"] {
        for sql in [
            "MATCH (a) WHERE similarity(a.vector, $q) > 0.1 RETURN a",
            "MATCH (a) WHERE similarity(a.vector, $q) > 0.1 RETURN a LIMIT 3",
            "MATCH (a) RETURN a ORDER BY similarity(a.vector, $q) DESC",
        ] {
            assert_code(
                execute(&db, sql, &params(Some(collection), &[1.0, 0.0])),
                "VELES-015",
                &format!("{collection}: {sql}"),
            );
        }
    }
}

/// REST `/match` with a query vector: checked before the walk, so a walk
/// that finds no row (`[:NOPE]` matches nothing) still refuses.
#[test]
fn a_match_scoring_vector_is_checked_before_the_walk() {
    let (_dir, db) = db();
    let clause = Parser::parse("MATCH (a)-[:NOPE]->(b) RETURN b")
        .expect("test: parse")
        .match_clause
        .expect("test: MATCH");
    for (collection, vector, code) in [
        ("docs", &[1.0_f32, 0.0, 0.0][..], "VELES-004"),
        ("tags", &[1.0, 0.0][..], "VELES-015"),
        ("kg", &[1.0, 0.0][..], "VELES-015"),
    ] {
        let scoring = MatchScoring::new(Some(vector), None).expect("test: scoring");
        assert_code(
            db.match_query(collection, &clause, &HashMap::new(), scoring),
            code,
            collection,
        );
    }
}
