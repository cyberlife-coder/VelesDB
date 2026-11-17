//! BDD scenarios: a MATCH over a collection binds that collection's points as
//! nodes, the way core does.
//!
//! In core one collection holds both the points and the edges, and a node is
//! a point: its payload is the node's properties and its `_labels` array the
//! node's labels. The WASM executor kept nodes only in the graph store, so a
//! point inserted through the collection (or a `VectorStore` attached to it)
//! was invisible to MATCH once edges linked it. `INSERT NODE` also read only
//! a `labels` key where core reads `_labels`, so the same statement labelled a
//! node in core and left it unlabelled here.

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

const ON_DOCS: Option<&str> = Some(r#"{"_collection": "docs"}"#);

/// A metadata collection `docs` with three `:Doc` points, and CITES edges
/// 1 -> 2 and 1 -> 3 added through VelesQL.
fn docs_with_citations(db: &mut DatabaseInner) {
    db.create_metadata_collection("docs")
        .expect("test: create docs");
    let store = db.get_shared_store("docs").expect("test: docs store");
    for id in 1..=3u64 {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &[],
            Some(serde_json::json!({"_labels": ["Doc"], "title": format!("doc {id}")})),
        );
    }
    for (source, target) in [(1, 2), (1, 3)] {
        execute(
            db,
            &format!(
                "INSERT EDGE INTO docs (source = {source}, target = {target}, label = 'CITES')"
            ),
            None,
        )
        .expect("test: edge");
    }
}

fn rows(db: &mut DatabaseInner, sql: &str) -> Vec<serde_json::Value> {
    let result = execute(db, sql, ON_DOCS).expect("test: MATCH");
    result
        .rows_ref()
        .iter()
        .map(|row| serde_json::from_str(row.data_json_ref()).expect("test: row json"))
        .collect()
}

/// GIVEN points carrying `_labels` and edges between them
/// WHEN MATCH filters on the label and its RETURN projects the target
/// THEN the points are the nodes, with their payload as properties.
#[test]
fn match_binds_the_collection_points_as_labelled_nodes() {
    let mut db = DatabaseInner::new();
    docs_with_citations(&mut db);

    let mut titles: Vec<String> = rows(&mut db, "MATCH (a:Doc)-[:CITES]->(b:Doc) RETURN a, b")
        .iter()
        .map(|row| {
            row["b"]["title"]
                .as_str()
                .expect("test: b.title")
                .to_string()
        })
        .collect();
    titles.sort();
    assert_eq!(titles, ["doc 2", "doc 3"]);
}

/// GIVEN the same points
/// WHEN WHERE filters on a point property
/// THEN it sees the point payload.
#[test]
fn match_where_reads_point_properties() {
    let mut db = DatabaseInner::new();
    docs_with_citations(&mut db);

    let found = rows(
        &mut db,
        "MATCH (a:Doc)-[:CITES]->(b:Doc) WHERE b.title = 'doc 3' RETURN a, b",
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["a"]["id"], 1);
}

/// GIVEN a point and an `INSERT NODE` for the same id
/// WHEN MATCH binds that id
/// THEN the graph node's payload wins, as the more specific write.
#[test]
fn an_inserted_node_overrides_the_point_payload() {
    let mut db = DatabaseInner::new();
    docs_with_citations(&mut db);
    execute(
        &mut db,
        "INSERT NODE INTO docs (id = 2, payload = '{\"_labels\": [\"Doc\"], \"title\": \"renamed\"}')",
        None,
    )
    .expect("test: insert node");

    let found = rows(
        &mut db,
        "MATCH (a:Doc)-[:CITES]->(b:Doc) WHERE b.title = 'renamed' RETURN a, b",
    );
    assert_eq!(found.len(), 1);
}

/// GIVEN `INSERT NODE` with core's `_labels` key
/// WHEN a single-node MATCH filters on that label
/// THEN the node matches, as it does in core.
#[test]
fn insert_node_reads_the_core_labels_key() {
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT NODE INTO graph (id = 9, payload = '{\"_labels\": [\"Person\"], \"name\": \"Ada\"}')",
        None,
    )
    .expect("test: insert node");

    let result = execute(&mut db, "MATCH (n:Person) RETURN n", None).expect("test: MATCH");
    assert_eq!(result.row_count(), 1);
}

/// GIVEN a collection with labelled points and no edge yet
/// WHEN a single-node MATCH targets it
/// THEN its points match (core needs no edge to bind a node).
#[test]
fn a_single_node_match_needs_no_edge() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs")
        .expect("test: create docs");
    let store = db.get_shared_store("docs").expect("test: docs store");
    for id in 1..=2u64 {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &[],
            Some(serde_json::json!({"_labels": ["Doc"]})),
        );
    }

    let result = execute(&mut db, "MATCH (n:Doc) RETURN n", ON_DOCS).expect("test: MATCH");
    assert_eq!(result.row_count(), 2);
}

/// GIVEN the same points
/// WHEN the pattern carries inline properties
/// THEN they filter the nodes, as in core.
#[test]
fn inline_pattern_properties_filter_the_nodes() {
    let mut db = DatabaseInner::new();
    docs_with_citations(&mut db);
    let got = rows(
        &mut db,
        "MATCH (a:Doc)-[:CITES]->(b:Doc {title: 'doc 3'}) RETURN b",
    );
    assert_eq!(got.len(), 1, "{got:?}");
    let start = rows(&mut db, "MATCH (a:Doc {title: 'doc 2'}) RETURN a");
    assert_eq!(start.len(), 1, "{start:?}");
}
