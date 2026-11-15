//! BDD scenarios for the edge API of a `WasmDatabase` (`addEdge`,
//! `getEdges`, `removeEdge`, `edgeCount`): REST `/collections/{name}/graph`
//! semantics over the graph store a MATCH on that collection reads.

use serde_json::json;

use crate::database::DatabaseInner;
use crate::database_graph::EdgeInput;
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
            "id": 10, "source": 1, "target": 2, "label": "CITES",
            "properties": {"weight": 0.5, "tags": ["a", "b"]},
        }])
    );
    let rows = execute_match_rows(
        &mut db,
        "MATCH (a)-[r:CITES]->(b) RETURN b.title, r.tags",
        Some(r#"{"_collection": "docs"}"#),
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
    assert!(err.contains("99"), "{err}");
    assert_eq!(db.edge_count("docs").expect("count"), 0);
}

#[test]
fn an_edge_on_a_missing_collection_is_refused() {
    let mut db = DatabaseInner::new();
    let err = db
        .add_edge("ghost", edge(1, 1, 2, "R"))
        .expect_err("no collection");
    assert!(err.contains("ghost"), "{err}");
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
