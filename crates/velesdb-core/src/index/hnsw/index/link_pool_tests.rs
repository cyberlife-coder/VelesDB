//! The drain's connect phase runs on its own rayon pool, never the global
//! one: it joins while holding `HnswIndex::inner.read()`, and a global worker
//! that takes that same lock closes the deadlock cycle of #2343.

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::direct_writer::DirectVectorWriter;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// Over `PARALLEL_BATCH_MIN` (100), so the drain takes the parallel connect
/// path and joins on a pool instead of linking node by node.
const POINTS: usize = 200;
const DIM: usize = 8;

/// How long a step may take before the test calls it a hang. A bound on
/// failure, not a synchronization delay: every wait below ends as soon as the
/// thing it watches reports.
const DEADLINE: Duration = Duration::from_secs(120);

/// Fills the arena with unlinked slots the way `upsert_bulk`'s direct writer
/// does, and returns the ids to drain.
fn place_unlinked(index: &HnswIndex) -> Vec<u64> {
    let vectors: Vec<Vec<f32>> = (0..POINTS)
        .map(|id| {
            (0..DIM)
                .map(|j| {
                    let k = u16::try_from(id * DIM + j).expect("test: fits a u16");
                    f32::from(k) * 0.015_625
                })
                .collect()
        })
        .collect();
    let batch: Vec<(u64, &[f32])> = vectors
        .iter()
        .enumerate()
        .map(|(id, v)| (u64::try_from(id).expect("test: fits a u64"), v.as_slice()))
        .collect();
    DirectVectorWriter::new(index)
        .write_batch_direct(&batch)
        .expect("test: direct write")
        .expect("test: an index with exact-distance features on places");
    batch.iter().map(|&(id, _)| id).collect()
}

/// With every global rayon worker parked, the drain still links its batch:
/// its connect phase runs on a pool of its own. Joined on the global pool —
/// the shape #2343 describes — the drain would wait for a worker the parked
/// jobs never give back, holding `inner.read()` the whole time.
#[test]
fn the_drain_links_while_every_global_rayon_worker_is_parked() {
    let workers = rayon::current_num_threads();
    let (started_tx, started_rx) = mpsc::channel();
    // One release sender per parked job: dropping them all unparks every job,
    // whatever order rayon ran them in.
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

    let index = Arc::new(HnswIndex::new(DIM, DistanceMetric::Euclidean).expect("test: index"));
    let ids = place_unlinked(&index);
    let (linked_tx, linked_rx) = mpsc::channel();
    let drain = {
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
    drain.join().expect("test: drain thread");

    let linked = linked
        .expect("the drain waited on the global rayon pool: it linked nothing in time")
        .expect("test: link the placed ids");
    assert_eq!(linked, POINTS, "every placed id is linked");
}
