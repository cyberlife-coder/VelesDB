//! A MATCH reads its nodes by the rule every read reads a collection's rows
//! by (`RowUniverse`): on a vector collection, an id that stores a payload
//! but no vector is no node, whichever way the walk reaches it — the
//! start-node scan, the label index, a WHERE, an edge.
//!
//! Such an id is what an earlier version let `upsert_metadata` store on a
//! vector collection; this one refuses it, so it is planted below the
//! public API here, as the label index of that version indexed it.

use crate::collection::graph::GraphEdge;
use crate::collection::types::Collection;
use crate::point::Point;
use crate::storage::PayloadStorage;
use crate::velesql::{MatchClause, Parser};
use std::collections::{BTreeSet, HashMap};
use tempfile::TempDir;

/// The payload-only id planted on the vector collection.
const GHOST: u64 = 99;

/// A vector collection with three people (ids 1-3), the ghost planted
/// beside them, and the edges `1 -KNOWS-> 2`, `1 -KNOWS-> ghost` and
/// `ghost -KNOWS-> 3`: 3 is reachable from 1 only through the ghost.
fn people_and_a_ghost(dir: &TempDir) -> Collection {
    let col = Collection::create(
        dir.path().join("people"),
        2,
        crate::distance::DistanceMetric::Cosine,
    )
    .expect("test: create collection");
    let person = |id: u64, name: &str| {
        Point::new(
            id,
            vec![1.0, 0.0],
            Some(serde_json::json!({ "_labels": ["Person"], "name": name })),
        )
    };
    col.upsert([person(1, "ada"), person(2, "alan"), person(3, "grace")])
        .expect("test: upsert");
    let ghost = serde_json::json!({ "_labels": ["Person"], "name": "ghost" });
    col.storage
        .payload_storage
        .write()
        .store(GHOST, &ghost)
        .expect("test: plant the ghost's payload");
    col.graph
        .label_index
        .write()
        .index_from_payload(GHOST, &ghost);
    for (id, source, target) in [(10, 1, 2), (11, 1, GHOST), (12, GHOST, 3)] {
        col.add_edge(GraphEdge::new(id, source, target, "KNOWS").expect("test: edge"))
            .expect("test: add edge");
    }
    col
}

fn clause(sql: &str) -> MatchClause {
    Parser::parse(sql)
        .expect("test: parse")
        .match_clause
        .expect("test: a MATCH statement")
}

/// The ids `alias` binds over the rows `sql` answers.
fn bound(col: &Collection, sql: &str, alias: &str) -> BTreeSet<u64> {
    col.execute_match(&clause(sql), &HashMap::new())
        .expect("test: match")
        .iter()
        .map(|row| row.bindings[alias])
        .collect()
}

#[test]
fn a_start_node_scan_reads_only_the_rows() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let col = people_and_a_ghost(&dir);
    assert_eq!(
        bound(&col, "MATCH (n) RETURN n.name", "n"),
        BTreeSet::from([1, 2, 3])
    );
}

#[test]
fn a_label_lookup_reads_only_the_rows() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let col = people_and_a_ghost(&dir);
    assert_eq!(
        bound(&col, "MATCH (n:Person) RETURN n.name", "n"),
        BTreeSet::from([1, 2, 3])
    );
}

#[test]
fn a_where_filtered_match_finds_no_payload_only_id() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let col = people_and_a_ghost(&dir);
    for sql in [
        "MATCH (n) WHERE n.name = 'ghost' RETURN n.name",
        "MATCH (n:Person) WHERE n.name = 'ghost' RETURN n.name",
    ] {
        assert!(bound(&col, sql, "n").is_empty(), "{sql}");
    }
}

#[test]
fn an_edge_never_leads_to_a_payload_only_id() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let col = people_and_a_ghost(&dir);
    assert_eq!(
        bound(&col, "MATCH (a)-[:KNOWS]->(b) RETURN b.name", "b"),
        BTreeSet::from([2])
    );
    assert_eq!(
        bound(&col, "MATCH (a)-[:KNOWS]->(b) RETURN a.name", "a"),
        BTreeSet::from([1]),
        "nor starts from one"
    );
    assert_eq!(
        bound(&col, "MATCH (a)-[:KNOWS*1..2]->(c) RETURN c.name", "c"),
        BTreeSet::from([2]),
        "nor passes through one"
    );
}

/// A VectorFirst MATCH seeds its walk with the nearest ids a vector search
/// returned, before the walk takes its storage guards: a delete landing in
/// between leaves a seed that is no row any more, and it starts no row —
/// nor does an id that never was one (the ghost) — as the start-node scan
/// would not.
#[test]
fn a_seed_that_is_no_row_starts_no_row() {
    use super::match_exec::{MatchSources, RowOrder, StartNodes};
    let dir = tempfile::tempdir().expect("test: tempdir");
    let col = people_and_a_ghost(&dir);
    col.delete(&[2]).expect("test: delete a seed");

    let rows = col
        .execute_match_from(
            &clause("MATCH (n) RETURN n.name"),
            &HashMap::new(),
            None,
            &MatchSources::default(),
            StartNodes::Seeded(&[2, GHOST, 3]),
            RowOrder::Clause,
        )
        .expect("test: seeded match");

    let starts: Vec<u64> = rows.iter().map(|row| row.bindings["n"]).collect();
    assert_eq!(starts, [3]);
}
