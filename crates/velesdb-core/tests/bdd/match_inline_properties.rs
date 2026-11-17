//! BDD scenarios: a pattern's inline properties mean what the WHERE `=`
//! means.
//!
//! `(n {k: v})` and `-[r {k: v}]->` filter as `n.k = v` / `r.k = v` would:
//! a `$name` takes the caller's value, `NULL` matches nothing (three-valued
//! logic), numbers compare by exact value, and `id` reads the element's id.
//! Before, a `$name` was compared as the literal text `$name` and never
//! matched, `{k: null}` matched a `null` property, and `{id: 3}` read a
//! payload field named `id`.

use std::collections::{HashMap, HashSet};

use serde_json::json;
use velesdb_core::{Database, GraphEdge, SearchResult};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params};

/// `g`: people 1 (Ann, age 30, payload `id: 99`), 2 (Bo, age null),
/// 3 (Cy, age 30.0); KNOWS 1 -> 2 (since 2020), 1 -> 3 (since 2021).
fn setup(db: &Database) {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION g (dimension = 2, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE g");
    let gc = db.get_graph_collection("g").expect("test: g");
    for (id, payload) in [
        (
            1,
            json!({"_labels": ["Person"], "name": "Ann", "age": 30, "id": 99}),
        ),
        (2, json!({"_labels": ["Person"], "name": "Bo", "age": null})),
        (3, json!({"_labels": ["Person"], "name": "Cy", "age": 30.0})),
    ] {
        gc.upsert_node_payload(id, &payload).expect("test: node");
    }
    for (id, target, since) in [(10, 2, 2020), (11, 3, 2021)] {
        let edge = GraphEdge::new(id, 1, target, "KNOWS")
            .expect("test: edge")
            .with_properties(HashMap::from([("since".to_string(), json!(since))]));
        gc.add_edge(edge).expect("test: add edge");
    }
}

fn ids(db: &Database, sql: &str, extra: &[(&str, serde_json::Value)]) -> HashSet<u64> {
    let mut params = HashMap::from([("_collection".to_string(), json!("g"))]);
    params.extend(extra.iter().map(|(k, v)| ((*k).to_string(), v.clone())));
    let rows: Vec<SearchResult> = execute_sql_with_params(db, sql, &params).expect("test: MATCH");
    rows.iter()
        .map(|r| {
            r.point.payload.as_ref().expect("test: payload")["_bindings"]["n"]
                .as_u64()
                .expect("test: n")
        })
        .collect()
}

#[test]
fn a_parameter_takes_the_callers_value() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let got = ids(
        &db,
        "MATCH (n:Person {name: $who}) RETURN n",
        &[("who", json!("Cy"))],
    );
    assert_eq!(got, HashSet::from([3]));
}

#[test]
fn a_missing_parameter_is_refused() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let params = HashMap::from([("_collection".to_string(), json!("g"))]);
    let err = execute_sql_with_params(&db, "MATCH (n:Person {name: $who}) RETURN n", &params)
        .expect_err("test: refused");
    assert!(err.to_string().contains("$who"), "{err}");
}

#[test]
fn null_matches_nothing_and_numbers_compare_exactly() {
    let (_dir, db) = create_test_db();
    setup(&db);
    assert!(ids(&db, "MATCH (n:Person {age: NULL}) RETURN n", &[]).is_empty());
    assert_eq!(
        ids(&db, "MATCH (n:Person {age: 30}) RETURN n", &[]),
        HashSet::from([1, 3])
    );
}

#[test]
fn id_reads_the_node_id() {
    let (_dir, db) = create_test_db();
    setup(&db);
    assert_eq!(
        ids(&db, "MATCH (n:Person {id: 3}) RETURN n", &[]),
        HashSet::from([3])
    );
    assert!(ids(&db, "MATCH (n:Person {id: 99}) RETURN n", &[]).is_empty());
}

#[test]
fn relationship_properties_filter_like_where() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let got = ids(
        &db,
        "MATCH (a:Person)-[:KNOWS {since: $year}]->(n:Person) RETURN n",
        &[("year", json!(2021))],
    );
    assert_eq!(got, HashSet::from([3]));
}
