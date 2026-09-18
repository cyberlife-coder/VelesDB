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

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::native::PARALLEL_BATCH_MIN;
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
#[test]
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
/// sequentially there, and paying `install`'s flat thread hand-off (~32 us
/// measured, against 541 ns of work for ten vectors) would be a latency
/// regression on every small insert. If that sub-threshold path ever reached
/// the global pool, the skip would become a deadlock instead of an
/// optimisation, and this test is what says so.
#[test]
fn a_sub_threshold_insert_places_while_every_global_rayon_worker_is_parked() {
    let inserted = place_with_every_global_worker_parked(POINTS_BELOW_THRESHOLD)
        .expect("a sub-threshold insert reached the global rayon pool: it placed nothing in time");
    assert_eq!(
        inserted, POINTS_BELOW_THRESHOLD,
        "every vector in the small batch is placed"
    );
}
