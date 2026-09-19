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
//! Each vacuum runs on its own thread and reports through a channel. A vacuum
//! the test calls parked can never be joined, so the test writes what it saw
//! to the process's stderr and exits with a failure code.
//!
//! What it calls parked is **the absence of progress, not slowness**. The
//! deadlock this test exists for stops the writing thread too: the batch that
//! holds the index read guard is the one waiting on a rayon worker that will
//! never come. So the guard watches the writer's own counter, and fires only
//! when neither the vacuum nor the writes have advanced for
//! [`STALL_WINDOWS`] windows of [`stall_window`] each. A slow vacuum keeps
//! its writer advancing and is never reported; a parked one advances nothing,
//! on any machine and in any profile.
//!
//! A time budget was tried first and is kept only as a backstop
//! ([`CEILING_FACTOR`]), because it is a hidden claim about the machine. The
//! first version allowed forty times one unraced vacuum on the strength of a
//! measured raced/unraced ratio "under three". Re-measured, four runs on an
//! idle machine with nothing else running, that claim does not hold: the
//! ratio is 20.6 to 26.7, and the closest run finished 152.9 s into a 229.2 s
//! bound, 1.50x from firing. A budget that nearly fires on a correct vacuum
//! is a red build waiting for a slower runner, which is how a guard gets
//! deleted. The ratio is what it is because a raced vacuum's cost is
//! dominated by an unbounded catch-up (#2335), not because racing adds a
//! constant to a rebuild.

use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

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
/// Consecutive windows without a single write completed before the test calls
/// the vacuum parked.
///
/// One window is not enough: a vacuum's swap takes the write guard, and every
/// writer waits on it for as long as that copy runs. Three windows of a whole
/// unraced vacuum each is far longer than any guard this index holds, and
/// still finite while a parked one is not.
const STALL_WINDOWS: u32 = 3;
/// The floor under one stall window, for a baseline so short that three of
/// them would fire on scheduling noise alone.
const STALL_FLOOR: Duration = Duration::from_secs(2);
/// Times the cost of one unraced vacuum a raced one may take before the test
/// gives up on it, whatever the writer is doing.
///
/// The backstop, not the detector: [`STALL_WINDOWS`] is what catches the
/// deadlock this test exists for, which stops the writer as well. This only
/// catches a park that somehow leaves the writes advancing, and it is set far
/// above anything measured so a correct vacuum never reaches it. Measured on
/// this index, in debug, on the two-thread pool this test builds — unraced
/// baseline against the raced vacuum of the same run, four runs on an idle
/// machine:
///
/// | baseline | raced | ratio |
/// | --- | --- | --- |
/// | 5.73 s | 152.95 s | 26.7 |
/// | 5.18 s | 107.01 s | 20.6 |
/// | 5.25 s | 125.25 s | 23.9 |
/// | 6.02 s | 128.58 s | 21.4 |
///
/// The forty this started at bounded the first of those at 229.2 s, which it
/// reached within 1.50x. A fifth run, on the same machine with the rest of
/// the suite building beside it, measured 18.05 s against 108.56 s: a ratio
/// of 6.0, because load slows the baseline more than it slows the catch-up.
/// That is the whole argument against a fixed factor -- the ratio is not a
/// property of the code, it moves between 6 and 27 with what else is
/// running. Four hundred is fifteen times the worst ratio seen: a vacuum
/// that ends cannot reach it, and a parked one is caught by the stall count
/// long before.
const CEILING_FACTOR: u32 = 400;

/// How long the guard waits for a write to land before counting a window as
/// stalled: one unraced vacuum, or [`STALL_FLOOR`], whichever is longer.
fn stall_window(baseline: Duration) -> Duration {
    baseline.max(STALL_FLOOR)
}

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

    // The bound the raced vacuums are held to, from one vacuum of this index
    // with nothing racing it: same rebuild, same machine, same profile.
    let started = Instant::now();
    index.vacuum().expect("test: the unraced baseline vacuum");
    let baseline = started.elapsed();
    let window = stall_window(baseline);
    let ceiling = CEILING_FACTOR * baseline;
    println!(
        "baseline vacuum {baseline:?}, stall window {window:?} x{STALL_WINDOWS}, \
         ceiling {ceiling:?} ({CEILING_FACTOR}x)"
    );

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
        let made = race_one_vacuum(&index, &next_write, &raced, window, ceiling);
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
/// A vacuum that neither ends nor lets a write land for [`STALL_WINDOWS`]
/// windows of `window`, or that has not ended within `ceiling` whatever the
/// writer is doing, exits the process.
fn race_one_vacuum(
    index: &Arc<HnswIndex>,
    next_write: &Arc<AtomicU64>,
    raced: &[u64],
    window: Duration,
    ceiling: Duration,
) -> u64 {
    let started = Instant::now();
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
    // Bumped by the writing thread after every batch: the guard below reads
    // it, not the clock, to tell a slow vacuum from a parked one.
    let progress = Arc::new(AtomicU64::new(0));
    let writes = {
        let (index, next_write, progress) = (
            Arc::clone(index),
            Arc::clone(next_write),
            Arc::clone(&progress),
        );
        thread::spawn(move || {
            let mut made = 0_u64;
            while vacuuming.load(Ordering::Acquire) {
                let first = next_write.fetch_add(RACING_BATCH, Ordering::AcqRel);
                made += write_racing_batch(&index, first);
                progress.store(made, Ordering::Release);
            }
            made
        })
    };

    let waited = wait_for_the_vacuum(&reported, &progress, window, ceiling);
    if let Waited::GaveUp(made, windows, elapsed) = waited {
        report_hang(raced, made, windows, window, elapsed);
    }
    let joined = vacuum.join();
    let Waited::Reported(result) = waited else {
        panic!("the vacuuming thread ended without reporting: {joined:?}")
    };
    joined.expect("test: the vacuuming thread panicked");
    let made = writes.join().expect("test: the writing thread panicked");
    assert!(result.is_ok(), "vacuum {}: {result:?}", raced.len());
    println!(
        "vacuum {} took {:?} of at most {ceiling:?}, {made} writes racing it",
        raced.len(),
        started.elapsed()
    );
    made
}

/// What waiting for a vacuum ended in.
#[derive(Debug)]
enum Waited<T> {
    /// The vacuum reported.
    Reported(T),
    /// The channel closed without a report.
    Closed,
    /// The guard gave up: writes completed, windows without one, time waited.
    GaveUp(u64, u32, Duration),
}

/// Waits for the vacuum to report, watching `progress` rather than the clock.
///
/// Decides; it does not act on the decision. `report_hang` exits the process,
/// which no test could observe, so the giving-up path is returned instead and
/// [`the_guard_gives_up_when_nothing_advances`] drives it.
fn wait_for_the_vacuum<T>(
    reported: &mpsc::Receiver<T>,
    progress: &AtomicU64,
    window: Duration,
    ceiling: Duration,
) -> Waited<T> {
    let started = Instant::now();
    let mut stalled_windows = 0;
    let mut last_progress = progress.load(Ordering::Acquire);
    loop {
        match reported.recv_timeout(window) {
            Ok(result) => return Waited::Reported(result),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Waited::Closed,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let made = progress.load(Ordering::Acquire);
        if made == last_progress {
            stalled_windows += 1;
        } else {
            // One window of progress clears the count: the guard reports a
            // stall that is current, never a sum of unrelated pauses.
            stalled_windows = 0;
            last_progress = made;
        }
        if stalled_windows >= STALL_WINDOWS || started.elapsed() >= ceiling {
            return Waited::GaveUp(made, stalled_windows, started.elapsed());
        }
    }
}

/// The positive control for the anti-hang guard.
///
/// The guard the test above relies on exits the process, so nothing it does
/// in a passing run says it can fire at all. Here the channel never reports
/// and the counter never moves, which is what a parked vacuum looks like:
/// [`STALL_WINDOWS`] windows later the guard must give up. Its companion --
/// a counter that advances every window -- must not, and that is what tells
/// "it can say stop" apart from "it only says stop".
#[test]
fn the_guard_gives_up_when_nothing_advances() {
    const WINDOW: Duration = Duration::from_millis(20);
    // Far beyond what either case needs: this control is about the stall
    // count, not about the backstop.
    const CEILING: Duration = Duration::from_secs(30);

    let (_keep_open, never_reports) = mpsc::channel::<()>();
    let frozen = AtomicU64::new(0);
    let started = Instant::now();
    match wait_for_the_vacuum(&never_reports, &frozen, WINDOW, CEILING) {
        Waited::GaveUp(made, windows, _) => {
            assert_eq!(
                (made, windows),
                (0, STALL_WINDOWS),
                "(writes made, windows)"
            );
        }
        other => panic!("a frozen counter must make the guard give up, got {other:?}"),
    }
    assert!(
        started.elapsed() >= STALL_WINDOWS * WINDOW,
        "the guard gave up before waiting {STALL_WINDOWS} windows"
    );

    // The control: a writer that keeps advancing is never called parked, so
    // the guard reaches its backstop instead of its stall count.
    let (_keep_open, never_reports) = mpsc::channel::<()>();
    let advancing = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (advancing, stop) = (Arc::clone(&advancing), Arc::clone(&stop));
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                advancing.fetch_add(1, Ordering::AcqRel);
                thread::sleep(WINDOW / 4);
            }
        })
    };
    let waited = wait_for_the_vacuum(&never_reports, &advancing, WINDOW, 10 * WINDOW);
    stop.store(true, Ordering::Release);
    writer.join().expect("test: the advancing writer panicked");
    match waited {
        Waited::GaveUp(made, windows, _) => {
            assert!(made > 0, "the advancing counter never moved");
            assert!(
                windows < STALL_WINDOWS,
                "a counter advancing every window reached {windows} stalled windows"
            );
        }
        other => panic!("the backstop must still end the wait, got {other:?}"),
    }
}

/// Reports a vacuum the guard gave up on, after the vacuums whose racing
/// writes `raced` holds, and exits the process.
fn report_hang(
    raced: &[u64],
    made: u64,
    stalled_windows: u32,
    window: Duration,
    elapsed: Duration,
) -> ! {
    // Straight to the process's stderr: libtest captures `eprintln!` on the
    // test thread and the threads it spawns, and `exit` drops what it holds.
    let _ = writeln!(
        std::io::stderr(),
        "HANG: vacuum {} has run {elapsed:?} beside batch searches on a {}-thread \
         rayon pool and has not ended; the writer completed {made} writes and has \
         completed none for {stalled_windows} window(s) of {window:?}; the vacuums \
         before it finished with these writes racing each: {raced:?}",
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
