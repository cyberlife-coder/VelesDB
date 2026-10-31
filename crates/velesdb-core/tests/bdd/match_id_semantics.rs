//! BDD-style end-to-end tests: in a MATCH, `alias.id` is the bound node's (or
//! relationship's) id — in WHERE, `IN`, ORDER BY and RETURN, with or without
//! a property index on `(Label, "id")` — even when its properties carry an
//! `id` of their own, as `id` reads in every other clause.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point, SearchResult};

use super::helpers::create_test_db;

/// Seeds `mdocs`: `:Doc` nodes 1..=4 whose payload `id` is `10 - node id`
/// (9, 8, 7, 6), so reading the payload reverses every id order, and an edge
/// 100 (1 -> 2) whose properties carry `id: 7`.
fn setup_docs(db: &Database) {
    db.create_vector_collection("mdocs", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create mdocs");
    let vc = db.get_vector_collection("mdocs").expect("test: get mdocs");
    let points: Vec<Point> = (1..=4_u64)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.0],
                Some(json!({"_labels": ["Doc"], "id": 10 - id})),
            )
        })
        .collect();
    vc.upsert(points).expect("test: upsert mdocs");
    let mut props = HashMap::new();
    props.insert("id".to_string(), json!(7));
    let edge = GraphEdge::new(100, 1, 2, "CITES")
        .expect("test: edge")
        .with_properties(props);
    vc.add_edge(edge).expect("test: add edge");
}

fn run(db: &Database, sql: &str) -> Vec<SearchResult> {
    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH");
    let params = HashMap::from([("_collection".to_string(), json!("mdocs"))]);
    db.execute_query(&query, &params)
        .expect("test: execute MATCH")
}

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn sorted_ids(results: &[SearchResult]) -> Vec<u64> {
    let mut ids = ids(results);
    ids.sort_unstable();
    ids
}

#[test]
fn test_match_where_alias_id_equality_reads_the_node_id() {
    let (_dir, db) = create_test_db();
    setup_docs(&db);

    let results = run(&db, "MATCH (d:Doc) WHERE d.id = 2 RETURN d");

    assert_eq!(ids(&results), vec![2], "payload ids are 6..=9: none is 2");
}

#[test]
fn test_match_where_alias_id_in_reads_the_node_id() {
    let (_dir, db) = create_test_db();
    setup_docs(&db);

    let results = run(&db, "MATCH (d:Doc) WHERE d.id IN (1, 3) RETURN d");

    assert_eq!(sorted_ids(&results), vec![1, 3]);
}

#[test]
fn test_match_order_by_alias_id_sorts_on_the_node_id() {
    let (_dir, db) = create_test_db();
    setup_docs(&db);

    let results = run(&db, "MATCH (d:Doc) RETURN d ORDER BY d.id DESC");

    assert_eq!(
        ids(&results),
        vec![4, 3, 2, 1],
        "the payload ids would sort 1..=4"
    );
}

#[test]
fn test_match_return_alias_id_projects_the_node_id() {
    let (_dir, db) = create_test_db();
    setup_docs(&db);

    let results = run(&db, "MATCH (d:Doc) WHERE d.id = 3 RETURN d.id");

    let projected = results
        .first()
        .and_then(|r| r.point.payload.as_ref())
        .and_then(|p| p.get("d.id").cloned());
    assert_eq!(projected, Some(json!(3)), "got {results:?}");
}

#[test]
fn test_match_relationship_id_reads_the_edge_id() {
    let (_dir, db) = create_test_db();
    setup_docs(&db);

    let hits = run(
        &db,
        "MATCH (a:Doc)-[r:CITES]->(b) WHERE r.id = 100 RETURN r.id",
    );
    let projected = hits
        .first()
        .and_then(|r| r.point.payload.as_ref())
        .and_then(|p| p.get("r.id").cloned());
    assert_eq!(projected, Some(json!(100)), "got {hits:?}");

    let misses = run(
        &db,
        "MATCH (a:Doc)-[r:CITES]->(b) WHERE r.id = 7 RETURN r.id",
    );
    assert!(misses.is_empty(), "7 is the edge property, not its id");
}

#[test]
fn test_match_alias_id_ignores_a_property_index_on_id() {
    // An index on the `id` property answers for the payload field: used for
    // `d.id`, it would hand over node 8 for `d.id = 2` (payload id 2 is
    // nobody) or the wrong node for `d.id = 3` (payload id 3 is nobody).
    let (_dir, db) = create_test_db();
    setup_docs(&db);
    db.get_vector_collection("mdocs")
        .expect("test: get mdocs")
        .create_range_index("Doc", "id")
        .expect("test: index (Doc, id)");

    assert_eq!(
        ids(&run(&db, "MATCH (d:Doc) WHERE d.id = 3 RETURN d")),
        vec![3]
    );
    assert_eq!(
        sorted_ids(&run(&db, "MATCH (d:Doc) WHERE d.id > 2 RETURN d")),
        vec![3, 4]
    );
    assert!(run(&db, "MATCH (d:Doc) WHERE d.id = 9 RETURN d").is_empty());
}
