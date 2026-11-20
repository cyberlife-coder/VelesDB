//! Tests for `rows`: a collection's rows read and written by what they
//! store — a graph's nodes, and a metadata collection's payload rows.

use std::path::PathBuf;

use serde_json::json;
use tempfile::TempDir;

use crate::collection::graph::{GraphSchema, NodeType};
use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::index::sparse::SparseVector;
use crate::point::Point;
use crate::storage::VectorStorage;

fn graph(dimension: Option<usize>) -> (TempDir, Collection) {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let graph = Collection::create_graph_collection(
        PathBuf::from(dir.path()),
        "g",
        GraphSchema::schemaless(),
        dimension,
        DistanceMetric::Cosine,
    )
    .expect("test: graph");
    (dir, graph)
}

fn stored_vectors(collection: &Collection) -> usize {
    collection.storage.vector_storage.read().len()
}

/// What node `id` stores: its vector and its payload.
fn stored(collection: &Collection, id: u64) -> Option<(Vec<f32>, Option<serde_json::Value>)> {
    collection
        .get(&[id])
        .remove(0)
        .map(|point| (point.vector, point.payload))
}

fn name_is(name: &str) -> Filter {
    Filter::new(Condition::Eq {
        field: "name".into(),
        value: json!(name),
    })
}

/// A point without a vector is a node without an embedding: it never
/// reaches the vector store, where a graph without embeddings used to keep
/// a zero-length vector for it.
#[test]
fn an_upsert_without_vectors_writes_no_vector_into_a_graph() {
    for dimension in [None, Some(2)] {
        let (_dir, graph) = graph(dimension);
        graph
            .upsert([
                Point::metadata_only(1, json!({"name": "x"})),
                Point::metadata_only(2, json!({"name": "y"})),
            ])
            .expect("test: upsert");
        assert_eq!(stored_vectors(&graph), 0, "{dimension:?}");
        assert_eq!(graph.len(), 2, "{dimension:?}");
        assert_eq!(
            graph.rows_where(Some(&name_is("x"))).len(),
            1,
            "{dimension:?}"
        );
    }
}

/// A batch mixing both kinds writes each by what it stores, and a repeated
/// id resolves as the same writes made one after the other do: a later
/// write without a vector keeps the vector an earlier one gave the id.
#[test]
fn a_mixed_batch_writes_each_row_by_what_it_stores() {
    let (_dir, graph) = graph(Some(2));
    graph
        .upsert([
            Point::new(1, vec![1.0, 0.0], Some(json!({"name": "first"}))),
            Point::metadata_only(2, json!({"name": "y"})),
            Point::metadata_only(1, json!({"name": "last"})),
        ])
        .expect("test: upsert");
    assert_eq!(
        stored_vectors(&graph),
        1,
        "the later payload write of id 1 keeps the vector the first gave it"
    );
    assert_eq!(
        vec![stored(&graph, 1), stored(&graph, 2)],
        vec![
            Some((vec![1.0, 0.0], Some(json!({"name": "last"})))),
            Some((vec![], Some(json!({"name": "y"})))),
        ]
    );
}

/// One batch leaves a graph as the same upserts made one by one do,
/// whatever order its rows with and without a vector come in.
#[test]
fn a_batch_resolves_a_repeated_id_as_sequential_upserts_do() {
    let v = || vec![1.0, 0.0];
    let batches: Vec<Vec<Point>> = vec![
        vec![
            Point::new(1, v(), Some(json!({"a": 1}))),
            Point::metadata_only(1, json!({"b": 2})),
        ],
        vec![
            Point::metadata_only(1, json!({"a": 1})),
            Point::new(1, v(), Some(json!({"b": 2}))),
        ],
        vec![
            Point::new(1, v(), Some(json!({"a": 1}))),
            Point::new(1, vec![], None),
        ],
        vec![
            Point::new(1, v(), Some(json!({"a": 1}))),
            Point::metadata_only(1, json!({"b": 2})),
            Point::new(1, vec![0.0, 1.0], None),
        ],
    ];
    for batch in batches {
        let (_dir_one, in_one_batch) = graph(Some(2));
        in_one_batch.upsert(batch.clone()).expect("test: batch");
        let (_dir_two, one_by_one) = graph(Some(2));
        for point in batch.clone() {
            one_by_one.upsert([point]).expect("test: one");
        }
        assert_eq!(
            stored(&in_one_batch, 1),
            stored(&one_by_one, 1),
            "{batch:?}"
        );
    }
}

/// A payload write leaves an embedding the node already has in place.
#[test]
fn a_payload_write_keeps_the_embedding_a_node_has() {
    let (_dir, graph) = graph(Some(2));
    graph
        .upsert([Point::new(1, vec![1.0, 0.0], Some(json!({"name": "x"})))])
        .expect("test: embedded");
    graph
        .upsert([Point::metadata_only(1, json!({"name": "z"}))])
        .expect("test: payload write");
    let row = graph.get(&[1]).remove(0).expect("test: node 1");
    assert_eq!(row.vector, vec![1.0, 0.0]);
    assert_eq!(row.payload, Some(json!({"name": "z"})));
}

/// A graph without embeddings still refuses a vector, before anything is
/// written.
#[test]
fn a_graph_without_embeddings_refuses_a_vector() {
    let (_dir, graph) = graph(None);
    let refused = graph.upsert([
        Point::metadata_only(1, json!({"name": "x"})),
        Point::new(2, vec![1.0], None),
    ]);
    assert!(refused.is_err());
    assert_eq!(graph.len(), 0, "a refused batch commits nothing");
}

/// The rows a filter keeps: payload-only and embedded nodes alike, through
/// the ids it names, a secondary index, or a scan.
#[test]
fn rows_where_reads_every_kind_of_node() {
    let (_dir, graph) = graph(Some(2));
    graph
        .upsert([
            Point::new(1, vec![1.0, 0.0], Some(json!({"name": "x"}))),
            Point::metadata_only(2, json!({"name": "x"})),
            Point::metadata_only(3, json!({"name": "y"})),
        ])
        .expect("test: upsert");
    let ids = |filter: Option<&Filter>| -> Vec<u64> {
        graph.rows_where(filter).iter().map(|p| p.id).collect()
    };
    assert_eq!(ids(None), vec![1, 2, 3]);
    assert_eq!(ids(Some(&name_is("x"))), vec![1, 2]);
    let named = Filter::new(Condition::In {
        field: crate::filter::POINT_ID_FIELD.into(),
        values: vec![json!(2), json!(9)].into(),
    });
    assert_eq!(ids(Some(&named)), vec![2]);
    graph.create_index("name").expect("test: index");
    assert_eq!(ids(Some(&name_is("x"))), vec![1, 2]);
}

/// The metadata-first `NEAR` scan reads only the nodes that store an
/// embedding: it used to score a payload-only node's empty vector, and the
/// cosine kernel's length assertion panicked.
#[test]
fn the_metadata_first_near_scan_skips_nodes_without_an_embedding() {
    let (_dir, graph) = graph(Some(2));
    graph
        .upsert([
            Point::metadata_only(1, json!({"name": "x"})),
            Point::new(3, vec![1.0, 0.0], Some(json!({"name": "x"}))),
        ])
        .expect("test: upsert");
    let ranked = graph.scan_and_score_by_vector(&name_is("x"), &[1.0, 0.0], 5);
    let ids: Vec<u64> = ranked.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![3]);
}

/// `get` reads a graph's rows: a payload-only node with an empty vector, as
/// SELECT reads it — every binding's point-get on a graph answers through it.
#[test]
fn get_reads_every_stored_node_of_a_graph() {
    let (_dir, graph) = graph(Some(2));
    graph
        .upsert([
            Point::new(1, vec![1.0, 0.0], Some(json!({"name": "x"}))),
            Point::metadata_only(2, json!({"name": "y"})),
        ])
        .expect("test: upsert");
    assert_eq!(
        vec![stored(&graph, 1), stored(&graph, 2), stored(&graph, 3)],
        vec![
            Some((vec![1.0, 0.0], Some(json!({"name": "x"})))),
            Some((vec![], Some(json!({"name": "y"})))),
            None,
        ]
    );
}

fn text_ids(collection: &Collection, query: &str) -> Vec<u64> {
    let mut ids: Vec<u64> = collection
        .text_search(query, 10)
        .expect("test: text search")
        .iter()
        .map(|hit| hit.point.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// A node written without a vector is indexed for full-text search, and
/// found by it, in a graph with embeddings or without: the node-payload
/// write never reached BM25, and the text search hydrated only rows with
/// a vector.
#[test]
fn a_payload_write_indexes_its_text_for_full_text_search() {
    for dimension in [None, Some(2)] {
        let (_dir, graph) = graph(dimension);
        graph
            .store_node_payload(1, &json!({"content": "hello world"}))
            .expect("test: node payload");
        graph
            .upsert([Point::metadata_only(2, json!({"content": "hello there"}))])
            .expect("test: upsert");
        assert_eq!(text_ids(&graph, "hello"), vec![1, 2], "{dimension:?}");
        let first = Filter::new(Condition::Eq {
            field: "content".into(),
            value: json!("hello world"),
        });
        let filtered: Vec<u64> = graph
            .text_search_with_filter("hello", 10, &first)
            .expect("test: filtered text search")
            .iter()
            .map(|hit| hit.point.id)
            .collect();
        assert_eq!(filtered, vec![1], "{dimension:?}");
    }
}

/// A payload write over an embedded node replaces its terms, as the vector
/// upsert does: the old text no longer matches, the new one does, and the
/// embedding stays.
#[test]
fn a_payload_write_replaces_the_terms_of_an_embedded_node() {
    let (_dir, graph) = graph(Some(2));
    graph
        .upsert([Point::new(
            3,
            vec![1.0, 0.0],
            Some(json!({"content": "hello again"})),
        )])
        .expect("test: embedded");
    graph
        .store_node_payload(3, &json!({"content": "bye now"}))
        .expect("test: node payload");
    assert_eq!(text_ids(&graph, "again"), Vec::<u64>::new());
    assert_eq!(text_ids(&graph, "bye"), vec![3]);
    graph
        .upsert([Point::metadata_only(3, json!({"content": "later"}))])
        .expect("test: payload row");
    assert_eq!(text_ids(&graph, "bye"), Vec::<u64>::new());
    assert_eq!(text_ids(&graph, "later"), vec![3]);
    assert_eq!(stored(&graph, 3).map(|(v, _)| v), Some(vec![1.0, 0.0]));
}

/// How many values the persisted `age` histogram counts.
fn age_histogram_count(dir: &std::path::Path) -> u64 {
    let bytes = std::fs::read(dir.join("collection.stats.json")).expect("test: stats file");
    let stats: crate::collection::stats::CollectionStats =
        serde_json::from_slice(&bytes).expect("test: stats json");
    stats
        .get_column_histogram("age")
        .expect("test: ANALYZE built an age histogram")
        .buckets
        .iter()
        .map(|bucket| bucket.count)
        .sum()
}

/// A payload write keeps the persisted histograms in step, as every other
/// write path does: a new node adds its value, an overwrite replaces it
/// (values inside the range ANALYZE bucketed, outside of which a histogram
/// counts nothing).
#[test]
fn a_payload_write_maintains_the_histograms() {
    let (dir, graph) = graph(Some(2));
    graph
        .upsert([
            Point::new(1, vec![1.0, 0.0], Some(json!({"age": 30}))),
            Point::metadata_only(2, json!({"age": 40})),
        ])
        .expect("test: upsert");
    let stats = graph.analyze().expect("test: analyze");
    graph.write_stats_guarded(&stats).expect("test: persist");
    let before = age_histogram_count(dir.path());
    graph
        .store_node_payload(3, &json!({"age": 35}))
        .expect("test: node payload");
    graph
        .upsert([Point::metadata_only(4, json!({"age": 32}))])
        .expect("test: payload row");
    assert_eq!(
        age_histogram_count(dir.path()),
        before + 2,
        "two new values"
    );
    graph
        .store_node_payload(1, &json!({"age": 38}))
        .expect("test: overwrite");
    assert_eq!(
        age_histogram_count(dir.path()),
        before + 2,
        "a replaced value"
    );
}

fn sparse(name: &str, term: u32) -> std::collections::BTreeMap<String, SparseVector> {
    std::collections::BTreeMap::from([(name.to_string(), SparseVector::new(vec![(term, 1.0)]))])
}

/// The ids the sparse index `name` answers for `term`.
fn sparse_ids(collection: &Collection, name: &str, term: u32) -> Vec<u64> {
    if !collection.query.sparse_indexes.read().contains_key(name) {
        return Vec::new();
    }
    collection
        .sparse_hits(name, &SparseVector::new(vec![(term, 1.0)]), 10, None, None)
        .expect("test: sparse search")
        .iter()
        .map(|hit| hit.point.id)
        .collect()
}

/// A sparse vector is independent of the dense one: a graph row without a
/// dense vector stores the sparse vectors it carries, searchable, where it
/// used to drop them; their names are checked as on a row with a vector.
#[test]
fn the_sparse_vectors_of_a_row_without_a_vector_are_stored() {
    for dimension in [None, Some(2)] {
        let (_dir, graph) = graph(dimension);
        graph
            .upsert([Point::with_sparse(
                1,
                vec![],
                Some(json!({"name": "x"})),
                Some(sparse("kw", 7)),
            )])
            .expect("test: upsert");
        assert_eq!(sparse_ids(&graph, "kw", 7), vec![1], "{dimension:?}");
        assert_eq!(stored_vectors(&graph), 0, "{dimension:?}");
        let refused = graph.upsert([Point::with_sparse(
            2,
            vec![],
            Some(json!({"name": "y"})),
            Some(sparse("../escape", 7)),
        )]);
        assert_eq!(
            refused.expect_err("test: a hostile name").code(),
            "VELES-017",
            "{dimension:?}"
        );
        assert_eq!(graph.len(), 1, "{dimension:?}");
    }
}

fn strict_graph() -> (TempDir, Collection) {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let schema = GraphSchema::new().with_node_type(NodeType::new("Person"));
    let graph = Collection::create_graph_collection(
        PathBuf::from(dir.path()),
        "s",
        schema,
        Some(2),
        DistanceMetric::Cosine,
    )
    .expect("test: graph");
    (dir, graph)
}

/// A strict schema checks the labels of every row, with a vector or without,
/// on every write entry point: a row carrying a vector used to skip it.
#[test]
fn a_strict_graph_checks_the_labels_of_every_row() {
    let (_dir, graph) = strict_graph();
    let alien = || Some(json!({"_labels": ["Alien"]}));
    let refusals = [
        graph.upsert([Point::new(1, vec![1.0, 0.0], alien())]),
        graph.upsert([Point::new(2, vec![], alien())]),
        graph
            .upsert_bulk(&[Point::new(3, vec![1.0, 0.0], alien())])
            .map(|_| ()),
        graph.upsert_metadata([Point::new(4, vec![], alien())]),
    ];
    for refused in refusals {
        assert_eq!(refused.expect_err("test: undeclared").code(), "VELES-017");
    }
    assert_eq!(graph.len(), 0, "a refused batch commits nothing");
    graph
        .upsert([Point::new(
            5,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Person"]})),
        )])
        .expect("test: declared");
}

/// The bulk entry points write a graph's rows by the same rule, keeping
/// their own contract that a row without a payload leaves the stored one:
/// a row without a vector never reaches the vector store of a graph without
/// embeddings, and a graph with embeddings takes it.
#[test]
fn a_bulk_write_follows_the_graph_rule() {
    let (_dir, plain) = graph(None);
    plain
        .upsert_bulk(&[Point::metadata_only(1, json!({"n": 1}))])
        .expect("test: bulk");
    plain
        .upsert_bulk_from_raw(&[], &[2], 0, Some(&[Some(json!({"n": 2}))]))
        .expect("test: raw bulk");
    assert_eq!(stored_vectors(&plain), 0);
    assert_eq!(plain.len(), 2);

    let (_dir, embedded) = graph(Some(2));
    embedded
        .upsert_bulk(&[
            Point::metadata_only(9, json!({"n": 9})),
            Point::new(10, vec![1.0, 0.0], Some(json!({"n": 10}))),
        ])
        .expect("test: mixed bulk");
    assert_eq!(stored_vectors(&embedded), 1);
    assert_eq!(stored(&embedded, 9), Some((vec![], Some(json!({"n": 9})))));
    assert!(embedded
        .upsert_bulk(&[Point::new(11, vec![1.0, 0.0, 0.0], None)])
        .is_err());
    embedded
        .upsert_bulk(&[
            Point::new(9, vec![], None),
            Point::new(10, vec![0.0, 1.0], None),
        ])
        .expect("test: rows without a payload");
    assert_eq!(stored(&embedded, 9), Some((vec![], Some(json!({"n": 9})))));
    assert_eq!(
        stored(&embedded, 10),
        Some((vec![0.0, 1.0], Some(json!({"n": 10}))))
    );
}

/// `upsert_metadata` on a graph writes payload rows as the node-payload
/// write does: an embedding the node has stays, and the vector count with it.
#[test]
fn upsert_metadata_on_a_graph_keeps_the_embedding() {
    let (_dir, graph) = graph(Some(2));
    graph
        .upsert([Point::new(1, vec![1.0, 0.0], Some(json!({"n": 1})))])
        .expect("test: embedded");
    graph
        .upsert_metadata([
            Point::metadata_only(1, json!({"n": 2})),
            Point::metadata_only(2, json!({"n": 3})),
        ])
        .expect("test: metadata write");
    assert_eq!(
        stored(&graph, 1),
        Some((vec![1.0, 0.0], Some(json!({"n": 2}))))
    );
    assert_eq!(stored_vectors(&graph), 1, "the vector count");
    assert_eq!(graph.len(), 2, "every stored node");
}

/// A bulk row carrying nothing — no vector, no payload, no sparse vector —
/// follows `upsert` where the id is new: it creates the node, with an empty
/// payload, and counts as written. Where the id is stored, the bulk contract
/// keeps what the row does not carry, so the row writes nothing and is not
/// counted. It used to be counted and reported either way, and wrote nothing
/// for a new id: `get` read `None` for the row it reported.
#[test]
fn a_bulk_row_carrying_nothing_creates_a_new_node_and_leaves_a_stored_one() {
    for dimension in [None, Some(2)] {
        let (_dir, graph) = graph(dimension);
        graph
            .store_node_payload(2, &json!({"n": 2}))
            .expect("test: stored node");

        let written = graph
            .upsert_bulk(&[Point::new(1, vec![], None), Point::new(2, vec![], None)])
            .expect("test: bulk");

        assert_eq!(written, 1, "only the new node is written: {dimension:?}");
        assert_eq!(
            stored(&graph, 1),
            Some((vec![], Some(json!({})))),
            "{dimension:?}"
        );
        assert_eq!(
            stored(&graph, 2),
            Some((vec![], Some(json!({"n": 2})))),
            "{dimension:?}"
        );
        let (_dir, upserted) = self::graph(dimension);
        upserted
            .upsert([Point::new(1, vec![], None)])
            .expect("test: upsert");
        assert_eq!(
            stored(&upserted, 1),
            stored(&graph, 1),
            "as upsert writes it"
        );
    }
}

fn metadata_rows() -> (TempDir, Collection) {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = Collection::create_metadata_only(PathBuf::from(dir.path()), "m")
        .expect("test: metadata collection");
    (dir, rows)
}

/// `upsert_bulk` writes a metadata collection's rows as `upsert` does: a
/// point without a vector is a payload row. The bulk path used to run the
/// vector checks, which refused every such point with `VELES-014` — the
/// points `upsert` accepts, and the bulk path took before.
#[test]
fn upsert_bulk_writes_the_rows_of_a_metadata_collection() {
    let (dir, rows) = metadata_rows();
    let written = rows
        .upsert_bulk(&[
            Point::new(4, vec![], Some(json!({"a": 4}))),
            Point::new(5, vec![], Some(json!({"a": 5}))),
        ])
        .expect("test: bulk");
    assert_eq!(written, 2, "both rows are written");
    assert_eq!(rows.len(), 2);
    assert_eq!(stored(&rows, 4), Some((vec![], Some(json!({"a": 4})))));
    rows.flush().expect("test: flush");
    drop(rows);
    let reopened = Collection::open(PathBuf::from(dir.path())).expect("test: reopen");
    assert_eq!(reopened.len(), 2, "reopened");
    assert_eq!(stored(&reopened, 5), Some((vec![], Some(json!({"a": 5})))));
}

/// On a metadata collection the bulk contract holds as on every other kind:
/// a point without a payload leaves the stored one and writes nothing. A
/// vector is refused as `upsert` refuses it, before anything is written.
#[test]
fn upsert_bulk_on_a_metadata_collection_keeps_the_bulk_contract() {
    let (_dir, rows) = metadata_rows();
    rows.upsert_bulk(&[Point::new(4, vec![], Some(json!({"a": 4})))])
        .expect("test: bulk");
    let written = rows
        .upsert_bulk(&[Point::new(4, vec![], None)])
        .expect("test: a row without a payload");
    assert_eq!(written, 0, "it writes nothing");
    assert_eq!(stored(&rows, 4), Some((vec![], Some(json!({"a": 4})))));

    let refused = rows.upsert_bulk(&[
        Point::new(6, vec![], Some(json!({"a": 6}))),
        Point::new(7, vec![1.0], Some(json!({"a": 7}))),
    ]);
    assert!(
        matches!(refused, Err(crate::Error::VectorNotAllowed(_))),
        "{refused:?}"
    );
    assert_eq!(
        rows.upsert([Point::new(7, vec![1.0], None)])
            .map_err(|e| e.code()),
        Err("VELES-014"),
        "as upsert refuses it"
    );
    assert_eq!(rows.len(), 1, "a refused batch writes nothing");
}
