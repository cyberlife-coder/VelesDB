//! Lock-rank enforcement for HNSW graph operations.
//!
//! Defines the global lock ordering invariant and provides runtime
//! checking to prevent deadlocks. The rank system encodes the rule:
//!
//! ```text
//! vectors (rank 10) → layers (rank 20) → neighbors (rank 30)
//! ```
//!
//! Acquiring a lock with lower-or-equal rank than the highest currently
//! held rank is a violation that gets recorded in safety counters.
//!
//! # Release Parity
//!
//! The rank checker uses thread-local storage and atomic counters,
//! both of which are cheap enough for release builds. Only expensive
//! deep adjacency scans should be gated behind `#[cfg(debug_assertions)]`.

use super::safety_counters::HNSW_COUNTERS;
use std::cell::RefCell;

/// Lock rank values — monotonically increasing acquisition order.
///
/// The global lock order is: vectors → layers → neighbors.
/// Any code path that acquires multiple locks must acquire them
/// in strictly increasing rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum LockRank {
    /// `vectors` RwLock — rank 10 (acquired first)
    Vectors = 10,
    /// `layers` RwLock — rank 20 (acquired second)
    Layers = 20,
    /// Per-node neighbor lists — rank 30 (acquired last)
    Neighbors = 30,
}

thread_local! {
    /// Stack of lock ranks currently held by this thread.
    /// Used for runtime verification of monotonic acquisition order.
    static LOCK_RANK_STACK: RefCell<Vec<LockRank>> = const { RefCell::new(Vec::new()) };
}

/// Records acquisition of a lock at the given rank.
///
/// If the rank is not strictly higher than the current highest held rank,
/// this is a lock-order violation. The violation is:
/// - Recorded in the global safety counters (always, in all builds)
/// - Logged as a warning via tracing (debug builds only, to avoid overhead)
///
/// # Cost
///
/// Thread-local stack push + comparison: ~10-20ns per call.
/// Acceptable for release builds on lock-acquisition paths.
#[inline]
pub(crate) fn record_lock_acquire(rank: LockRank) {
    LOCK_RANK_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        if let Some(&highest) = stack.last() {
            if rank <= highest {
                // Lock-order violation detected
                HNSW_COUNTERS.record_invariant_violation();

                #[cfg(debug_assertions)]
                {
                    tracing::warn!(
                        acquired = ?rank,
                        highest_held = ?highest,
                        "HNSW lock-order violation: acquiring {:?} while holding {:?}",
                        rank,
                        highest,
                    );
                }
            }
        }
        stack.push(rank);
    });
}

/// Records release of the most recent lock at the given rank.
///
/// Pops the rank from the thread-local stack. If the top of stack
/// doesn't match the expected rank, records a corruption signal.
#[inline]
pub(crate) fn record_lock_release(rank: LockRank) {
    LOCK_RANK_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        if let Some(top) = stack.pop() {
            if top != rank {
                // Release order doesn't match acquisition — corruption signal
                HNSW_COUNTERS.record_corruption();
            }
        }
    });
}

/// Returns the current depth of the lock rank stack for this thread.
///
/// Useful for assertions in tests.
#[cfg(test)]
pub(crate) fn lock_depth() -> usize {
    LOCK_RANK_STACK.with(|stack| stack.borrow().len())
}
