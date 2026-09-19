//! No operation that holds `HnswIndex::inner.read()` waits on the **global**
//! rayon pool.
//!
//! That wait is the one edge #2343's cycle cannot close without: a `vacuum`
//! asking for `inner.write()` blocks every new reader, so a global worker that
//! takes `inner.read()` — `rerank_candidates_simd`, the per-query search —
//! parks; the guard holder never gets the worker its join needs, and the guard
//! the writer waits on is never released. Nobody advances.
//!
//! Each test below parks every global worker first, deterministically, and
//! then asks the operation to finish anyway. An operation that joins on the
//! global pool cannot, whatever the machine's timing; one that keeps its rayon
//! work on a pool of its own is unaffected. Work is not stolen across rayon
//! pools, so the isolation these tests assert is the isolation that holds in
//! production.

//! # Why these tests are `#[serial]`
//!
//! Each one parks EVERY global rayon worker, which is process-global state.
//! Run in parallel with each other, the first takes them all and the second
//! waits for a worker that never comes: its premise times out at `DEADLINE`,
//! and for those 60 seconds every other test in the binary that needs the
//! global pool is starved too. Measured, not feared — the pre-commit hook
//! runs `cargo test --workspace --lib` with no `--test-threads=1`, and it
//! failed with `gpu_rerank_tests::test_batch_search_*` "running for over 60
//! seconds" and a SIGABRT.
//!
//! `#[serial]` is what fixes that, and only that: as
//! `alloc_guard_tests.rs` records for the allocation ceiling, it excludes
//! other `#[serial]` tests and nothing else. It is enough here because each
//! of these tests holds the parking for milliseconds (0.05 s for all three) --
//! the 60-second stall came from two of them deadlocking, not from the
//! parking itself. `link_pool_tests.rs`, the single test this file replaces,
//! needed no annotation for exactly that reason: there was nothing for it to
//! race.
//!
//! Moving them to their own test binary -- the repository's other answer to
//! process-global state -- would cost more than it buys: they would leave
//! `--lib`, and `cargo mutants` runs `-- --lib`, so the mutants they kill
//! (including `replace >= with <` on the threshold) would start surviving.

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::native::PARALLEL_BATCH_MIN;
use serial_test::serial;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// Over `PARALLEL_BATCH_MIN`, so the insert takes the parallel connect path
/// and joins on a pool instead of placing node by node.
const POINTS: usize = 2 * PARALLEL_BATCH_MIN;

/// Under `PARALLEL_BATCH_MIN`, so the insert places node by node and enters no
/// rayon pool at all. Derived from the same constant as `POINTS`: a threshold
/// change moves both, and neither test can silently land on the wrong side.
const POINTS_BELOW_THRESHOLD: usize = PARALLEL_BATCH_MIN - 1;
const DIM: usize = 8;

/// How long a step may take before the test calls it a hang. A bound on
/// failure, not a synchronization delay: the wait below ends as soon as the
/// insert reports.
const DEADLINE: Duration = Duration::from_secs(60);

/// Parks every global rayon worker and returns the senders that release them.
///
/// Returns once every worker has confirmed it is parked, so the premise holds
/// before the test acts — no sleep stands in for that confirmation. Dropping
/// the returned senders unparks every job, whatever order rayon ran them in.
fn park_every_global_worker() -> Vec<mpsc::Sender<()>> {
    let workers = rayon::current_num_threads();
    let (started_tx, started_rx) = mpsc::channel();
    let mut release: Vec<mpsc::Sender<()>> = Vec::with_capacity(workers);
    for _ in 0..workers {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        release.push(release_tx);
        let started_tx = started_tx.clone();
        rayon::spawn(move || {
            started_tx
                .send(())
                .expect("test: the receiver outlives this");
            // Err as soon as the test drops its sender: that is the release.
            let _released = release_rx.recv();
        });
    }
    drop(started_tx);
    for _ in 0..workers {
        started_rx
            .recv_timeout(DEADLINE)
            .expect("premise: every global rayon worker parks");
    }
    release
}

/// `count` vectors, spread far enough apart that the graph keeps them
/// distinct.
fn batch_vectors(count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|id| {
            (0..DIM)
                .map(|j| {
                    let k = u16::try_from(id * DIM + j).expect("test: fits a u16");
                    f32::from(k) * 0.015_625
                })
                .collect()
        })
        .collect()
}

/// Places `count` vectors through `insert_batch_parallel` with every global
/// rayon worker parked, and returns what it placed.
///
/// `Err` means the insert did not report before `DEADLINE`: it waited on a
/// worker the parked jobs never give back. The workers are released before
/// the thread is joined either way, so a stuck insert ends the test instead of
/// hanging it.
fn place_with_every_global_worker_parked(count: usize) -> Result<usize, mpsc::RecvTimeoutError> {
    let release = park_every_global_worker();

    let index = Arc::new(HnswIndex::new(DIM, DistanceMetric::Euclidean).expect("test: index"));
    let (inserted_tx, inserted_rx) = mpsc::channel();
    let placing_thread = std::thread::spawn(move || {
        let vectors = batch_vectors(count);
        let batch: Vec<(u64, &[f32])> = vectors
            .iter()
            .enumerate()
            .map(|(id, v)| (u64::try_from(id).expect("test: fits a u64"), v.as_slice()))
            .collect();
        let inserted = index.insert_batch_parallel(batch);
        inserted_tx
            .send(inserted)
            .expect("test: the receiver outlives this");
    });

    let inserted = inserted_rx.recv_timeout(DEADLINE);
    drop(release);
    placing_thread.join().expect("test: placing thread");
    inserted
}

/// With every global rayon worker parked, `insert_batch_parallel` still places
/// its batch: its connect phase runs on a pool of its own.
///
/// Joined on the global pool — the shape #2343 describes — it would wait for a
/// worker the parked jobs never give back, holding `inner.read()` the whole
/// time. That is exactly the guard a concurrent `vacuum` writer is waiting on.
#[test]
#[serial]
fn insert_batch_parallel_places_while_every_global_rayon_worker_is_parked() {
    let inserted = place_with_every_global_worker_parked(POINTS)
        .expect("insert_batch_parallel waited on the global rayon pool: it placed nothing in time");
    assert_eq!(inserted, POINTS, "every vector in the batch is placed");
}

/// The same rule for the drain (#2290): with every global rayon worker parked,
/// `link_placed` still links its batch.
///
/// This file replaces `link_pool_tests.rs`, which asserted the same thing for
/// the drain alone: the rule is one rule over both holders, and the pool it
/// names is no longer the drain's own.
/// `link_placed` is `#[cfg(feature = "persistence")]`, so this test carries
/// the same gate. The module moved to a plain `cfg(test)` when it stopped
/// being about the drain alone; without this line the file would name a
/// method that does not exist in a build without that feature. Inert today —
/// `batch.rs` imports `rayon` unconditionally and `rayon` is `optional`, so
/// the crate never builds without `persistence` — but a gate gap that only
/// stays closed by accident is one nobody will notice closing.
#[cfg(feature = "persistence")]
#[test]
#[serial]
fn link_placed_links_while_every_global_rayon_worker_is_parked() {
    use crate::index::hnsw::direct_writer::DirectVectorWriter;

    let release = park_every_global_worker();

    let index = Arc::new(HnswIndex::new(DIM, DistanceMetric::Euclidean).expect("test: index"));
    let vectors = batch_vectors(POINTS);
    let batch: Vec<(u64, &[f32])> = vectors
        .iter()
        .enumerate()
        .map(|(id, v)| (u64::try_from(id).expect("test: fits a u64"), v.as_slice()))
        .collect();
    DirectVectorWriter::new(&index)
        .write_batch_direct(&batch)
        .expect("test: direct write")
        .expect("test: an index with exact-distance features on places");
    let ids: Vec<u64> = batch.iter().map(|&(id, _)| id).collect();

    let (linked_tx, linked_rx) = mpsc::channel();
    let draining_thread = {
        let index = Arc::clone(&index);
        std::thread::spawn(move || {
            let linked = index.link_placed(&ids);
            linked_tx
                .send(linked)
                .expect("test: the receiver outlives this");
        })
    };

    let linked = linked_rx.recv_timeout(DEADLINE);
    drop(release);
    draining_thread.join().expect("test: draining thread");

    let linked = linked
        .expect("link_placed waited on the global rayon pool: it linked nothing in time")
        .expect("test: link the placed ids");
    assert_eq!(linked, POINTS, "every placed id is linked");
}

/// A batch under `PARALLEL_BATCH_MIN` places node by node, so it takes no pool
/// — and with every global worker parked it must still finish.
///
/// This pins the coupling the fast path rests on. `insert_batch_parallel`
/// skips the dedicated pool below the threshold because `place_batch` places
/// sequentially there, and paying `install`'s flat thread hand-off for a pool
/// it never enters is cost for nothing — most of all on a tiny batch, where
/// the hand-off dwarfs the placement (see `batch.rs` for the measured
/// figures). If that sub-threshold path ever reached
/// the global pool, the skip would become a deadlock instead of an
/// optimisation, and this test is what says so.
#[test]
#[serial]
fn a_sub_threshold_insert_places_while_every_global_rayon_worker_is_parked() {
    let inserted = place_with_every_global_worker_parked(POINTS_BELOW_THRESHOLD)
        .expect("a sub-threshold insert reached the global rayon pool: it placed nothing in time");
    assert_eq!(
        inserted, POINTS_BELOW_THRESHOLD,
        "every vector in the small batch is placed"
    );
}

/// The precondition is a check, and a check nobody trips is not one.
///
/// Calling from a rayon worker is the shape that re-closes \#2343 by stealing
/// (`graph_pool`'s doc has the `rayon-core` quote), so the `debug_assert!`
/// that refuses it must be shown refusing. Without this, a wrong predicate --
/// `.is_some()` instead of `.is_none()` -- would pass the whole suite: the
/// three tests above only ever reach the assert from a plain thread, where it
/// holds either way.
///
/// The batch is over `PARALLEL_BATCH_MIN` on purpose: only that path installs
/// the pool, and only it carries the precondition. The sub-threshold case is
/// the opposite claim and has its own test above.
#[test]
#[serial]
#[should_panic(expected = "rayon worker")]
fn calling_from_a_rayon_worker_is_refused() {
    let index = HnswIndex::new(DIM, DistanceMetric::Euclidean).expect("test: index");
    let vectors = batch_vectors(POINTS);
    let batch: Vec<(u64, &[f32])> = vectors
        .iter()
        .enumerate()
        .map(|(id, v)| (u64::try_from(id).expect("test: fits a u64"), v.as_slice()))
        .collect();

    // A pool of its own, so the refusal is what fails the test rather than
    // this call competing with whatever else holds the global pool.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("test: probe pool");
    pool.install(|| index.insert_batch_parallel(batch));
}
