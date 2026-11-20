//! Tests for `size`: one count behind every size surface, kept exact by the
//! writes themselves.
//!
//! Each case drives one kind of collection through its write paths and, after
//! every step, compares the O(1) count with a recount from storage — so a
//! write that changes which ids a store holds without moving the count fails
//! here, naming the step.

use std::path::PathBuf;

use serde_json::json;
use tempfile::TempDir;

use crate::collection::graph::GraphSchema;
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;

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

/// The size counted from storage, the way `Collection::open` counts it.
fn recount(collection: &Collection) -> usize {
    let rule = RowUniverse::of(&collection.storage.config.read());
    let vectors = collection.storage.vector_storage.read();
    let payloads = collection.storage.payload_storage.read();
    rule.count(&vectors, &payloads)
}

/// Every size surface answers `expected`, and so does a recount.
fn assert_size(collection: &Collection, expected: usize, step: &str) {
    assert_eq!(recount(collection), expected, "{step}: recount");
    assert_eq!(collection.len(), expected, "{step}: len");
    assert_eq!(collection.is_empty(), expected == 0, "{step}: is_empty");
    assert_eq!(
        collection.config().point_count,
        expected,
        "{step}: config().point_count"
    );
}

fn payload(n: u64) -> serde_json::Value {
    json!({ "n": n })
}

/// A graph's size is its stored nodes, with an embedding, a payload or
/// both, each counted once — on every surface, whatever wrote them.
#[test]
fn a_graph_counts_every_stored_node_once_through_every_write() {
    let (_dir, graph) = graph(Some(2));
    assert_size(&graph, 0, "created");

    graph.store_node_payload(1, &payload(1)).expect("test");
    assert_size(&graph, 1, "a payload-only node");
    let (two, three) = (payload(2), payload(3));
    graph
        .store_node_payloads(&[(2, &two), (3, &three), (2, &two)])
        .expect("test");
    assert_size(&graph, 3, "a node-payload batch with a repeated id");
    graph.store_node_payload(1, &payload(10)).expect("test");
    assert_size(&graph, 3, "a payload rewrite");

    graph
        .upsert([Point::new(1, vec![1.0, 0.0], Some(payload(1)))])
        .expect("test");
    assert_size(&graph, 3, "an embedding attached to a payload-only node");
    graph
        .upsert([Point::new(4, vec![0.0, 1.0], None)])
        .expect("test");
    assert_size(&graph, 4, "an embedded node without a payload");
    graph
        .upsert([Point::new(1, vec![1.0, 0.0], None)])
        .expect("test");
    assert_size(&graph, 4, "a node whose payload an upsert removes");
    graph
        .upsert([Point::metadata_only(5, payload(5))])
        .expect("test");
    assert_size(&graph, 5, "an upsert without a vector");

    graph
        .upsert_bulk(&[
            Point::new(6, vec![1.0, 1.0], None),
            Point::metadata_only(7, payload(7)),
            Point::metadata_only(1, payload(11)),
        ])
        .expect("test");
    assert_size(&graph, 7, "a mixed bulk write");
    graph
        .upsert_bulk_from_raw(&[1.0, 0.0, 0.0, 1.0], &[8, 4], 2, None)
        .expect("test");
    assert_size(&graph, 8, "a raw bulk write");

    graph.delete(&[1, 6, 42, 7, 7]).expect("test");
    assert_size(&graph, 5, "a delete naming absent and repeated ids");
}

/// A graph without embeddings counts its payload nodes, from any write.
#[test]
fn a_graph_without_embeddings_counts_its_payload_nodes() {
    let (_dir, graph) = graph(None);
    graph.store_node_payload(1, &payload(1)).expect("test");
    graph
        .upsert([Point::metadata_only(2, payload(2))])
        .expect("test");
    graph
        .upsert_bulk(&[Point::metadata_only(3, payload(3))])
        .expect("test");
    assert_size(&graph, 3, "three writes");
    graph.delete(&[2]).expect("test");
    assert_size(&graph, 2, "a delete");
}

/// The count `open` reconciles is the one the writes kept, flushed or not.
#[test]
fn a_reopened_graph_counts_the_same_nodes() {
    let (dir, graph) = graph(Some(2));
    graph.store_node_payload(1, &payload(1)).expect("test");
    graph
        .upsert([
            Point::new(2, vec![1.0, 0.0], Some(payload(2))),
            Point::new(3, vec![0.0, 1.0], None),
        ])
        .expect("test");
    drop(graph);
    let reopened = Collection::open(PathBuf::from(dir.path())).expect("test: reopen");
    assert_size(&reopened, 3, "reopened without a flush");
    reopened.flush().expect("test: flush");
    drop(reopened);
    let again = Collection::open(PathBuf::from(dir.path())).expect("test: reopen");
    assert_size(&again, 3, "reopened after a flush");
}

/// A write that fails part-way moves the count by what it left in storage.
#[test]
fn a_failed_node_payload_write_moves_the_count_by_what_storage_holds() {
    let (_dir, graph) = graph(Some(2));
    graph.store_node_payload(1, &payload(1)).expect("test");
    {
        let _fault = crate::storage::fail_next_wal_flush();
        assert!(graph.store_node_payload(2, &payload(2)).is_err());
    }
    assert_eq!(graph.len(), recount(&graph), "the count follows storage");
    graph.store_node_payload(3, &payload(3)).expect("test");
    assert_eq!(graph.len(), recount(&graph), "and keeps following it");
}

/// A vector collection counts its vectors, a metadata collection its
/// payloads, as before.
#[test]
fn vector_and_metadata_collections_count_what_they_store() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let vectors = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Cosine)
        .expect("test: vector collection");
    vectors
        .upsert([
            Point::new(1, vec![1.0, 0.0], Some(payload(1))),
            Point::new(2, vec![0.0, 1.0], None),
        ])
        .expect("test");
    vectors
        .upsert_bulk(&[Point::new(3, vec![1.0, 1.0], None)])
        .expect("test");
    vectors
        .upsert_bulk_from_raw(&[1.0, 0.0], &[4], 2, None)
        .expect("test");
    assert_size(&vectors, 4, "vector writes");
    assert_eq!(
        vectors
            .upsert_metadata([Point::metadata_only(9, payload(9))])
            .map_err(|e| e.code()),
        Err("VELES-016"),
        "a payload without a vector would be no row of it"
    );
    vectors
        .store_node_payload(9, &payload(9))
        .expect("test: an edge endpoint's payload");
    assert_size(&vectors, 4, "a payload without a vector is no row of it");
    vectors.delete(&[1, 9]).expect("test");
    assert_size(&vectors, 3, "a vector delete");

    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = Collection::create_metadata_only(PathBuf::from(dir.path()), "m")
        .expect("test: metadata collection");
    rows.upsert([
        Point::metadata_only(1, payload(1)),
        Point::metadata_only(2, payload(2)),
    ])
    .expect("test");
    assert_size(&rows, 2, "metadata writes");
    rows.delete(&[2, 9]).expect("test");
    assert_size(&rows, 1, "a metadata delete");
}

/// A vector collection's rows are its vectors, the ids its size counts:
/// payloads stored there without a vector are neither. They used to be
/// SELECT rows while the collection held no vector at all — and stopped
/// being rows the moment it held one — while its size counted none of them.
#[test]
fn a_vector_collection_reads_as_rows_only_what_its_size_counts() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let vectors = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Cosine)
        .expect("test: vector collection");
    vectors
        .store_node_payloads(&[(1, &payload(1)), (2, &payload(2))])
        .expect("test: edge endpoints' payloads");
    assert_size(&vectors, 0, "payloads without a vector");
    assert!(vectors.row_ids().is_empty(), "row ids");
    assert!(vectors.rows_where(None).is_empty(), "the rows DML reads");
    let params = std::collections::HashMap::new();
    let selected = vectors
        .execute_query_str("SELECT * FROM c LIMIT 10", &params)
        .expect("test: select");
    assert!(selected.is_empty(), "SELECT *: {selected:?}");
    let count = crate::velesql::Parser::parse("SELECT COUNT(*) FROM c").expect("test: parse");
    assert_eq!(
        vectors.execute_aggregate(&count, &params).expect("test")["count"],
        json!(0),
        "COUNT(*)"
    );
}

/// The MATCH planner and ANALYZE read the same count the size surfaces do.
#[test]
fn the_planner_and_analyze_read_the_one_count() {
    let (_dir, graph) = graph(Some(2));
    graph.store_node_payload(1, &payload(1)).expect("test");
    graph
        .upsert([Point::new(2, vec![1.0, 0.0], None)])
        .expect("test");
    assert_eq!(graph.compute_match_collection_stats().total_nodes, 2);
    assert_eq!(graph.analyze().expect("test: analyze").row_count, 2);
}

/// The count stays exact while writers race over the same ids of a graph
/// with embeddings: node-payload writes, vector upserts, mixed bulk writes
/// and deletes. Every write moves the count under a storage write guard, so
/// a sampler holding both stores' read guards — no write in flight — must
/// read the count a recount finds, every time; and so must the end state.
#[test]
fn concurrent_writes_keep_the_count_exact() {
    const WRITERS: u64 = 4;
    const ROUNDS: u64 = 60;
    const IDS: u64 = 12;
    let (_dir, graph) = graph(Some(2));
    let done = std::sync::atomic::AtomicBool::new(false);
    let samples = std::thread::scope(|scope| {
        let sampler = scope.spawn(|| sample_until(&graph, &done));
        let writers: Vec<_> = (0..WRITERS)
            .map(|writer| {
                let graph = &graph;
                scope.spawn(move || {
                    for round in 0..ROUNDS {
                        race_one_write(graph, writer + round, (writer * 5 + round) % IDS, IDS);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("test: writer");
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        sampler.join().expect("test: sampler")
    });
    assert!(samples > 0, "the sampler read the count while writes ran");
    assert_eq!(graph.len(), recount(&graph), "the end state");
}

/// One write of the race, chosen by `turn`, over `id` (and its neighbour).
fn race_one_write(graph: &Collection, turn: u64, id: u64, ids: u64) {
    let next = (id + 1) % ids;
    let written = match turn % 4 {
        0 => graph.store_node_payload(id, &payload(id)),
        1 => graph.upsert([Point::new(id, vec![1.0, 0.0], None)]),
        2 => graph
            .upsert_bulk(&[
                Point::new(id, vec![], Some(payload(id))),
                Point::new(next, vec![0.0, 1.0], None),
            ])
            .map(drop),
        _ => graph.delete(&[id, next]),
    };
    written.expect("test: a racing write");
}

/// Compares the count with a recount under both stores' read guards until
/// `done`; returns how many comparisons it made.
fn sample_until(graph: &Collection, done: &std::sync::atomic::AtomicBool) -> usize {
    let mut samples = 0;
    while !done.load(std::sync::atomic::Ordering::Relaxed) {
        let rule = RowUniverse::of(&graph.storage.config.read());
        let vectors = graph.storage.vector_storage.read();
        let payloads = graph.storage.payload_storage.read();
        assert_eq!(
            graph.size(),
            rule.count(&vectors, &payloads),
            "the count under a consistent read"
        );
        drop(payloads);
        drop(vectors);
        samples += 1;
        std::thread::yield_now();
    }
    samples
}

/// Runs `paused` on its own thread until it pauses once it read the other
/// store's membership of its ids (`write_hooks`), then `racing` on another
/// thread, then resumes `paused`. A write whose node lock `paused` holds
/// waits for it, so `racing` is given half a second to finish inside the
/// pause: unguarded, it does; guarded, it waits either way. Every outcome
/// leaves both writes done, so the caller asserts on the end state alone.
fn race_inside_the_membership_pause(
    graph: &Collection,
    paused: impl FnOnce(&Collection) + Send + 'static,
    racing: impl FnOnce(&Collection) + Send + 'static,
) {
    use std::sync::mpsc;
    use std::time::Duration;
    let (paused_tx, paused_rx) = mpsc::channel::<()>();
    let (resume_tx, resume_rx) = mpsc::channel::<()>();
    let first = graph.clone();
    let first = std::thread::spawn(move || {
        super::write_hooks::pause_after_node_membership(move || {
            paused_tx.send(()).expect("test: paused");
            let _ = resume_rx.recv_timeout(Duration::from_secs(10));
        });
        paused(&first);
    });
    paused_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("test: the first write paused");
    let (wrote_tx, wrote_rx) = mpsc::channel::<()>();
    let second = graph.clone();
    let second = std::thread::spawn(move || {
        racing(&second);
        let _ = wrote_tx.send(());
    });
    let _ = wrote_rx.recv_timeout(Duration::from_millis(500));
    resume_tx.send(()).expect("test: resume");
    first.join().expect("test: the paused write");
    second.join().expect("test: the racing write");
}

/// A graph's payload write and vector write of one new id, run beside each
/// other: the payload write pauses once it read the vector store's
/// membership of its id, and the vector write — through `upsert`, whose
/// payload half deletes a payload too — runs then. The node lock of the id
/// makes the vector write wait for the payload write; without it, each
/// would count the new node — the vector write seeing no payload yet, the
/// payload write having read no vector — and the size would read 2 for one
/// node.
#[test]
fn a_payload_write_and_a_vector_write_of_one_new_node_count_it_once() {
    let (_dir, graph) = graph(Some(2));
    race_inside_the_membership_pause(
        &graph,
        |graph| {
            graph
                .store_node_payload(1, &payload(1))
                .expect("test: the payload write");
        },
        |graph| {
            graph
                .upsert([Point::new(1, vec![1.0, 0.0], None)])
                .expect("test: the vector write");
        },
    );
    assert_eq!(recount(&graph), 1, "one node");
    assert_eq!(graph.len(), 1, "counted once");
}

/// The same race with a write that only stores a vector — a bulk point
/// without a payload, which leaves the payload store alone — so the only
/// node lock it can wait on is its vector write's own. Without it, the
/// vector write counts the new node from a payload store it read empty, and
/// so does the payload write it slipped past.
#[test]
fn a_vector_only_write_waits_for_a_payload_write_of_the_same_new_node() {
    let (_dir, graph) = graph(Some(2));
    race_inside_the_membership_pause(
        &graph,
        |graph| {
            graph
                .store_node_payload(1, &payload(1))
                .expect("test: the payload write");
        },
        |graph| {
            graph
                .upsert_bulk(&[Point::new(1, vec![1.0, 0.0], None)])
                .expect("test: the vector-only write");
        },
    );
    assert_eq!(recount(&graph), 1, "one node");
    assert_eq!(graph.len(), 1, "counted once");
}

/// A delete of a payload-only node, run while a vector-only write of it is
/// paused having read that the node holds a payload. The delete takes the
/// node's lock, so it waits for the vector write and then removes the whole
/// node. Without it, the delete removes the payload and uncounts the node,
/// and the vector write, still counting on the payload it read, leaves the
/// node it embeds uncounted.
#[test]
fn a_delete_waits_for_a_vector_only_write_of_a_payload_only_node() {
    let (_dir, graph) = graph(Some(2));
    graph
        .store_node_payload(1, &payload(1))
        .expect("test: seed");
    race_inside_the_membership_pause(
        &graph,
        |graph| {
            graph
                .upsert_bulk(&[Point::new(1, vec![1.0, 0.0], None)])
                .expect("test: the vector-only write");
        },
        |graph| graph.delete(&[1]).expect("test: the delete"),
    );
    assert_eq!(graph.len(), recount(&graph), "the count follows storage");
    assert_eq!(graph.len(), 0, "the delete came last");
}

/// An upsert without a payload of a node that stores only one removes the
/// payload while it stores the vector. Its payload write may remove a value,
/// so it measures the node under its lock, whatever its store holds: the
/// node loses its payload and gains its embedding, and stays counted once.
/// Judged by the store alone, as a write that only stores is, the payload
/// removal would move nothing and the vector write would count the node
/// again.
#[test]
fn a_payloadless_upsert_of_a_payload_only_node_counts_it_once() {
    let (_dir, graph) = graph(Some(2));
    graph
        .store_node_payload(1, &payload(1))
        .expect("test: seed");
    graph
        .upsert([Point::new(1, vec![1.0, 0.0], None)])
        .expect("test: the payloadless upsert");
    assert_size(&graph, 1, "the node, embedded, without its payload");
}
