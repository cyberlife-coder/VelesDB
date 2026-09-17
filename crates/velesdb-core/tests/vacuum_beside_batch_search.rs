#![cfg(feature = "persistence")]
//! A vacuum finishes while batch searches keep rayon busy (#2262).
//!
//! `HnswIndex::vacuum` copies the writes made during its rebuild into the new
//! graph. It used to copy them under the index write guard with a parallel
//! insert, which runs on rayon once a hundred writes are waiting. The batch
//! searches of `search_batch_parallel` run on the same pool and take the
//! index read guard: every worker parked on the guard the vacuum held, the
//! insert never ran, and the vacuum and every search hung for ever. The fix
//! copies the writes before taking the write guard, and copies what is left
//! under it one insert at a time.
//!
//! # Why an integration test, with a global pool of two threads
//!
//! A hang wedges rayon's global pool, which every test of a binary shares.
//! Alone in its own binary, this test builds that pool itself, small enough
//! for two batch searches to hold every worker, and a hang can only take this
//! binary down.
//!
//! # Anti-hang guard
//!
//! Each vacuum runs on its own thread and reports through a channel read with
//! a timeout. A vacuum that does not report in time is a hang: the test
//! prints what it saw and exits the process with a failure code, since the
//! parked threads can never be joined.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use velesdb_core::distance::DistanceMetric;
use velesdb_core::index::HnswIndex;
use velesdb_core::{SearchQuality, VectorIndex};

const DIMENSION: usize = 16;
const INDEXED: u64 = 4_000;
/// Writes that must race one vacuum before the test counts it: well above
/// the hundred at which the old copy ran on rayon.
const RACING_WRITES: u64 = 500;
/// Vacuums tried before the test gives up on racing `RACING_WRITES` writes
/// against one of them.
const ATTEMPTS: usize = 5;
/// A vacuum of this index takes seconds; one that has not reported by then
/// is parked for good.
const HANG_BOUND: Duration = Duration::from_secs(120);

fn vector(id: u64) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)] // test data generation only
    (0..DIMENSION)
        .map(|i| ((id as f32) * 0.31 + (i as f32) * 0.17).sin())
        .collect()
}

/// Prints `msg` from a fresh thread: libtest captures the test thread's
/// stderr and would swallow it before `process::exit`.
fn print_to_real_stderr(msg: String) {
    let _ = thread::spawn(move || eprintln!("{msg}")).join();
}

#[test]
fn a_vacuum_carrying_writes_finishes_beside_batch_searches() {
    // Ignored if another test of this binary already started the pool, which
    // none does.
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build_global();

    let index = Arc::new(HnswIndex::new(DIMENSION, DistanceMetric::Euclidean).unwrap());
    for id in 0..INDEXED {
        index.insert(id, &vector(id));
    }
    for id in (0..INDEXED).step_by(8) {
        index.remove(id);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let searches = {
        let (index, stop) = (Arc::clone(&index), Arc::clone(&stop));
        thread::spawn(move || {
            let queries: Vec<Vec<f32>> = (0..32).map(vector).collect();
            let refs: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();
            while !stop.load(Ordering::Acquire) {
                let _ = index.search_batch_parallel(&refs, 10, SearchQuality::Balanced);
            }
        })
    };

    let next_id = Arc::new(AtomicU64::new(INDEXED));
    let mut raced = Vec::new();
    for attempt in 0..ATTEMPTS {
        let vacuuming = Arc::new(AtomicBool::new(true));
        let (report, reported) = mpsc::channel();
        let vacuum = {
            let (index, vacuuming) = (Arc::clone(&index), Arc::clone(&vacuuming));
            thread::spawn(move || {
                let result = index.vacuum();
                vacuuming.store(false, Ordering::Release);
                let _ = report.send(result);
            })
        };
        let writes = {
            let (index, vacuuming, next_id) = (
                Arc::clone(&index),
                Arc::clone(&vacuuming),
                Arc::clone(&next_id),
            );
            thread::spawn(move || {
                let mut made = 0_u64;
                while vacuuming.load(Ordering::Acquire) {
                    let id = next_id.fetch_add(1, Ordering::AcqRel);
                    index.insert(id, &vector(id));
                    made += 1;
                }
                made
            })
        };

        let Ok(result) = reported.recv_timeout(HANG_BOUND) else {
            print_to_real_stderr(format!(
                "HANG: vacuum {attempt} did not finish within {HANG_BOUND:?} beside batch \
                 searches on a {}-thread rayon pool; vacuums finished before it, with the \
                 writes that raced each: {raced:?}",
                rayon::current_num_threads(),
            ));
            std::process::exit(1);
        };
        vacuum.join().expect("test: the vacuuming thread panicked");
        let made = writes.join().expect("test: the writing thread panicked");
        assert!(result.is_ok(), "vacuum {attempt}: {result:?}");
        raced.push(made);
        if made >= RACING_WRITES {
            break;
        }
    }
    stop.store(true, Ordering::Release);
    searches
        .join()
        .expect("test: the searching thread panicked");

    // The positive control: a vacuum no write raced proves nothing about the
    // copy of the writes.
    assert!(
        raced.iter().any(|&made| made >= RACING_WRITES),
        "no vacuum raced {RACING_WRITES} writes in {ATTEMPTS} attempts: {raced:?}"
    );
    let written = next_id.load(Ordering::Acquire);
    let scanned: HashSet<u64> = index
        .search_brute_force(&vector(0), index.len())
        .expect("test: exhaustive scan")
        .iter()
        .map(|hit| hit.id)
        .collect();
    let lost: Vec<u64> = (INDEXED..written)
        .filter(|id| !scanned.contains(id))
        .collect();
    assert!(
        lost.is_empty(),
        "{} of {} ids written during the vacuums missing from an exhaustive scan \
         (first {:?})",
        lost.len(),
        written - INDEXED,
        lost.first()
    );
}
