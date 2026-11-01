//! A collection called from an application's own rayon job.
//!
//! An application may call a collection from a job on its own rayon pool. A
//! rayon worker that waits on another pool's `install` keeps running its own
//! pool's jobs meanwhile (`rayon-core`'s `in_worker_cross`); if that worker
//! holds a collection guard, and one of those jobs reads the collection behind
//! a queued writer, `parking_lot` parks the job on the worker the guard holder
//! needs, and the whole process stops. No thread holding a collection guard
//! may wait on any pool when it is itself a rayon worker.

use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const DIM: usize = 64;
const POINTS: u64 = 1_000;

/// How long the probe may take before the test calls it a deadlock. A bound
/// on failure only: the wait ends as soon as the probe reports.
const DEADLINE: Duration = Duration::from_secs(120);

/// A distinct vector per id (a splitmix-style hash of `id` and the
/// dimension, mapped to [-1, 1)).
fn vector(id: u64) -> Vec<f32> {
    (0..DIM as u64)
        .map(|d| {
            let mut x =
                id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ d.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x ^= x >> 31;
            x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^= x >> 29;
            #[allow(clippy::cast_precision_loss)] // Reason: 24 bits fit an f32 exactly.
            let unit = (x >> 40) as f32 / (1_u64 << 24) as f32;
            unit * 2.0 - 1.0
        })
        .collect()
}

/// A merge of `merged` stored-but-unindexed ids, run from a job on a
/// one-worker application pool, while that pool's other job reads the
/// collection behind a writer queued on the merge's storage guard.
///
/// The interleaving is staged, never timed. `rayon::join` runs its first arm
/// inline on the worker and queues the second, so the merge runs first and
/// the reader leg is the job an `in_worker_cross` wait would steal. The
/// merge signals through `merge_hooks::set_after_read` once it holds the
/// `vector_storage` read guard — where a racing writer then queues — and
/// the reader leg reads only once the writer is provably queued:
/// `try_read` going `None` means a writer took the writer slot, whether it
/// runs or still waits for the merge's readers to drain.
///
/// Returns `Err` when the probe did not finish before `DEADLINE`.
fn merge_from_an_application_job(merged: u64) -> Result<(), mpsc::RecvTimeoutError> {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = Collection::create(dir.path().join("c"), DIM, DistanceMetric::Cosine)
        .expect("test: collection");
    coll.upsert((0..POINTS).map(|id| Point::without_payload(id, vector(id))))
        .expect("test: load");
    let ids: Vec<u64> = (0..merged).collect();
    coll.remove_from_index_for_bench(&ids);

    let (guard_held_tx, guard_held_rx) = mpsc::channel();
    let writer_done = Arc::new(AtomicBool::new(false));
    let writer = {
        let coll = coll.clone();
        let writer_done = writer_done.clone();
        std::thread::spawn(move || {
            // The merge signals once it holds the storage read guard, so the
            // upsert's `vector_storage.write()` queues behind it.
            let _ = guard_held_rx.recv();
            let result = coll.upsert([Point::without_payload(POINTS, vector(POINTS))]);
            writer_done.store(true, Ordering::Release);
            result.expect("test: writer");
        })
    };

    let (done_tx, done_rx) = mpsc::channel();
    let probe = {
        let coll = coll.clone();
        std::thread::spawn(move || {
            crate::pools::application_pool(1).install(|| {
                rayon::join(
                    || {
                        crate::collection::search::pending::merge_hooks::set_after_read(
                            move || {
                                let _ = guard_held_tx.send(());
                            },
                        );
                        coll.index_current(&ids)
                    },
                    || {
                        while !writer_done.load(Ordering::Acquire)
                            && coll.storage.vector_storage.try_read().is_some()
                        {
                            std::thread::yield_now();
                        }
                        let read = coll.get(&[1]);
                        writer.join().expect("test: writer thread");
                        read
                    },
                )
            });
            let _ = done_tx.send(());
        })
    };
    let finished = done_rx.recv_timeout(DEADLINE);
    if finished.is_ok() {
        probe.join().expect("test: probe thread");
    }
    // A deadlocked probe is left behind: nothing can unpark it.
    drop(dir);
    finished
}

/// A merge large enough for the graph's dedicated pool (256 ids, over
/// `PARALLEL_BATCH_MIN`), run from an application's rayon job, finishes:
/// the merge must not wait on any pool while it holds the storage guard.
#[test]
fn a_merge_from_an_application_rayon_job_finishes() {
    merge_from_an_application_job(256)
        .expect("the merge waited on a pool from a rayon worker, holding the storage guard");
}
