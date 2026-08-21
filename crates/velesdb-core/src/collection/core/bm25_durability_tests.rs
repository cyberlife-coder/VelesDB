//! What a document's durability actually depends on, in the BM25 index.
//!
//! The bulk write path opens and `sync_all()`s the BM25 WAL once per document
//! (#1797). Before that is batched into one sync, the consequence of losing an
//! un-synced entry has to be established by execution rather than assumed,
//! because it decides what the fix is allowed to trade away.
//!
//! Three stores hold overlapping state, and only one of them is canonical:
//!
//! * the payload store — canonical, holds the text itself;
//! * the BM25 snapshot — an O(1) cold-start accelerator (#389);
//! * the BM25 WAL — the recovery journal covering the delta SINCE that
//!   snapshot.
//!
//! `load_bm25_index` reconstructs the whole index from payloads when the
//! snapshot is ABSENT, but when a snapshot is present it loads it and replays
//! the WAL on top — and nothing rebuilds what a lost WAL entry took with it.
//! These tests pin both halves of that behaviour.

#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use serde_json::json;
use std::path::PathBuf;

const DIM: usize = 4;

/// A collection holding `n` text-bearing documents, ids `1..=n`.
fn seeded(dir: &tempfile::TempDir, n: u64) -> Collection {
    let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
        .expect("collection created");
    let points: Vec<Point> = (1..=n)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({ "content": format!("alpha document {id}") })),
            )
        })
        .collect();
    col.upsert(points).expect("seed");
    col
}

/// Whether a text search finds the document with this id.
fn found(col: &Collection, id: u64) -> bool {
    col.text_search("alpha", 100)
        .expect("text search")
        .iter()
        .any(|r| r.point.id == id)
}

/// The BM25 WAL path for a collection directory.
fn wal_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    crate::index::bm25_persistence_wal::wal_path_for_bm25(dir.path())
}

/// With no snapshot, the payload store alone restores the whole index.
///
/// This is what makes the payload canonical: deleting the WAL outright costs
/// nothing, because the rebuild path does not consult it.
#[test]
fn without_a_snapshot_the_payloads_rebuild_the_index_even_with_no_wal() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let col = seeded(&dir, 3);
        assert!(found(&col, 3), "seeded document must be searchable");
    }
    // No flush() -> no snapshot was written. Remove the WAL entirely.
    let wal = wal_path(&dir);
    if wal.exists() {
        std::fs::remove_file(&wal).expect("remove wal");
    }

    let reopened = Collection::open(PathBuf::from(dir.path())).expect("reopen");
    for id in 1..=3 {
        assert!(
            found(&reopened, id),
            "document {id} must be rebuilt from payload storage when no snapshot exists"
        );
    }
}

/// With a snapshot, a lost WAL entry is NOT rebuilt — the document stays in the
/// payload store but disappears from text search.
///
/// This is the behaviour the batched fix must not make reachable for
/// acknowledged writes. It is pinned here as the engine's current contract, not
/// endorsed: the data is recoverable in principle (the payload still holds the
/// text) but nothing recovers it automatically, and no public primitive asks
/// for a rebuild.
#[test]
fn with_a_snapshot_a_lost_wal_entry_is_not_rebuilt_from_payloads() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let col = seeded(&dir, 2);
        // `flush()` does NOT persist BM25 — only `flush_full()` reaches
        // `flush_derived_indexes` -> `flush_bm25_index`, which writes the
        // snapshot and truncates the WAL. Using the wrong one silently leaves
        // no snapshot, and the reopen then rebuilds from payloads and hides
        // exactly the behaviour under test.
        col.flush_full()
            .expect("full flush writes the BM25 snapshot");
        assert!(
            crate::index::bm25_persistence::snapshot_path(dir.path()).exists(),
            "the snapshot must exist, or this test proves nothing about the \
             snapshot-present branch"
        );

        // This third document lands in the WAL, after the snapshot.
        col.upsert(vec![Point::new(
            3,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({ "content": "alpha document 3" })),
        )])
        .expect("post-snapshot write");
        assert!(found(&col, 3), "document 3 is searchable before the loss");
    }

    // Simulate the loss of an un-synced WAL: drop the post-snapshot delta.
    let wal = wal_path(&dir);
    assert!(wal.exists(), "a post-snapshot write must have left a WAL");
    std::fs::remove_file(&wal).expect("remove wal");

    let reopened = Collection::open(PathBuf::from(dir.path())).expect("reopen");

    // The snapshot half survives.
    for id in 1..=2 {
        assert!(
            found(&reopened, id),
            "document {id} was in the snapshot and must survive"
        );
    }
    // The WAL half does not, and is not rebuilt.
    assert!(
        !found(&reopened, 3),
        "a snapshot short-circuits the payload rebuild, so a lost WAL entry stays \
         lost to text search — if this now passes, the engine gained an automatic \
         reconciliation and #1797's durability argument must be re-stated"
    );

    // But the text itself was never lost: the payload store still has it. That
    // is what makes this recoverable in principle rather than data loss.
    let payload = reopened
        .get(&[3])
        .into_iter()
        .next()
        .flatten()
        .and_then(|p| p.payload)
        .expect("document 3 is still in the payload store");
    assert_eq!(
        payload.get("content").and_then(serde_json::Value::as_str),
        Some("alpha document 3"),
        "the payload store is canonical and must still carry the text"
    );
}

// ---------------------------------------------------------------------------
// The structural claim: one batch, one durability barrier
// ---------------------------------------------------------------------------

/// A bulk insert of N text-bearing documents must cost ONE BM25 WAL sync.
///
/// Counted at the syscall, not timed: the claim is about how many durability
/// barriers a batch takes, and a stopwatch would only measure this machine's
/// SSD. `count_wal_io` filters on the BM25 context, so the graph edge WAL —
/// which shares the same framing helpers — cannot inflate the numbers.
///
/// Before the fix this fails with N opens / N flushes / N syncs, because
/// `bulk_store_payloads_inner` calls `update_text_index` per point and each
/// call opens the WAL, writes one frame and `sync_all()`s it.
#[test]
fn bulk_text_documents_use_one_wal_sync() {
    const N: u64 = 8;
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
        .expect("collection created");
    let points: Vec<Point> = (1..=N)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({ "content": format!("alpha document {id}") })),
            )
        })
        .collect();

    let (written, counts) = counted_on(&dir, || col.upsert_bulk(&points));
    assert_eq!(
        written.expect("bulk upsert"),
        usize::try_from(N).expect("fits"),
        "the batch must actually have been written"
    );

    assert_eq!(
        counts.syncs, 1,
        "a batch of {N} documents took {} fsyncs on the BM25 WAL; one batch is \
         one durability barrier, and a per-document fsync is what caps bulk \
         text insertion at roughly one fsync per point",
        counts.syncs
    );
    assert_eq!(
        counts.opens, 1,
        "a batch of {N} documents opened the BM25 WAL {} times; the append path \
         must open it once per batch",
        counts.opens
    );
    assert_eq!(
        counts.flushes, 1,
        "a batch of {N} documents flushed the BM25 WAL {} times",
        counts.flushes
    );

    // The batching must not have cost the documents themselves.
    for id in 1..=N {
        assert!(
            found(&col, id),
            "document {id} must be searchable after the batch"
        );
    }
}

/// Points carrying `content`, ids `1..=n`.
fn text_points(n: u64) -> Vec<Point> {
    (1..=n)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({ "content": format!("alpha document {id}") })),
            )
        })
        .collect()
}

/// Counts the BM25 WAL syscalls a closure performs.
fn counted_on<T>(
    dir: &tempfile::TempDir,
    f: impl FnOnce() -> T,
) -> (T, crate::index::wal_framing::io_counters::WalIoCounts) {
    crate::index::wal_framing::io_counters::count_wal_io(&wal_path(dir), f)
}

#[test]
fn bulk_text_documents_are_searchable_after_reopen() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
            .expect("created");
        col.upsert_bulk(&text_points(5)).expect("bulk");
        col.flush_full().expect("full flush");
    }
    let reopened = Collection::open(PathBuf::from(dir.path())).expect("reopen");
    for id in 1..=5 {
        assert!(found(&reopened, id), "document {id} must survive a reopen");
    }
}

/// Every document of an ACKNOWLEDGED batch is recoverable from the WAL alone.
///
/// The snapshot is written first and the batch lands after it, so the reopen
/// takes the snapshot + WAL-replay branch — the branch that does NOT fall back
/// to a payload rebuild. What is proven here is therefore the WAL's own
/// contribution, not the payload store's.
#[test]
fn every_acknowledged_document_survives_reopen() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
            .expect("created");
        col.upsert_bulk(&text_points(2)).expect("first batch");
        col.flush_full().expect("snapshot");
        assert!(
            crate::index::bm25_persistence::snapshot_path(dir.path()).exists(),
            "the snapshot must exist for this to test the WAL branch"
        );
        // Acknowledged after the snapshot: only the WAL carries these.
        let later: Vec<Point> = (3..=6)
            .map(|id| {
                Point::new(
                    id,
                    vec![1.0, 0.0, 0.0, 0.0],
                    Some(json!({ "content": format!("alpha document {id}") })),
                )
            })
            .collect();
        col.upsert_bulk(&later).expect("acknowledged batch");
    }
    let reopened = Collection::open(PathBuf::from(dir.path())).expect("reopen");
    for id in 1..=6 {
        assert!(
            found(&reopened, id),
            "document {id} was acknowledged and must be recoverable"
        );
    }
}

/// A WAL failure returns `Err` AND leaves the in-memory index untouched.
///
/// This is the ordering claim the counters cannot make. The WAL path is broken
/// by putting a DIRECTORY where `bm25.wal` belongs, so opening it for append
/// fails deterministically on any platform — no fault-injection hooks, no
/// timing. If the index were mutated before the durability barrier, the search
/// below would find the documents even though nothing was acknowledged.
#[test]
fn a_wal_failure_is_propagated_and_leaves_the_index_untouched() {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
        .expect("created");
    std::fs::create_dir(wal_path(&dir)).expect("obstruct the WAL path with a directory");

    let result = col.upsert_bulk(&text_points(4));

    assert!(
        result.is_err(),
        "a WAL that cannot be written must not be reported as success"
    );
    for id in 1..=4 {
        assert!(
            !found(&col, id),
            "document {id} was never acknowledged, so it must not be in the \
             in-memory index — mutating memory before the fsync would make an \
             unacknowledged batch visible and unrecoverable"
        );
    }
}

/// A batch with no indexable text writes nothing to the BM25 WAL.
#[test]
fn payload_without_text_does_not_write_the_bm25_wal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
        .expect("created");
    let numeric: Vec<Point> = (1..=4)
        .map(|id| Point::new(id, vec![1.0, 0.0, 0.0, 0.0], Some(json!({ "n": id }))))
        .collect();

    let (res, counts) = counted_on(&dir, || col.upsert_bulk(&numeric));
    res.expect("bulk");

    assert_eq!(
        counts.syncs, 0,
        "a payload with no indexable string must not reach the BM25 WAL"
    );
    assert!(
        !wal_path(&dir).exists(),
        "no BM25 WAL file should have been created at all"
    );
}

/// An empty batch does not touch the WAL.
#[test]
fn empty_batch_does_not_touch_the_wal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
        .expect("created");

    let (res, counts) = counted_on(&dir, || col.upsert_bulk(&[]));
    assert_eq!(res.expect("empty bulk"), 0);

    assert_eq!(counts.opens, 0, "an empty batch must not open the WAL");
    assert_eq!(counts.syncs, 0, "an empty batch must not fsync");
    assert!(!wal_path(&dir).exists(), "no WAL file for an empty batch");
}

/// `upsert` keeps its per-call durability contract at ONE barrier per call.
///
/// The contract callers rely on is "everything this call acknowledged is
/// durable when it returns" — nothing is acknowledged between the points of
/// one call, so per-document barriers bought no guarantee, only N-1 extra
/// fsyncs (#1797). The whole batch now rides a single `wal_append_batch`
/// barrier, same as `upsert_bulk`.
#[test]
fn upsert_call_gets_one_durability_barrier() {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
        .expect("created");

    let (res, counts) = counted_on(&dir, || col.upsert(text_points(3)));
    res.expect("upsert");

    assert_eq!(
        counts.syncs, 1,
        "one durability barrier covers the whole upsert call's BM25 batch"
    );
}

/// Re-indexing an existing id keeps the last text, as before.
#[test]
fn duplicate_document_updates_keep_existing_semantics() {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
        .expect("created");

    col.upsert_bulk(&[Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(json!({ "content": "alpha original" })),
    )])
    .expect("first");
    col.upsert_bulk(&[Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(json!({ "content": "beta replacement" })),
    )])
    .expect("second");

    let beta = col.text_search("beta", 10).expect("search");
    assert!(
        beta.iter().any(|r| r.point.id == 1),
        "the replacement text must be searchable"
    );
}

/// A torn final frame does not cost the complete frames written before it.
#[test]
fn a_truncated_final_frame_preserves_prior_complete_frames() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let col = Collection::create(PathBuf::from(dir.path()), DIM, DistanceMetric::Cosine)
            .expect("created");
        col.upsert_bulk(&text_points(2)).expect("seed");
        col.flush_full().expect("snapshot");
        // These land in the WAL, in one batch, as complete frames.
        let later: Vec<Point> = (3..=5)
            .map(|id| {
                Point::new(
                    id,
                    vec![1.0, 0.0, 0.0, 0.0],
                    Some(json!({ "content": format!("alpha document {id}") })),
                )
            })
            .collect();
        col.upsert_bulk(&later).expect("batch");
    }

    // Tear the tail: drop the last few bytes, leaving a partial final frame.
    let wal = wal_path(&dir);
    let len = std::fs::metadata(&wal).expect("wal metadata").len();
    assert!(len > 8, "the WAL must hold several frames to be torn");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&wal)
        .expect("open wal");
    file.set_len(len - 3).expect("truncate the final frame");

    let reopened = Collection::open(PathBuf::from(dir.path())).expect("reopen despite a torn tail");
    for id in 1..=2 {
        assert!(found(&reopened, id), "snapshot document {id} must survive");
    }
    assert!(
        found(&reopened, 3),
        "a complete frame written before the torn one must still replay"
    );
}

// ---------------------------------------------------------------------------
// Throughput of the bulk text path (#1797), measured — not asserted from theory
// ---------------------------------------------------------------------------

/// Bulk text insertion throughput across dimensions, volumes and batch sizes.
///
/// `#[ignore]`d: it writes tens of thousands of documents and takes minutes.
/// Run deliberately, on a machine at rest, in `--release`, single-threaded —
/// the sync counts are only exact when nothing else writes a BM25 WAL:
///
/// ```text
/// cargo test --release -p velesdb-core --lib bm25_batch_throughput \
///   -- --ignored --nocapture --test-threads=1
/// ```
///
/// The same body is run before and after the fix (by temporarily restoring the
/// per-document call in `bulk_store_payloads_inner`), so the two columns are
/// produced by identical measurement code on the same machine.
#[test]
#[ignore = "writes tens of thousands of documents; run deliberately, on a machine at rest"]
fn bm25_batch_throughput() {
    for dim in [4usize, 1024] {
        for volume in [1_000u64, 4_000, 16_000] {
            for batch in [1usize, 64, 256, 1024] {
                let dir = tempfile::tempdir().expect("temp dir");
                let col =
                    Collection::create(PathBuf::from(dir.path()), dim, DistanceMetric::Cosine)
                        .expect("created");
                let points: Vec<Point> = (1..=volume)
                    .map(|id| {
                        Point::new(
                            id,
                            vec![1.0; dim],
                            Some(json!({ "content": format!("alpha document {id}") })),
                        )
                    })
                    .collect();

                let start = std::time::Instant::now();
                let ((), counts) = counted_on(&dir, || {
                    for chunk in points.chunks(batch) {
                        col.upsert_bulk(chunk).expect("bulk");
                    }
                });
                let elapsed = start.elapsed();

                let micros =
                    u32::try_from(elapsed.as_micros()).map_or(f64::from(u32::MAX), f64::from);
                let per_doc = micros / f64::from(u32::try_from(volume).expect("fits"));
                println!(
                    "  dim={dim:<5} n={volume:<6} batch={batch:<5} {elapsed:>10.2?} \
                     {per_doc:>9.1} us/doc {:>9.0} doc/s  opens={:<6} flushes={:<6} syncs={}",
                    1_000_000.0 / per_doc,
                    counts.opens,
                    counts.flushes,
                    counts.syncs
                );
            }
        }
    }
}

/// The same store, reopened, still answers — checked once at a realistic
/// dimension so the throughput table above cannot be green on a broken index.
#[test]
#[ignore = "writes 4 000 documents at dimension 1024"]
fn bm25_batch_throughput_survives_reopen() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let col = Collection::create(PathBuf::from(dir.path()), 1024, DistanceMetric::Cosine)
            .expect("created");
        let points: Vec<Point> = (1..=4_000)
            .map(|id| {
                Point::new(
                    id,
                    vec![1.0; 1024],
                    // A token unique to each document: searching for the shared
                    // word would only return the top-k of 4 000 equal matches,
                    // and a document outside that window would look lost when
                    // it is merely outranked.
                    Some(json!({ "content": format!("alpha doc{id} document") })),
                )
            })
            .collect();
        for chunk in points.chunks(1024) {
            col.upsert_bulk(chunk).expect("bulk");
        }
        col.flush_full().expect("full flush");
    }
    let reopened = Collection::open(PathBuf::from(dir.path())).expect("reopen");
    for id in [1u64, 2_000, 4_000] {
        let hits = reopened
            .text_search(&format!("doc{id}"), 10)
            .expect("text search");
        assert!(
            hits.iter().any(|r| r.point.id == id),
            "document {id} must survive the reopen"
        );
    }
}
