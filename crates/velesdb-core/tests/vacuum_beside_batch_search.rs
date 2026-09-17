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
//! # Why the racing writes come in batches
//!
//! The copy made under the write guard only reaches rayon at a hundred
//! vectors. The writes must therefore arrive in batches of at least that
//! many: a batch maps its ids in one go, after the last catch-up round looked
//! and before the guard is granted, so the copy under the guard covers a
//! whole batch whatever the catch-up managed to carry. One id at a time, the
//! catch-up left a handful, the copy took the batch path's sequential branch,
//! and a swap copying on rayon finished like a correct one.
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
//! writes what it saw to the process's stderr and exits with a failure code,
//! since the parked threads can never be joined.

use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use velesdb_core::distance::DistanceMetric;
use velesdb_core::index::HnswIndex;
use velesdb_core::{SearchQuality, VectorIndex};

const DIMENSION: usize = 16;
const INDEXED: u64 = 4_000;
/// Ids each racing write carries, at least the hundred at or above which the
/// graph's batch path places on rayon.
///
/// This is what makes the test proof of its invariant rather than of the
/// catch-up's zeal. The batch in flight when the vacuum asks for the write
/// guard maps its ids after the last catch-up round looked and before the
/// guard is granted, so the copy the swap makes covers at least one whole
/// batch. Written one id at a time, that copy stayed under a hundred, and a
/// swap copying on rayon — the deadlock this test exists for — went through
/// the batch path's sequential branch and finished.
const RACING_BATCH: u64 = 256;
/// Ids the racing writer cycles through, above [`INDEXED`].
///
/// It writes for as long as the vacuum runs, so a fresh id per write would
/// grow the index for as long as the test needs it to keep writing. An
/// upsert lands in the same copy: the swap carries an id whose slot changed
/// exactly like one it never saw.
const RACING_IDS: u64 = 4_096;
const _: () = assert!(
    RACING_BATCH >= 100 && RACING_IDS.is_multiple_of(RACING_BATCH),
    "a racing batch must reach the graph's parallel batch path, and tile the racing ids"
);
/// Writes that must race one vacuum before the test counts it: two whole
/// batches, so at least one of them was issued from start to finish while
/// that vacuum ran. A vacuum no batch raced proves nothing about the copy the
/// swap makes of one.
const RACING_WRITES: u64 = 2 * RACING_BATCH;
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

    let next_write = Arc::new(AtomicU64::new(0));
    let mut raced = Vec::new();
    while raced.len() < ATTEMPTS && raced.last().is_none_or(|&made| made < RACING_WRITES) {
        let made = race_one_vacuum(&index, &next_write, &raced);
        raced.push(made);
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
    // The writing threads are joined, so every id handed out was written, and
    // the cursor names them in order from `INDEXED` until it wraps.
    let written = next_write.load(Ordering::Acquire).min(RACING_IDS);
    let lost = unscanned(&index, INDEXED..INDEXED + written);
    assert!(
        lost.is_empty(),
        "{} of the {written} ids written during the vacuums missing from an \
         exhaustive scan (first {:?})",
        lost.len(),
        lost.first()
    );
}

/// Writes the [`RACING_BATCH`] racing ids that `first` opens, as one batch,
/// and returns how many it wrote.
///
/// The ids wrap around [`RACING_IDS`], which [`RACING_BATCH`] tiles: no batch
/// holds one id twice, and every pass writes each racing id once.
fn write_racing_batch(index: &HnswIndex, first: u64) -> u64 {
    let ids: Vec<u64> = (first..first + RACING_BATCH)
        .map(|n| INDEXED + n % RACING_IDS)
        .collect();
    let vectors: Vec<Vec<f32>> = ids.iter().map(|&id| vector(id)).collect();
    let written =
        index.insert_batch_parallel(ids.iter().copied().zip(vectors.iter().map(Vec::as_slice)));
    assert_eq!(
        u64::try_from(written),
        Ok(RACING_BATCH),
        "test: a racing batch placed short"
    );
    RACING_BATCH
}

/// Runs one vacuum of `index` while a thread writes batches from `next_write`
/// until it ends, and returns how many ids it wrote. `raced` holds what the
/// vacuums before this one returned, for the hang report.
///
/// The writing thread never pauses, so the write guard the vacuum asks for is
/// nearly always held by a batch in flight, which maps its ids before the
/// guard is granted: the swap's own copy covers that batch (see
/// [`RACING_BATCH`]).
///
/// A vacuum that has not ended within [`HANG_BOUND`] exits the process.
fn race_one_vacuum(index: &Arc<HnswIndex>, next_write: &Arc<AtomicU64>, raced: &[u64]) -> u64 {
    let vacuuming = Arc::new(AtomicBool::new(true));
    let (report, reported) = mpsc::channel();
    let vacuum = {
        let (index, vacuuming) = (Arc::clone(index), Arc::clone(&vacuuming));
        thread::spawn(move || {
            let result = index.vacuum();
            vacuuming.store(false, Ordering::Release);
            let _ = report.send(result);
        })
    };
    let writes = {
        let (index, next_write) = (Arc::clone(index), Arc::clone(next_write));
        thread::spawn(move || {
            let mut made = 0_u64;
            while vacuuming.load(Ordering::Acquire) {
                let first = next_write.fetch_add(RACING_BATCH, Ordering::AcqRel);
                made += write_racing_batch(&index, first);
            }
            made
        })
    };

    let result = match reported.recv_timeout(HANG_BOUND) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!(
            "the vacuuming thread ended without reporting: {:?}",
            vacuum.join()
        ),
        Err(mpsc::RecvTimeoutError::Timeout) => report_hang(raced),
    };
    vacuum.join().expect("test: the vacuuming thread panicked");
    let made = writes.join().expect("test: the writing thread panicked");
    assert!(result.is_ok(), "vacuum {}: {result:?}", raced.len());
    made
}

/// Reports a vacuum that did not end within [`HANG_BOUND`], after the
/// vacuums whose racing writes `raced` holds, and exits the process.
fn report_hang(raced: &[u64]) -> ! {
    // Straight to the process's stderr: libtest captures `eprintln!` on the
    // test thread and the threads it spawns, and `exit` drops what it holds.
    let _ = writeln!(
        std::io::stderr(),
        "HANG: vacuum {} did not finish within {HANG_BOUND:?} beside batch searches \
         on a {}-thread rayon pool; the vacuums before it finished with these writes \
         racing each: {raced:?}",
        raced.len(),
        rayon::current_num_threads(),
    );
    std::process::exit(1);
}

/// The ids of `ids` an exhaustive scan of `index` does not return.
fn unscanned(index: &HnswIndex, ids: std::ops::Range<u64>) -> Vec<u64> {
    let scanned: HashSet<u64> = index
        .search_brute_force(&vector(0), index.len())
        .expect("test: exhaustive scan")
        .iter()
        .map(|hit| hit.id)
        .collect();
    ids.filter(|id| !scanned.contains(id)).collect()
}
