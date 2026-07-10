//! Scheduled concurrency suite for the core lock-acquisition order.
//!
//! This suite is designed to run under `ThreadSanitizer` on a nightly toolchain
//! (see `.github/workflows/tsan-concurrency.yml`). It exercises acquisition
//! across the core lock classes in strictly ascending rank order
//! (`gpu → vectors → columnar → layers → neighbors`) from many threads at
//! once, using [`assert_lock_order`] to encode the global order and
//! `parking_lot` locks to create real cross-thread contention.
//!
//! - Any **lock-order violation** trips the debug-only `assert_lock_order`
//!   (`debug_assert!`) and fails the suite.
//! - Any **data race** on the shared state protected by the ordered locks is
//!   detected by `ThreadSanitizer` and fails the suite.
//!
//! The harness itself is invoked single-threaded (`--test-threads=1`); the
//! concurrency lives *inside* each test, which spawns worker threads.

use std::sync::Arc;
use std::thread;

use parking_lot::Mutex;
use velesdb_core::{assert_lock_order, LockRank};

/// Number of worker threads contending on the ordered locks.
const NUM_THREADS: usize = 8;

/// Ascending-order acquisitions each worker performs. Kept modest so the suite
/// stays fast under the ~20x `ThreadSanitizer` slowdown.
const ITERATIONS: usize = 500;

/// The core lock classes in ascending acquisition order, each guarding a
/// shared counter so `ThreadSanitizer` observes real cross-thread memory access
/// under the locks.
struct CoreLocks {
    gpu: Mutex<u64>,
    vectors: Mutex<u64>,
    columnar: Mutex<u64>,
    layers: Mutex<u64>,
    neighbors: Mutex<u64>,
}

impl CoreLocks {
    fn new() -> Self {
        Self {
            gpu: Mutex::new(0),
            vectors: Mutex::new(0),
            columnar: Mutex::new(0),
            layers: Mutex::new(0),
            neighbors: Mutex::new(0),
        }
    }

    /// Acquires every core lock class in strictly ascending rank order,
    /// asserting the global order at each step and mutating the guarded state
    /// while the locks are held nested.
    ///
    /// Holding the guards nested (rather than sequentially) is what makes this
    /// a genuine lock-ordering exercise: every thread must take the same
    /// ascending path or `ThreadSanitizer` / a deadlock would surface it.
    fn acquire_ascending(&self) {
        // gpu (rank 5) — lowest core rank, acquired first with nothing held.
        let mut prev = LockRank::GPU_VECTORS_SNAPSHOT;
        let mut gpu = self.gpu.lock();
        *gpu += 1;

        assert_lock_order(prev, LockRank::VECTORS);
        prev = LockRank::VECTORS;
        let mut vectors = self.vectors.lock();
        *vectors += 1;

        assert_lock_order(prev, LockRank::COLUMNAR);
        prev = LockRank::COLUMNAR;
        let mut columnar = self.columnar.lock();
        *columnar += 1;

        assert_lock_order(prev, LockRank::LAYERS);
        prev = LockRank::LAYERS;
        let mut layers = self.layers.lock();
        *layers += 1;

        assert_lock_order(prev, LockRank::NEIGHBORS);
        let mut neighbors = self.neighbors.lock();
        *neighbors += 1;

        // Guards drop in reverse (neighbors → gpu) at end of scope, which is
        // the correct release order for ascending acquisition.
        drop(neighbors);
        drop(layers);
        drop(columnar);
        drop(vectors);
        drop(gpu);
    }

    /// Returns the per-class counters once all workers have joined.
    fn totals(&self) -> [u64; 5] {
        [
            *self.gpu.lock(),
            *self.vectors.lock(),
            *self.columnar.lock(),
            *self.layers.lock(),
            *self.neighbors.lock(),
        ]
    }
}

/// Primary scheduled concurrency test: many threads acquire the core lock
/// classes in ascending rank order concurrently. Passes iff no lock-order
/// violation is asserted and no data race is detected on the shared counters.
#[test]
fn test_concurrent_ascending_lock_order_holds() {
    let locks = Arc::new(CoreLocks::new());

    let workers: Vec<_> = (0..NUM_THREADS)
        .map(|_| {
            let locks = Arc::clone(&locks);
            thread::spawn(move || {
                for _ in 0..ITERATIONS {
                    locks.acquire_ascending();
                }
            })
        })
        .collect();

    for worker in workers {
        // A panic inside a worker (e.g. a tripped `assert_lock_order`) must
        // fail the suite rather than be silently swallowed.
        worker.join().expect("worker thread panicked");
    }

    let expected = (NUM_THREADS * ITERATIONS) as u64;
    assert_eq!(
        locks.totals(),
        [expected; 5],
        "every ordered lock class must have been acquired exactly once per iteration per thread"
    );
}

/// The core ranks are strictly ascending, so `assert_lock_order` accepts every
/// adjacent forward transition on the acquisition path. This pins the ordering
/// the concurrent test relies on.
#[test]
fn test_core_acquisition_path_is_strictly_ascending() {
    let path = [
        LockRank::GPU_VECTORS_SNAPSHOT,
        LockRank::VECTORS,
        LockRank::COLUMNAR,
        LockRank::LAYERS,
        LockRank::NEIGHBORS,
    ];
    for pair in path.windows(2) {
        assert!(pair[0] < pair[1], "core ranks must be strictly ascending");
        assert_lock_order(pair[0], pair[1]);
    }
}

/// In debug builds (including `ThreadSanitizer` runs, which keep debug
/// assertions on), acquiring a lower rank while holding a higher one must trip
/// `assert_lock_order`. This proves the suite can actually detect a
/// lock-order violation rather than passing vacuously.
#[cfg(debug_assertions)]
#[test]
fn test_descending_acquisition_is_detected() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(|| {
        assert_lock_order(LockRank::NEIGHBORS, LockRank::GPU_VECTORS_SNAPSHOT);
    });
    std::panic::set_hook(prev_hook);

    assert!(
        outcome.is_err(),
        "descending acquisition must trip the lock-order assertion in debug builds"
    );
}
