//! Edge ids leave `VelesQL` as decimal strings.
//!
//! Edge ids are `u64`, and a derived one (hashed from source, target and
//! label) is routinely above 2^53, where a JSON number loses digits in every
//! JavaScript reader. Every row that names an edge — `SELECT EDGES`,
//! `DELETE EDGE`, and a `MATCH` row's relationship aliases — carries it as
//! its exact decimal string, as the REST graph endpoints already do.
#![cfg(feature = "persistence")]

use std::collections::HashMap;

use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_core::collection::graph::GraphEdge;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point};

/// 2^53 + 1: the first integer a JavaScript number cannot hold.
const HASHED: u64 = (1_u64 << 53) + 1;
const HASHED_STR: &str = "9007199254740993";
const SECOND: u64 = HASHED + 2;
const SECOND_STR: &str = "9007199254740995";

/// Given a graph `g` with `1 -KNOWS(HASHED)-> 2 -KNOWS(SECOND)-> 3`.
fn graph() -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("g", 2, DistanceMetric::Cosine)
        .expect("create g");
    let g = db.get_vector_collection("g").expect("get g");
    g.upsert(
        (1..=3)
            .map(|id| Point::new(id, vec![1.0, 0.0], Some(json!({"_labels": ["P"]}))))
            .collect::<Vec<_>>(),
    )
    .expect("upsert");
    g.add_edge(GraphEdge::new(HASHED, 1, 2, "KNOWS").expect("edge"))
        .expect("add edge");
    g.add_edge(GraphEdge::new(SECOND, 2, 3, "KNOWS").expect("edge"))
        .expect("add edge");
    (dir, db)
}

fn rows(db: &Database, sql: &str) -> Vec<Value> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    let params = HashMap::from([("_collection".to_string(), json!("g"))]);
    db.execute_query(&query, &params)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .into_iter()
        .map(|r| r.point.payload.unwrap_or(Value::Null))
        .collect()
}

#[test]
fn select_edges_names_each_edge_by_its_exact_string() {
    let (_dir, db) = graph();
    let mut ids: Vec<Value> = rows(&db, "SELECT EDGES FROM g")
        .into_iter()
        .map(|row| row["edge_id"].clone())
        .collect();
    ids.sort_by_key(ToString::to_string);
    assert_eq!(ids, vec![json!(HASHED_STR), json!(SECOND_STR)]);
}

#[test]
fn delete_edge_answers_the_id_it_was_given_as_a_string() {
    let (_dir, db) = graph();
    let rows = rows(&db, &format!("DELETE EDGE {HASHED} FROM g"));
    assert_eq!(rows[0]["edge_id"], HASHED_STR);
    assert_eq!(rows[0]["deleted"], true);
}

#[test]
fn a_match_row_names_its_fixed_relationship_by_string() {
    let (_dir, db) = graph();
    let rows = rows(
        &db,
        &format!("MATCH (a)-[r:KNOWS]->(b) WHERE r.id = {HASHED} RETURN r.id"),
    );
    assert_eq!(
        rows.len(),
        1,
        "a numeric WHERE on r.id still matches: {rows:?}"
    );
    assert_eq!(rows[0]["r.id"], HASHED_STR);
    assert_eq!(rows[0]["_edge_bindings"]["r"], HASHED_STR);
    assert_eq!(rows[0]["_bindings"]["a"], 1, "node ids are not edge ids");
}

#[test]
fn a_match_row_names_a_variable_length_path_by_strings() {
    let (_dir, db) = graph();
    let rows = rows(&db, "MATCH (a)-[r:KNOWS*2..2]->(b) RETURN r, r.id");
    assert_eq!(rows.len(), 1, "{rows:?}");
    let path = json!([HASHED_STR, SECOND_STR]);
    assert_eq!(rows[0]["r"], path);
    assert_eq!(rows[0]["r.id"], path);
    assert_eq!(rows[0]["_edge_paths"]["r"], path);
}
