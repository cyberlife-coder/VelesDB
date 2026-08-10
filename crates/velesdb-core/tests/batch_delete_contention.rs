#![cfg(feature = "persistence")]
//! Batch-delete durability and lock-contention tests (finding C3).
//!
//! The batch delete path used to pay ~3 synchronous fsyncs PER POINT
//! (vector WAL + payload WAL + BM25 WAL) while holding every collection
//! write lock, wedging all concurrent readers and writers for the whole
//! batch. The fix pays one durability barrier per touched store per batch.
//!
//! Two guarantees are pinned here:
//! 1. Correctness: a batch delete is durable across reopen, and surviving
//!    points remain intact (vector + payload).
//! 2. Contention: while a large batch delete runs on one thread, a
//!    concurrent reader completes within a generous bound instead of being
//!    wedged behind O(N) fsyncs. A watchdog (`recv_timeout`) guarantees the
//!    test itself can never hang.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::quantization::StorageMode;
use velesdb_core::{Point, VectorCollection};

/// Deterministic vector so survivors can be checked byte-for-byte.
fn make_vector(seed: u64, dimension: usize) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)] // test data generation only
    (0..dimension)
        .map(|i| ((seed as f32) * 0.3 + (i as f32) * 0.1).sin())
        .collect()
}

/// Points `0..n` with a text-bearing payload (exercises the BM25 WAL path).
fn make_points(n: u64, dimension: usize) -> Vec<Point> {
    (0..n)
        .map(|id| {
            Point::new(
                id,
                make_vector(id, dimension),
                Some(json!({ "title": format!("document {id}"), "rank": id })),
            )
        })
        .collect()
}

fn create_collection(dir: &std::path::Path, dimension: usize) -> VectorCollection {
    VectorCollection::create(
        dir.to_path_buf(),
        "batch_delete_test",
        dimension,
        DistanceMetric::Euclidean,
        StorageMode::Full,
    )
    .expect("create collection")
}

/// Batch-deleting half the points must survive a drop + reopen: deleted ids
/// stay gone (their WAL tombstones were fsynced by the delete itself — no
/// explicit `flush()` after the delete on purpose), survivors keep their
/// exact vector and payload.
#[test]
fn batch_delete_persists_across_reopen() {
    let dir = TempDir::new().expect("tempdir");
    let coll_dir = dir.path().join("coll");
    let dimension = 8;
    let total: u64 = 200;
    let deleted: Vec<u64> = (0..total / 2).collect();
    let survivors: Vec<u64> = (total / 2..total).collect();

    {
        let coll = create_collection(&coll_dir, dimension);
        coll.upsert_bulk(&make_points(total, dimension))
            .expect("upsert");
        coll.flush().expect("flush after upsert");
        coll.delete(&deleted).expect("batch delete");
        // Intentionally NO flush here: the delete barrier itself must have
        // made the tombstones durable before `delete()` returned.
    }

    let coll = VectorCollection::open(coll_dir).expect("reopen");

    for (id, got) in deleted.iter().zip(coll.get(&deleted)) {
        assert!(got.is_none(), "deleted point {id} resurrected after reopen");
    }
    for (id, got) in survivors.iter().copied().zip(coll.get(&survivors)) {
        let point = got.unwrap_or_else(|| panic!("survivor {id} lost after reopen"));
        assert_eq!(
            point.vector,
            make_vector(id, dimension),
            "vector of {id} changed"
        );
        let payload = point.payload.expect("survivor payload lost");
        assert_eq!(payload["rank"], json!(id), "payload of {id} changed");
    }
    assert_eq!(
        coll.all_point_ids(),
        survivors,
        "storage id set diverged after reopen"
    );
}

/// Generous ceiling for one concurrent `get()` racing a large batch delete.
///
/// Post-fix the delete holds the write locks for the in-memory batch work
/// plus THREE fsyncs total, so the reader completes in well under a second
/// even on slow disks. Pre-fix the same delete held the locks across
/// ~3 fsyncs per victim (~24000 fsyncs here) and the reader was wedged
/// for the full duration — an order of magnitude (or more) past this bound.
const CONCURRENT_READ_BOUND: Duration = Duration::from_secs(5);

/// Hard watchdog so a regression can never hang the suite.
const WATCHDOG: Duration = Duration::from_secs(300);

/// While a batch delete of thousands of points runs on one thread, a reader
/// on another thread must complete within [`CONCURRENT_READ_BOUND`].
#[test]
fn concurrent_reader_completes_during_batch_delete() {
    let dir = TempDir::new().expect("tempdir");
    let coll_dir = dir.path().join("coll");
    let dimension = 16;
    let total: u64 = 8000;
    let keeper = total - 1;
    let victims: Vec<u64> = (0..keeper).collect();

    let coll = create_collection(&coll_dir, dimension);
    coll.upsert_bulk(&make_points(total, dimension))
        .expect("upsert");
    coll.flush().expect("flush after upsert");
    let coll = Arc::new(coll);

    let deleter = {
        let coll = Arc::clone(&coll);
        thread::spawn(move || {
            let started = Instant::now();
            coll.delete(&victims).expect("batch delete");
            started.elapsed()
        })
    };

    let (tx, rx) = mpsc::channel();
    let reader = {
        let coll = Arc::clone(&coll);
        thread::spawn(move || {
            // Give the delete thread a head start so the read overlaps the
            // lock-holding window instead of sneaking in before it.
            thread::sleep(Duration::from_millis(100));
            let started = Instant::now();
            let got = coll.get(&[keeper]);
            let latency = started.elapsed();
            tx.send((latency, got[0].is_some())).expect("send result");
        })
    };

    // Watchdog: `recv_timeout` guarantees the test fails loudly instead of
    // hanging if the reader is wedged behind the delete.
    let (read_latency, keeper_visible) = rx
        .recv_timeout(WATCHDOG)
        .expect("concurrent reader wedged behind batch delete (watchdog hit)");
    let delete_duration = deleter.join().expect("deleter thread panicked");
    reader.join().expect("reader thread panicked");

    eprintln!(
        "batch delete of {keeper} points: {delete_duration:?}; \
         concurrent get latency: {read_latency:?}"
    );
    assert!(keeper_visible, "surviving point invisible during delete");
    assert!(
        read_latency < CONCURRENT_READ_BOUND,
        "concurrent reader took {read_latency:?} (bound {CONCURRENT_READ_BOUND:?}) — \
         batch delete is holding write locks across per-point fsyncs again"
    );
}
