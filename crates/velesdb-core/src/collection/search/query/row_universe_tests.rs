//! Tests for `row_universe`: which ids each collection kind reads as rows.

use super::{RowScope, RowUniverse};

#[test]
fn a_graph_reads_every_stored_node_and_a_ranking_only_the_embedded_ones() {
    assert!(RowUniverse::StoredNodes.admits(false, true));
    assert!(RowUniverse::StoredNodes.admits(true, false));
    assert!(!RowUniverse::StoredNodes.admits(false, false));
    // A NEAR scores a row by its vector: a payload-only node is no row of it.
    assert!(!RowUniverse::Vectors.admits(false, true));
    assert!(RowUniverse::Vectors.admits(true, true));
}

#[test]
fn a_collection_without_vectors_reads_its_payloads() {
    assert!(RowUniverse::Payloads.admits(false, true));
    assert!(!RowUniverse::Payloads.admits(true, false));
}

#[test]
fn the_embedded_scope_is_the_vector_universe_whatever_the_collection() {
    for rows in [
        RowUniverse::Vectors,
        RowUniverse::Payloads,
        RowUniverse::StoredNodes,
    ] {
        assert_eq!(RowScope::Embedded.universe(rows), RowUniverse::Vectors);
        assert_eq!(RowScope::All.universe(rows), rows);
    }
}

/// The rows are the kind of collection's, read from its configuration: a
/// vector collection's are its vectors even while it holds none — its
/// payloads used to be its rows then, and only then.
#[test]
fn the_rows_are_the_kind_of_collection_s() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let path = std::path::PathBuf::from(dir.path());
    let config = |collection: crate::collection::Collection| collection.config();
    let vectors = crate::collection::Collection::create(
        path.join("v"),
        2,
        crate::distance::DistanceMetric::Cosine,
    )
    .expect("test: vector collection");
    assert_eq!(RowUniverse::of(&config(vectors)), RowUniverse::Vectors);
    let rows = crate::collection::Collection::create_metadata_only(path.join("m"), "m")
        .expect("test: metadata collection");
    assert_eq!(RowUniverse::of(&config(rows)), RowUniverse::Payloads);
    let graph = crate::collection::Collection::create_graph_collection(
        path.join("g"),
        "g",
        crate::collection::graph::GraphSchema::schemaless(),
        Some(2),
        crate::distance::DistanceMetric::Cosine,
    )
    .expect("test: graph");
    assert_eq!(RowUniverse::of(&config(graph)), RowUniverse::StoredNodes);
}
