//! BDD scenarios for the edge API of a `WasmDatabase` (`addEdge`,
//! `getEdges`, `removeEdge`, `edgeCount`): REST `/collections/{name}/graph`
//! semantics over the graph store a MATCH on that collection reads.

use serde_json::json;

use crate::database::DatabaseInner;
use crate::database_graph::{parse_edge_id, EdgeInput};
use crate::velesql_match_core::execute_match_rows;

/// A metadata collection `docs` holding points 1, 2 and 3.
fn docs() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs")
        .expect("test: create docs");
    let store = db.get_shared_store("docs").expect("test: docs store");
    for id in 1..=3u64 {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &[],
            Some(json!({"title": format!("doc {id}")})),
        );
    }
    db
}

fn edge(id: u64, source: u64, target: u64, label: &str) -> EdgeInput {
    EdgeInput {
        id: Some(id),
        source,
        target,
        label: label.to_string(),
        properties: None,
    }
}

#[test]
fn an_added_edge_is_listed_counted_and_matched() {
    let mut db = docs();
    let mut cites = edge(10, 1, 2, "CITES");
    cites.properties = Some(json!({"weight": 0.5, "tags": ["a", "b"]}));
    assert_eq!(db.add_edge("docs", cites).expect("add"), 10);

    assert_eq!(db.edge_count("docs").expect("count"), 1);
    assert_eq!(
        serde_json::to_value(db.edges("docs", None).expect("edges")).expect("json"),
        json!([{
            "id": "10", "source": "1", "target": "2", "label": "CITES",
            "properties": {"weight": 0.5, "tags": ["a", "b"]},
        }])
    );
    let rows = execute_match_rows(
        &mut db,
        "MATCH (a)-[r:CITES]->(b) RETURN b.title, r.tags",
        Some(r#"{"_collection": "docs"}"#),
        None,
    )
    .expect("MATCH");
    assert_eq!(
        rows[0]["projected"],
        json!({"b.title": "doc 2", "r.tags": ["a", "b"]})
    );
}

#[test]
fn edges_filter_by_label() {
    let mut db = docs();
    db.add_edge("docs", edge(10, 1, 2, "CITES")).expect("add");
    db.add_edge("docs", edge(11, 1, 3, "LINKS")).expect("add");

    let links = db.edges("docs", Some("LINKS")).expect("edges");
    assert_eq!(links.iter().map(|e| e.id).collect::<Vec<_>>(), [11]);
}

#[test]
fn an_edge_id_is_derived_when_omitted() {
    let mut db = docs();
    let mut e = edge(0, 1, 2, "CITES");
    e.id = None;
    let id = db.add_edge("docs", e).expect("add");
    assert_eq!(id, velesdb_core::hash_edge_id(1, 2, "CITES"));
}

#[test]
fn an_edge_to_a_missing_node_is_refused() {
    let mut db = docs();
    let err = db
        .add_edge("docs", edge(10, 1, 99, "CITES"))
        .expect_err("node 99 does not exist");
    assert!(err.to_string().contains("99"), "{err}");
    assert_eq!(db.edge_count("docs").expect("count"), 0);
}

#[test]
fn an_edge_on_a_missing_collection_is_refused() {
    let mut db = DatabaseInner::new();
    let err = db
        .add_edge("ghost", edge(1, 1, 2, "R"))
        .expect_err("no collection");
    assert!(err.to_string().contains("ghost"), "{err}");
    assert!(db.edges("ghost", None).is_err());
    assert!(db.edge_count("ghost").is_err());
}

#[test]
fn a_removed_edge_is_gone_and_removing_twice_reports_false() {
    let mut db = docs();
    db.add_edge("docs", edge(10, 1, 2, "CITES")).expect("add");

    assert!(db.remove_edge("docs", 10).expect("remove"));
    assert!(!db.remove_edge("docs", 10).expect("remove again"));
    assert_eq!(db.edge_count("docs").expect("count"), 0);
}

#[test]
fn a_collection_without_edges_has_none() {
    let db = docs();
    assert!(db.edges("docs", None).expect("edges").is_empty());
    assert_eq!(db.edge_count("docs").expect("count"), 0);
}

/// 2^53 + 1: the first integer a JavaScript number cannot hold.
const HASHED: u64 = (1_u64 << 53) + 1;

#[test]
fn a_hashed_edge_id_is_listed_as_its_exact_string() {
    let mut db = docs();
    db.add_edge("docs", edge(HASHED, 1, 2, "CITES"))
        .expect("add");
    let listed = serde_json::to_value(db.edges("docs", None).expect("edges")).expect("json");
    assert_eq!(listed[0]["id"], "9007199254740993");
}

#[test]
fn an_edge_id_is_sent_as_a_string_or_a_number() {
    let as_string: EdgeInput = serde_json::from_value(
        json!({"id": "9007199254740993", "source": 1, "target": 2, "label": "L"}),
    )
    .expect("a string id");
    assert_eq!(as_string.id, Some(HASHED));
    let as_number: EdgeInput =
        serde_json::from_value(json!({"id": 7, "source": 1, "target": 2, "label": "L"}))
            .expect("a number id");
    assert_eq!(as_number.id, Some(7));
    let absent: EdgeInput =
        serde_json::from_value(json!({"source": 1, "target": 2, "label": "L"})).expect("no id");
    assert_eq!(absent.id, None);
}

/// The ends of an edge read as REST's do: a node id past 2^53 arrives as
/// its decimal string from a JavaScript client.
#[test]
fn an_edges_ends_are_sent_as_strings_or_numbers() {
    let edge: EdgeInput =
        serde_json::from_value(json!({"source": "9007199254740993", "target": 2, "label": "L"}))
            .expect("a string source");
    assert_eq!((edge.source, edge.target), (HASHED, 2));
}

#[test]
fn an_edge_id_argument_is_a_decimal_u64() {
    assert_eq!(parse_edge_id("9007199254740993").ok(), Some(HASHED));
    for bad in ["", "x", "-1", "1.5", "18446744073709551616"] {
        assert!(parse_edge_id(bad).is_err(), "{bad:?} is not an edge id");
    }
}

#[test]
fn a_match_returns_a_relationship_id_as_a_string() {
    let mut db = docs();
    db.add_edge("docs", edge(HASHED, 1, 2, "CITES"))
        .expect("add");
    let rows = execute_match_rows(
        &mut db,
        "MATCH (a)-[r:CITES]->(b) RETURN r.id, b.id",
        Some(r#"{"_collection": "docs"}"#),
        None,
    )
    .expect("MATCH");
    assert_eq!(
        rows[0]["projected"],
        json!({"r.id": "9007199254740993", "b.id": 2}),
        "an edge id is a string, a node id stays a number"
    );
}

/// As REST's `EdgeResponse`: an edge's ends are node ids, listed as decimal
/// strings, so a node id past 2^53 reads back as the node it named.
#[test]
fn an_edges_ends_are_listed_as_exact_strings() {
    let mut db = docs();
    db.add_edge("docs", edge(10, 1, 2, "CITES")).expect("add");
    let listed = serde_json::to_value(db.edges("docs", None).expect("edges")).expect("json");
    assert_eq!(
        (&listed[0]["source"], &listed[0]["target"]),
        (&json!("1"), &json!("2"))
    );
}

/// As core's `Collection::delete` (#900): removing a point removes the edges
/// it is an end of, so a later point under the same id starts unconnected.
#[test]
fn removing_points_removes_the_edges_they_end() {
    let mut db = docs();
    db.add_edge("docs", edge(10, 1, 2, "CITES")).expect("add");
    db.add_edge("docs", edge(11, 3, 1, "CITES")).expect("add");
    db.add_edge("docs", edge(12, 2, 3, "CITES")).expect("add");

    assert_eq!(db.remove_points("docs", &[1, 99]).expect("remove"), 1);

    let left: Vec<u64> = db
        .edges("docs", None)
        .expect("edges")
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(left, [12], "only the edge not touching point 1 is left");
    assert!(db
        .get_shared_store("docs")
        .expect("store")
        .borrow()
        .ids
        .iter()
        .all(|&id| id != 1));
}

#[test]
fn removing_points_of_a_missing_collection_is_an_error() {
    let mut db = docs();
    assert!(db.remove_points("ghost", &[1]).is_err());
}
