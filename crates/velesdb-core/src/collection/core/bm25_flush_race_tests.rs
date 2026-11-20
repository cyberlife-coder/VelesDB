//! A BM25 change racing `flush_full`: the change is appended to the BM25 WAL,
//! then applied in memory, while the flush serializes the index, then
//! truncates the WAL. A change appended before the truncate and applied
//! after the serialization would be in neither, and lost on the next open —
//! for a graph too, whose catch-up re-reads only the payloads stored after
//! the position its marker records. `Collection::flush_bm25_index` holds
//! `text_wal` exclusively across both, every change holds it shared across
//! its own two steps, and these tests pause a change between its steps
//! (`write_hooks`) while a full flush — or, for a delete, the snapshot step
//! alone — runs.

#![cfg(all(test, feature = "persistence"))]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;

use crate::collection::graph::GraphSchema;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;

use super::write_hooks::{pause_after_text_wal, pause_before_text_wal};

const WAIT: Duration = Duration::from_secs(10);

/// Runs `write` on its own thread, paused between its BM25 WAL append and
/// its in-memory apply, while `flush_full` runs on another
/// ([`snapshot_during_a_text_change`]).
fn flush_during_a_text_change(collection: &Collection, write: fn(&Collection)) {
    snapshot_during_a_text_change(collection, write, |c| {
        c.flush_full().expect("test: flush_full");
    });
}

/// Runs `write` on its own thread, paused between its BM25 WAL append and
/// its in-memory apply, while `flush` runs on another. Unguarded, a flush
/// reaching the BM25 snapshot finishes it inside the pause; guarded, it
/// waits for the change, so it is given half a second before the change
/// resumes either way.
fn snapshot_during_a_text_change(
    collection: &Collection,
    write: fn(&Collection),
    flush: fn(&Collection),
) {
    let (paused_tx, paused_rx) = mpsc::channel::<()>();
    let (resume_tx, resume_rx) = mpsc::channel::<()>();
    let writer = collection.clone();
    let writer = std::thread::spawn(move || {
        pause_after_text_wal(move || {
            paused_tx.send(()).expect("test: paused");
            let _ = resume_rx.recv_timeout(WAIT);
        });
        write(&writer);
    });
    paused_rx
        .recv_timeout(WAIT)
        .expect("test: the change paused");

    let (flushed_tx, flushed_rx) = mpsc::channel::<()>();
    let flusher = collection.clone();
    let flusher = std::thread::spawn(move || {
        flush(&flusher);
        let _ = flushed_tx.send(());
    });
    let _ = flushed_rx.recv_timeout(Duration::from_millis(500));
    resume_tx.send(()).expect("test: resume");
    writer.join().expect("test: writer");
    flusher.join().expect("test: flusher");
}

fn found(collection: &Collection, id: u64) -> bool {
    collection
        .text_search("needle", 10)
        .expect("test: text search")
        .iter()
        .any(|r| r.point.id == id)
}

fn reopened(dir: &Path) -> Collection {
    Collection::open(PathBuf::from(dir)).expect("test: reopen")
}

#[test]
fn a_text_change_racing_a_full_flush_survives_a_reopen() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Cosine)
        .expect("test: collection");
    collection
        .upsert([Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"content": "hay"})),
        )])
        .expect("test: seed");

    flush_during_a_text_change(&collection, |c| {
        c.upsert([Point::new(
            2,
            vec![0.0, 1.0],
            Some(json!({"content": "needle"})),
        )])
        .expect("test: the racing write");
    });
    assert!(found(&collection, 2), "in memory");
    drop(collection);

    assert!(found(&reopened(dir.path()), 2), "after the reopen");
}

#[test]
fn a_graph_text_change_racing_a_full_flush_survives_a_reopen() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let graph = Collection::create_graph_collection(
        PathBuf::from(dir.path()),
        "g",
        GraphSchema::schemaless(),
        Some(2),
        DistanceMetric::Cosine,
    )
    .expect("test: graph");
    graph
        .store_node_payload(1, &json!({"content": "hay"}))
        .expect("test: seed");

    flush_during_a_text_change(&graph, |g| {
        g.store_node_payload(2, &json!({"content": "needle"}))
            .expect("test: the racing write");
    });
    assert!(found(&graph, 2), "in memory");
    drop(graph);

    assert!(found(&reopened(dir.path()), 2), "after the reopen");
}

/// A delete racing the BM25 snapshot. The delete holds its storage write
/// guards across its BM25 change, so a whole `flush_full` would wait for it
/// at the vector store, before ever reaching the snapshot: the snapshot step
/// runs alone here. And the reopened index itself is read, not a text
/// search, whose admission hides a document left for an id that is no row.
#[test]
fn a_text_delete_racing_the_bm25_snapshot_survives_a_reopen() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Cosine)
        .expect("test: collection");
    collection
        .upsert([
            Point::new(1, vec![1.0, 0.0], Some(json!({"content": "needle"}))),
            Point::new(2, vec![0.0, 1.0], Some(json!({"content": "hay"}))),
        ])
        .expect("test: seed");

    snapshot_during_a_text_change(
        &collection,
        |c| c.delete(&[1]).expect("test: the racing delete"),
        |c| c.flush_bm25_index().expect("test: BM25 snapshot"),
    );
    assert!(!indexed(&collection, 1), "in memory");
    drop(collection);

    assert!(!indexed(&reopened(dir.path()), 1), "after the reopen");
}

/// Whether the BM25 index holds a "needle" document for `id`, whether or
/// not `id` is a row.
fn indexed(collection: &Collection, id: u64) -> bool {
    collection
        .storage
        .text_index
        .score_all("needle")
        .iter()
        .any(|&(hit, _)| hit == id)
}

/// While `flush_full` holds `text_wal` exclusively — serializing the BM25
/// snapshot, syncing it, truncating the WAL — a delete waits for it. It
/// waits holding no storage guard: it takes `text_wal` before its node
/// locks and its storage write guards, so a read of another id is not
/// queued behind it for the length of the snapshot. The test holds the
/// exclusive guard itself, as the flush does, and lets the delete reach it.
#[test]
fn a_delete_waiting_for_a_bm25_snapshot_holds_up_no_read() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Cosine)
        .expect("test: collection");
    collection
        .upsert([
            Point::new(1, vec![1.0, 0.0], Some(json!({"content": "hay"}))),
            Point::new(2, vec![0.0, 1.0], Some(json!({"content": "needle"}))),
        ])
        .expect("test: seed");

    let snapshot = collection.storage.text_wal.write();
    let (queued_tx, queued_rx) = mpsc::channel::<()>();
    let deleter = collection.clone();
    let deleter = std::thread::spawn(move || {
        pause_before_text_wal(move || queued_tx.send(()).expect("test: queued"));
        deleter.delete(&[1]).expect("test: the delete");
    });
    queued_rx
        .recv_timeout(WAIT)
        .expect("test: the delete reached text_wal");
    let (read_tx, read_rx) = mpsc::channel();
    let reader = collection.clone();
    let reader = std::thread::spawn(move || {
        let _ = read_tx.send(reader.get(&[2]));
    });
    let read = read_rx.recv_timeout(Duration::from_secs(5));
    drop(snapshot);
    deleter.join().expect("test: deleter");
    reader.join().expect("test: reader");

    let read = read.expect("test: a read of another id waited for the BM25 snapshot");
    assert_eq!(read[0].as_ref().map(|p| p.id), Some(2));
    assert!(collection.get(&[1])[0].is_none(), "the delete ran after it");
}
