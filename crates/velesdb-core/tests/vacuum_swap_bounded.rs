#![cfg(all(feature = "persistence", feature = "internal-bench"))]
//! A vacuum's swap holds the write guard for a bounded amount of work (#2335).
//!
//! `catch_up` copies the writes made during the rebuild without the write
//! guard, then `reconcile` copies whatever is left **under** it. Nothing
//! bounds that remainder: a batch insert in flight maps its ids after the
//! last `written_since` round looked and before the guard is granted, so a
//! catch-up that stopped with nothing left can still hand the swap a whole
//! batch to copy. Searches and writes wait on that guard while it runs.
//!
//! # What is asserted, and why a counter
//!
//! `internal_bench::vacuum_peak_reconciled_under_guard` counts the ids one
//! swap copies under the guard. Wall-clock would be noise on a shared runner
//! and the copy is invisible from outside, so the bound is asserted on the
//! count, as `cost_crossover` does for the sparse dispatch.
//!
//! The bound is stated in terms of the batch this test races, not as a
//! constant: a remainder of at most [`BOUND`] means the swap no longer takes
//! an unbounded batch under the guard. A run whose swaps see no racing write
//! at all proves nothing, so the harness asserts first that writes did race.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use velesdb_core::distance::DistanceMetric;
use velesdb_core::index::HnswIndex;
use velesdb_core::internal_bench::{
    reset_vacuum_reconciled_under_guard, vacuum_peak_reconciled_under_guard,
    vacuum_reconciled_under_guard, vacuum_settled_under_seal,
};
use velesdb_core::VectorIndex;

const DIM: usize = 8;
const SEEDED: u64 = 3_000;
const _: () = assert!(
    RACING_IDS.is_multiple_of(BATCH),
    "a batch must tile the racing window"
);
/// Ids one racing batch writes. Far above `CATCH_UP_REMAINDER` (64), so a
/// batch landing in the window is unmistakable in the count.
const BATCH: u64 = 2_000;
/// Ids the writer cycles through, above [`SEEDED`].
///
/// A window, not fresh ids for ever: writing only fresh ones grows the index
/// without bound, and each vacuum then rebuilds a larger graph than the last
/// -- measured, twelve of them did not finish in twenty minutes. Cycling
/// upserts the same ids onto new slots, which is exactly the "written since"
/// the catch-up and the swap have to settle.
const RACING_IDS: u64 = 8_000;
/// Vacuums run while the writer loops.
const VACUUMS: usize = 8;
/// The remainder one swap may copy under the write guard.
///
/// `CATCH_UP_REMAINDER` is 64: a catch-up stops when a round sees that many
/// or fewer, so that many may legitimately be left for the swap. The bound
/// allows one such round's worth, and nothing like a whole `BATCH`.
const BOUND: u64 = 64;

fn vector(id: u64) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..DIM)
        .map(|i| ((id as f32) * 0.31 + (i as f32) * 0.17).sin())
        .collect()
}

#[test]
fn a_swap_copies_a_bounded_remainder_under_the_write_guard() {
    let index = Arc::new(HnswIndex::new(DIM, DistanceMetric::Euclidean).unwrap());
    for id in 0..SEEDED {
        index.insert(id, &vector(id));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let next = Arc::new(AtomicU64::new(SEEDED));
    let writer = {
        let (index, stop, next) = (Arc::clone(&index), Arc::clone(&stop), Arc::clone(&next));
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let first = next.fetch_add(BATCH, Ordering::AcqRel);
                let items: Vec<(u64, Vec<f32>)> = (first..first + BATCH)
                    .map(|n| SEEDED + n % RACING_IDS)
                    .map(|id| (id, vector(id)))
                    .collect();
                index.insert_batch_parallel(items.iter().map(|(id, v)| (*id, v.as_slice())));
                thread::sleep(Duration::from_millis(40));
            }
        })
    };

    reset_vacuum_reconciled_under_guard();
    for _ in 0..VACUUMS {
        index.vacuum().expect("test: vacuum");
    }
    let total = vacuum_reconciled_under_guard();
    let peak = vacuum_peak_reconciled_under_guard();
    let settled = vacuum_settled_under_seal();
    stop.store(true, Ordering::Release);
    writer.join().expect("test: the writing thread panicked");

    println!(
        "{VACUUMS} vacuums: {settled} ids settled under the seal, \
         {total} reconciled under the guard, peak {peak} in one swap"
    );

    // The positive control, and it cannot be the guarded count: once the seal
    // works that count is zero on a correct run *and* on a run no write ever
    // raced. What separates them is the work the seal absorbed.
    assert!(
        settled > 0,
        "no vacuum met a racing write in {VACUUMS} attempts: the harness proves nothing"
    );
    assert!(
        peak <= BOUND,
        "one swap copied {peak} ids under the write guard, over the bound of {BOUND} \
         (a whole racing batch is {BATCH}); searches and writes wait on that guard"
    );
}
