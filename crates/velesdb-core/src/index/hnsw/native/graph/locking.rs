//! Lock-rank enforcement for HNSW graph operations.
//!
//! Defines the global lock ordering invariant and provides runtime
//! checking to prevent deadlocks. The rank system encodes the rule:
//!
//! ```text
//! vectors (rank 10) → columnar (rank 15) → layers (rank 20) → neighbors (rank 30)
//! ```
//!
//! Acquiring a lock with lower-or-equal rank than the highest currently
//! held rank is a violation that gets recorded in safety counters.
//!
//! # Release Build Behavior (F-25)
//!
//! In release builds, lock-rank tracking is a no-op for maximum
//! search throughput. Only the atomic violation counter is incremented
//! (no thread-local stack overhead). In debug builds, full stack-based
//! tracking with tracing warnings is enabled.

use super::safety_counters::HNSW_COUNTERS;

/// Lock rank values — monotonically increasing acquisition order.
///
/// The global lock order is: vectors → columnar → layers → neighbors.
/// Any code path that acquires multiple locks must acquire them
/// in strictly increasing rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum LockRank {
    /// `vectors` RwLock — rank 10 (acquired first)
    Vectors = 10,
    /// `columnar` RwLock — rank 15 (PDX block-columnar layout)
    #[allow(dead_code)] // Reason: PDX columnar lock rank — used when PDX search is wired
    Columnar = 15,
    /// `layers` RwLock — rank 20 (acquired after vectors/columnar)
    Layers = 20,
    /// Per-node neighbor lists — rank 30 (acquired last)
    #[allow(dead_code)] // Reason: Neighbor-level lock rank — reserved for fine-grained locking
    Neighbors = 30,
}

// F-25: Thread-local stack only in debug builds to avoid ~10-20ns overhead
// per lock acquire/release in hot search loops.
#[cfg(debug_assertions)]
use std::cell::RefCell;

#[cfg(debug_assertions)]
thread_local! {
    /// Stack of lock ranks currently held by this thread.
    /// Used for runtime verification of monotonic acquisition order.
    static LOCK_RANK_STACK: RefCell<Vec<LockRank>> = const { RefCell::new(Vec::new()) };
}

/// Records acquisition of a lock at the given rank.
///
/// In debug builds: full thread-local stack tracking with violation detection.
/// In release builds: no-op (zero overhead on hot search paths).
#[inline]
pub(crate) fn record_lock_acquire(rank: LockRank) {
    #[cfg(debug_assertions)]
    {
        LOCK_RANK_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(&highest) = stack.last() {
                if rank <= highest {
                    HNSW_COUNTERS.record_invariant_violation();

                    tracing::warn!(
                        acquired = ?rank,
                        highest_held = ?highest,
                        "HNSW lock-order violation: acquiring {:?} while holding {:?}",
                        rank,
                        highest,
                    );
                }
            }
            stack.push(rank);
        });
    }

    // Release builds: suppress unused variable warning
    #[cfg(not(debug_assertions))]
    let _ = rank;
}

/// Records release of the most recent lock at the given rank.
///
/// In debug builds: pops rank from thread-local stack, detects corruption.
/// In release builds: no-op (zero overhead).
#[inline]
pub(crate) fn record_lock_release(rank: LockRank) {
    #[cfg(debug_assertions)]
    {
        LOCK_RANK_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(top) = stack.pop() {
                if top != rank {
                    HNSW_COUNTERS.record_corruption();
                }
            }
        });
    }

    #[cfg(not(debug_assertions))]
    let _ = rank;
}

/// Returns the current depth of the lock rank stack for this thread.
///
/// Useful for assertions in tests. Requires debug_assertions (always true in test builds).
#[allow(dead_code)] // Reason: Debug introspection — available for lock-ordering tests
#[cfg(all(test, debug_assertions))]
pub(crate) fn lock_depth() -> usize {
    LOCK_RANK_STACK.with(|stack| stack.borrow().len())
}
