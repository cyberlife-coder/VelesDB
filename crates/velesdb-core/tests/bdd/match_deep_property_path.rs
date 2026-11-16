//! BDD scenarios: a MATCH reads a node property path as deep as it is
//! written, as SELECT's clauses and the browser read it.
//!
//! VelesQL's `RETURN` takes one property segment (`a.name`), but REST
//! `/match` sends the clause as JSON, whose RETURN expression can name a
//! nested path. The MATCH RETURN reader refused any path deeper than 10
//! segments, so that row came back without the value while `WHERE` on the
//! same path matched the node.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::velesql::{MatchClause, Parser, ReturnItem};
use velesdb_core::{Database, SearchResult};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params};

const DEPTH: usize = 12;

/// `kg` (graph): node 1 `:Doc` whose payload nests `k` 12 levels deep down
/// to `"leaf"`.
fn setup(db: &Database) {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE kg");
    let mut nested = json!("leaf");
    for _ in 0..DEPTH {
        nested = json!({ "k": nested });
    }
    let mut payload = nested;
    payload["_labels"] = json!(["Doc"]);
    db.get_graph_collection("kg")
        .expect("test: kg")
        .upsert_node_payload(1, &payload)
        .expect("test: node 1");
}

fn deep_path() -> String {
    ["k"; DEPTH].join(".")
}

fn run(db: &Database, sql: &str) -> Vec<SearchResult> {
    let params = HashMap::from([("_collection".to_string(), json!("kg"))]);
    execute_sql_with_params(db, sql, &params).expect("test: MATCH")
}

/// The clause REST `/match` would send for `MATCH (a:Doc) RETURN a.<path>`.
fn returning(path: &str) -> MatchClause {
    let mut clause = Parser::parse("MATCH (a:Doc) RETURN a")
        .expect("test: parse")
        .match_clause
        .expect("test: MATCH clause");
    clause.return_clause.items = vec![ReturnItem {
        expression: format!("a.{path}"),
        alias: None,
    }];
    clause
}

/// GIVEN a node with a 12-segment property path
/// WHEN a `/match` clause returns that path
/// THEN the row carries the value.
#[test]
fn return_projects_a_path_of_any_depth() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let path = deep_path();
    let rows = db
        .match_query("kg", &returning(&path), &HashMap::new(), None)
        .expect("test: match_query");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].projected.get(format!("a.{path}").as_str()),
        Some(&json!("leaf"))
    );
}

/// GIVEN the same node
/// WHEN WHERE compares that path
/// THEN the node matches, and a different value does not.
#[test]
fn where_reads_a_path_of_any_depth() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let path = deep_path();
    let hit = run(
        &db,
        &format!("MATCH (a:Doc) WHERE a.{path} = 'leaf' RETURN a"),
    );
    assert_eq!(hit.len(), 1);
    let miss = run(
        &db,
        &format!("MATCH (a:Doc) WHERE a.{path} = 'other' RETURN a"),
    );
    assert!(miss.is_empty());
}
