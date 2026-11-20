//! Tests for `bm25_coverage`: a graph's BM25 snapshot is trusted only where
//! its coverage marker vouches for it, and caught up from the payload log.
//!
//! An older binary indexed a graph's embedded nodes only. It still opens a
//! data directory this one wrote — the snapshot format is the one it knows —
//! and it may write nodes without an embedding there, unindexed, rewriting
//! the snapshot or not. Each case below is one such history.

#![cfg(all(test, feature = "persistence"))]

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::collection::graph::GraphSchema;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::index::bm25_persistence::{
    coverage_path, load_coverage, save_coverage, snapshot_path, Bm25Coverage,
};
use crate::index::Bm25Index;
use crate::point::Point;
use crate::storage::{LogPayloadStorage, PayloadStorage};

/// A graph with a payload-only node (1) and an embedded node (3), its BM25
/// snapshot and coverage marker written by a full flush.
fn flushed_graph(dir: &Path) -> Collection {
    let graph = Collection::create_graph_collection(
        PathBuf::from(dir),
        "kge",
        GraphSchema::schemaless(),
        Some(2),
        DistanceMetric::Cosine,
    )
    .expect("test: graph");
    graph
        .store_node_payload(1, &json!({ "content": "hello world" }))
        .expect("test: payload-only node");
    graph
        .upsert(vec![Point::new(
            3,
            vec![1.0, 0.0],
            Some(json!({ "content": "other text" })),
        )])
        .expect("test: embedded node");
    graph.flush_full().expect("test: full flush");
    graph
}

/// The snapshot an older writer leaves for that graph: its embedded node
/// only, in the one wire format both binaries read.
fn write_older_snapshot(dir: &Path) {
    let older = Bm25Index::new();
    older.add_document(3, "other text");
    let bytes = postcard::to_allocvec(&older.to_snapshot()).expect("test: serialize");
    std::fs::write(snapshot_path(dir), bytes).expect("test: write snapshot");
}

fn text_hits(col: &Collection, query: &str) -> Vec<u64> {
    let mut ids: Vec<u64> = col
        .text_search(query, 10)
        .expect("test: text search")
        .iter()
        .map(|r| r.point.id)
        .collect();
    ids.sort_unstable();
    ids
}

fn reopen(dir: &Path) -> Collection {
    Collection::open(PathBuf::from(dir)).expect("test: reopen")
}

fn snapshot_version(dir: &Path) -> u32 {
    let bytes = std::fs::read(snapshot_path(dir)).expect("test: snapshot");
    postcard::take_from_bytes::<u32>(&bytes)
        .expect("test: header")
        .0
}

/// A frame the WAL replays over a snapshot it trusts — and that a rebuild,
/// which discards the WAL, never sees: whether node 1 is still found tells
/// which of the two an open did.
fn append_stale_removal(dir: &Path) {
    let wal = crate::index::bm25_persistence_wal::wal_path_for_bm25(dir);
    crate::index::bm25_persistence_wal::wal_append_remove_document(&wal, 1)
        .expect("test: stale frame");
}

/// Every kind of collection writes the snapshot format an older binary reads:
/// the graph's marker lives beside it, in a file the older binary ignores.
/// The format used to move to version 3 for every kind, which an older
/// binary refused as corrupted.
#[test]
fn the_snapshot_keeps_the_format_older_binaries_read() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    drop(flushed_graph(dir.path()));
    assert_eq!(snapshot_version(dir.path()), 2, "a graph's snapshot");
    assert!(coverage_path(dir.path()).exists(), "a graph's marker");

    let other = tempfile::tempdir().expect("test: tempdir");
    let vectors = Collection::create(PathBuf::from(other.path()), 2, DistanceMetric::Cosine)
        .expect("test: vector collection");
    vectors
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"content": "vector words"})),
        )])
        .expect("test: upsert");
    vectors.flush_full().expect("test: full flush");
    assert_eq!(snapshot_version(other.path()), 2, "a vector collection's");
    assert!(
        !coverage_path(other.path()).exists(),
        "only a graph has one"
    );
}

/// A graph snapshot no marker vouches for — an older binary wrote it, or a
/// crash came between a snapshot and its marker — is rebuilt from the payload
/// store, once: the rebuilt snapshot gets its marker, so the next open loads
/// it and replays its WAL.
#[test]
fn a_graph_snapshot_without_a_marker_is_rebuilt_once() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    drop(flushed_graph(dir.path()));
    write_older_snapshot(dir.path());
    std::fs::remove_file(coverage_path(dir.path())).expect("test: no marker");

    let rebuilt = reopen(dir.path());
    assert_eq!(text_hits(&rebuilt, "world"), vec![1], "rebuilt");
    assert_eq!(text_hits(&rebuilt, "other"), vec![3]);
    assert!(
        coverage_path(dir.path()).exists(),
        "the rebuild is vouched for"
    );
    drop(rebuilt);

    append_stale_removal(dir.path());
    let loaded = reopen(dir.path());
    assert!(
        text_hits(&loaded, "world").is_empty(),
        "the next open loads the snapshot and replays its WAL"
    );
}

/// The marker vouches for one snapshot, byte for byte: a snapshot an older
/// binary rewrote over it — without the nodes it never indexed — is not
/// that snapshot, so it is rebuilt.
#[test]
fn a_snapshot_an_older_binary_rewrote_is_rebuilt() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    drop(flushed_graph(dir.path()));
    write_older_snapshot(dir.path());

    let reopened = reopen(dir.path());
    assert_eq!(text_hits(&reopened, "world"), vec![1]);
    assert_eq!(text_hits(&reopened, "other"), vec![3]);
}

/// An older binary that stores a node without an embedding leaves the
/// snapshot and the BM25 WAL as they were — it indexes no such node — so
/// nothing in the index tells of it. The payload log does: every node stored
/// after the position the marker records is indexed again on open.
#[test]
fn nodes_an_older_binary_stored_since_the_snapshot_are_indexed() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    drop(flushed_graph(dir.path()));
    {
        let mut payloads = LogPayloadStorage::new(dir.path()).expect("test: payload store");
        payloads
            .store(5, &json!({ "content": "fresh words" }))
            .expect("test: unindexed node");
        payloads
            .store(1, &json!({ "content": "replaced" }))
            .expect("test: unindexed rewrite");
    }

    let reopened = reopen(dir.path());
    assert_eq!(text_hits(&reopened, "fresh"), vec![5], "a new node");
    assert_eq!(text_hits(&reopened, "replaced"), vec![1], "a rewritten one");
    assert!(
        text_hits(&reopened, "world").is_empty(),
        "its old terms are gone"
    );
    assert_eq!(text_hits(&reopened, "other"), vec![3], "the rest loads");
}

/// The crash window of a rebuild: the WAL emptied, the rebuilt snapshot not
/// yet written. The snapshot on disk is still the untrusted one, so the next
/// open rebuilds again — and the payload store holds every write either
/// recorded.
#[test]
fn a_crash_inside_a_rebuild_rebuilds_again() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    drop(flushed_graph(dir.path()));
    write_older_snapshot(dir.path());
    drop(reopen(dir.path()));

    // The crash window: the untrusted snapshot, its WAL already emptied.
    write_older_snapshot(dir.path());
    let wal = crate::index::bm25_persistence_wal::wal_path_for_bm25(dir.path());
    assert_eq!(std::fs::metadata(&wal).map_or(0, |m| m.len()), 0);
    let recovered = reopen(dir.path());
    assert_eq!(text_hits(&recovered, "world"), vec![1]);
}

/// A vector collection's snapshot was always written under today's rule, so
/// it loads as it is, with no marker, its WAL replayed on top.
#[test]
fn a_vector_collection_snapshot_loads_without_a_marker() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let vectors = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Cosine)
        .expect("test: vector collection");
    vectors
        .upsert(vec![
            Point::new(1, vec![1.0, 0.0], Some(json!({"content": "alpha one"}))),
            Point::new(2, vec![0.0, 1.0], Some(json!({"content": "alpha two"}))),
        ])
        .expect("test: upsert");
    vectors.flush_full().expect("test: full flush");
    drop(vectors);
    let only_one = Bm25Index::new();
    only_one.add_document(1, "alpha one");
    let bytes = postcard::to_allocvec(&only_one.to_snapshot()).expect("test: serialize");
    std::fs::write(snapshot_path(dir.path()), bytes).expect("test: write snapshot");

    let reopened = reopen(dir.path());
    assert_eq!(
        text_hits(&reopened, "alpha"),
        vec![1],
        "loaded from the snapshot, not rebuilt from the payloads"
    );
}

/// A marker whose payload log position lies past the end of the log — the
/// log lost its tail since the marker was written — vouches for nothing: the
/// payloads it would have the open catch up on are not all in the log, so
/// the index is rebuilt (the stale WAL frame, which a trusted load replays,
/// is discarded with it).
#[test]
fn a_marker_past_the_end_of_the_payload_log_is_rebuilt() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    drop(flushed_graph(dir.path()));
    let marker = load_coverage(dir.path()).expect("test: marker");
    let log_end = LogPayloadStorage::new(dir.path())
        .expect("test: payload store")
        .wal_position();
    save_coverage(dir.path(), &Bm25Coverage::new(marker.snapshot, log_end + 1))
        .expect("test: marker past the log");
    append_stale_removal(dir.path());

    let reopened = reopen(dir.path());
    assert_eq!(text_hits(&reopened, "world"), vec![1], "rebuilt");
    assert_eq!(text_hits(&reopened, "other"), vec![3]);
}

/// The control of the case above: the same marker at the log's end exactly
/// is trusted, so the stale WAL frame is replayed.
#[test]
fn a_marker_at_the_end_of_the_payload_log_is_trusted() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    drop(flushed_graph(dir.path()));
    let marker = load_coverage(dir.path()).expect("test: marker");
    let log_end = LogPayloadStorage::new(dir.path())
        .expect("test: payload store")
        .wal_position();
    save_coverage(dir.path(), &Bm25Coverage::new(marker.snapshot, log_end))
        .expect("test: marker at the log's end");
    append_stale_removal(dir.path());

    assert!(text_hits(&reopen(dir.path()), "world").is_empty(), "loaded");
}

/// A marker that cannot be decoded — torn by a crash mid-write, or
/// overwritten with garbage — vouches for nothing, and the index is rebuilt.
#[test]
fn a_corrupt_or_partial_marker_is_rebuilt() {
    for (case, corrupt) in [
        (
            "partial",
            (|bytes: &[u8]| bytes[..bytes.len() - 1].to_vec()) as fn(&[u8]) -> Vec<u8>,
        ),
        ("garbage", |_: &[u8]| vec![0xFF; 3]),
        ("empty", |_: &[u8]| Vec::new()),
    ] {
        let dir = tempfile::tempdir().expect("test: tempdir");
        drop(flushed_graph(dir.path()));
        let bytes = std::fs::read(coverage_path(dir.path())).expect("test: marker");
        std::fs::write(coverage_path(dir.path()), corrupt(&bytes)).expect("test: corrupt");
        assert!(load_coverage(dir.path()).is_none(), "{case}: no marker");
        append_stale_removal(dir.path());

        let reopened = reopen(dir.path());
        assert_eq!(text_hits(&reopened, "world"), vec![1], "{case}: rebuilt");
        assert!(
            load_coverage(dir.path()).is_some(),
            "{case}: the rebuild is vouched for"
        );
    }
}
