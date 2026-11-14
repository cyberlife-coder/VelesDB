//! BDD tests for N-hop MATCH path expansion in the WASM executor.
//!
//! Before: the executor hard-coded 0-, 1- and 2-hop shapes and rejected
//! anything longer, so the browser build could not run a query the native
//! build answers. These scenarios pin the general behaviour, including the
//! Cypher relationship-isomorphism rule core already enforces (an edge is
//! traversed at most once per matched path).

use velesdb_core::velesql::{InsertEdgeStatement, InsertNodeStatement, Parser};

use crate::database::DatabaseInner;
use crate::velesql_match::execute_match;
use crate::velesql_value::Params;

fn node(db: &mut DatabaseInner, id: u64, name: &str, label: &str) {
    let stmt = InsertNodeStatement {
        collection: "graph".to_string(),
        node_id: id,
        payload: serde_json::json!({"name": name, "labels": [label]}),
    };
    crate::velesql_graph::insert_node(db, &stmt).expect("test: insert node");
}

fn edge(db: &mut DatabaseInner, source: u64, target: u64, label: &str) {
    let stmt = InsertEdgeStatement {
        collection: "graph".to_string(),
        edge_id: None,
        source,
        target,
        label: label.to_string(),
        properties: Vec::new(),
    };
    crate::velesql_graph::insert_edge(db, &stmt, &Params::new()).expect("test: insert edge");
}

/// `1 -R-> 2 -R-> 3 -R-> 4 -R-> 5`, all labelled `P`.
fn chain_of_five(db: &mut DatabaseInner) {
    for id in 1..=5 {
        node(db, id, &format!("n{id}"), "P");
    }
    for id in 1..5 {
        edge(db, id, id + 1, "R");
    }
}

fn rows(db: &mut DatabaseInner, sql: &str) -> Vec<serde_json::Value> {
    let query = Parser::parse(sql).expect("test: parse");
    execute_match(db, &query, &Params::new())
        .expect("test: match")
        .iter()
        .map(|row| serde_json::from_str(row.data_json_ref()).expect("test: row json"))
        .collect()
}

fn names(row: &serde_json::Value, aliases: &[&str]) -> Vec<String> {
    aliases
        .iter()
        .map(|alias| row[*alias]["name"].as_str().expect("test: name").to_owned())
        .collect()
}

#[test]
fn given_a_chain_when_matching_three_hops_then_every_path_binds_four_aliases() {
    let mut db = DatabaseInner::new();
    chain_of_five(&mut db);

    let found = rows(
        &mut db,
        "MATCH (a:P)-[:R]->(b:P)-[:R]->(c:P)-[:R]->(d:P) RETURN a, b, c, d LIMIT 10",
    );

    // Without ORDER BY the row order is unspecified: compare as a set.
    let mut paths: Vec<Vec<String>> = found
        .iter()
        .map(|r| names(r, &["a", "b", "c", "d"]))
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        vec![vec!["n1", "n2", "n3", "n4"], vec!["n2", "n3", "n4", "n5"],]
    );
}

#[test]
fn given_a_chain_when_matching_four_hops_then_the_single_full_path_is_returned() {
    let mut db = DatabaseInner::new();
    chain_of_five(&mut db);

    let found = rows(
        &mut db,
        "MATCH (a:P)-[:R]->(b:P)-[:R]->(c:P)-[:R]->(d:P)-[:R]->(e:P) RETURN a, e LIMIT 10",
    );

    assert_eq!(found.len(), 1);
    assert_eq!(names(&found[0], &["a", "e"]), vec!["n1", "n5"]);
}

#[test]
fn given_a_three_hop_pattern_when_where_targets_the_last_node_then_it_filters() {
    let mut db = DatabaseInner::new();
    chain_of_five(&mut db);

    let found = rows(
        &mut db,
        "MATCH (a:P)-[:R]->(b:P)-[:R]->(c:P)-[:R]->(d:P) WHERE d.name = 'n5' RETURN a, d LIMIT 10",
    );

    assert_eq!(found.len(), 1);
    assert_eq!(names(&found[0], &["a", "d"]), vec!["n2", "n5"]);
}

#[test]
fn given_unaliased_nodes_when_matching_four_nodes_then_aliases_follow_position() {
    let mut db = DatabaseInner::new();
    chain_of_five(&mut db);

    let found = rows(
        &mut db,
        "MATCH (a:P)-[:R]->(:P)-[:R]->(:P)-[:R]->(:P) WHERE a.name = 'n1' RETURN * LIMIT 10",
    );

    assert_eq!(found.len(), 1);
    assert_eq!(
        names(&found[0], &["a", "b", "c", "d"]),
        vec!["n1", "n2", "n3", "n4"]
    );
}

#[test]
fn given_an_undirected_pattern_when_matching_then_an_edge_is_never_reused_in_a_path() {
    // One edge 1-2: `(a)-[:R]-(b)-[:R]-(c)` would bind a=1, b=2, c=1 by walking
    // the same edge back. Core forbids that (relationship isomorphism); so must
    // the browser build.
    let mut db = DatabaseInner::new();
    node(&mut db, 1, "n1", "P");
    node(&mut db, 2, "n2", "P");
    edge(&mut db, 1, 2, "R");

    let found = rows(
        &mut db,
        "MATCH (a:P)-[:R]-(b:P)-[:R]-(c:P) RETURN a, b, c LIMIT 10",
    );

    assert!(found.is_empty(), "an edge was traversed twice: {found:?}");
}

#[test]
fn given_a_variable_length_relationship_when_matching_then_it_is_rejected_not_misread() {
    // `[*1..3]` used to be executed as a single hop, silently returning the
    // wrong paths. Until ranges are implemented here, refuse them.
    let mut db = DatabaseInner::new();
    chain_of_five(&mut db);
    let query =
        Parser::parse("MATCH (a:P)-[:R*1..3]->(b:P) RETURN a, b LIMIT 10").expect("test: parse");

    let err = execute_match(&mut db, &query, &Params::new()).expect_err("test: rejected");

    assert!(err.contains("Variable-length"), "unexpected error: {err}");
}
