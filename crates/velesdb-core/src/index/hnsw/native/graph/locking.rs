//! Lock-rank enforcement for HNSW graph operations.
//!
//! Defines the global lock ordering invariant and provides runtime
//! checking to prevent deadlocks. The rank system encodes the rule:
//!
//! ```text
//! gpu_vectors_snapshot (rank 5) → vectors (rank 10) → columnar (rank 15)
//!     → layers (rank 20) → neighbors (rank 30)
//! ```
//!
//! The `gpu_vectors_snapshot` mutex is acquired before `vectors` in the
//! GPU path (`get_or_refresh_vector_snapshot` takes the mutex first, then
//! calls `with_vectors_read` which takes `vectors`). Writers release
//! `vectors` before reacquiring `gpu_vectors_snapshot` to invalidate, so
//! both call sites observe the same order.
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

#[cfg(debug_assertions)]
use super::safety_counters::HNSW_COUNTERS;

/// Lock rank values — monotonically increasing acquisition order.
///
/// The global lock order is:
/// `gpu_vectors_snapshot → vectors → columnar → layers → neighbors`.
/// Any code path that acquires multiple locks must acquire them
/// in strictly increasing rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum LockRank {
    /// `gpu_vectors_snapshot` Mutex — rank 5 (acquired before `Vectors`).
    ///
    /// Caches the flat vector buffer for GPU upload. The snapshot refresh
    /// path acquires this mutex, then calls `with_vectors_read` which takes
    /// `Vectors`, so the rank must be strictly lower than `Vectors`.
    // Only exercised when the `gpu` feature is active; stays in the enum so
    // lock-ordering logic is the same across feature configurations.
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    GpuVectorsSnapshot = 5,
    /// `vectors` RwLock — rank 10 (acquired first among the core HNSW locks)
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

/// Returns `true` if the current thread is currently holding a lock at `rank`.
///
/// Debug-builds only: callers guarded by `debug_assert!` will compile to
/// nothing in release builds. Use this to encode caller contracts for
/// cache rebuild helpers that must run under a parent lock — e.g. a
/// GPU CSR rebuild is only race-free while the layers read lock is held.
///
/// Returns `true` in release builds (the thread-local stack is not
/// maintained there, so any runtime assertion is a no-op). Callers
/// should always wrap the invocation in `debug_assert!` so the entire
/// check compiles out of release binaries.
// Only reachable via `debug_assert!` in `gpu_csr` (feature-gated) and
// via the locking test module; outside those contexts rustc treats the
// function as dead code. Keep it visible at the crate level so future
// callers can reuse it.
#[cfg_attr(not(any(feature = "gpu", test)), allow(dead_code))]
#[cfg(debug_assertions)]
pub(crate) fn holds_lock(rank: LockRank) -> bool {
    LOCK_RANK_STACK.with(|stack| stack.borrow().contains(&rank))
}

/// Release build stub — never panics, but callers should only invoke this
/// behind `debug_assert!` so the call is compiled out entirely.
#[cfg_attr(not(any(feature = "gpu", test)), allow(dead_code))]
#[cfg(not(debug_assertions))]
#[inline]
pub(crate) fn holds_lock(_rank: LockRank) -> bool {
    true
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn holds_lock_reports_currently_held_rank() {
        // Empty stack — nothing held.
        assert!(!holds_lock(LockRank::Layers));
        assert!(!holds_lock(LockRank::Vectors));

        record_lock_acquire(LockRank::Layers);
        assert!(holds_lock(LockRank::Layers));
        assert!(!holds_lock(LockRank::Vectors));

        record_lock_release(LockRank::Layers);
        assert!(!holds_lock(LockRank::Layers));
    }

    #[test]
    fn gpu_vectors_snapshot_rank_sorts_before_vectors() {
        // Monotone rank check — the core invariant of the enum.
        assert!(LockRank::GpuVectorsSnapshot < LockRank::Vectors);
        assert!(LockRank::Vectors < LockRank::Columnar);
        assert!(LockRank::Columnar < LockRank::Layers);
        assert!(LockRank::Layers < LockRank::Neighbors);
    }

    #[test]
    fn nested_acquire_in_declared_order_reports_both_held() {
        // Simulate `get_or_refresh_vector_snapshot`: snapshot then vectors.
        record_lock_acquire(LockRank::GpuVectorsSnapshot);
        record_lock_acquire(LockRank::Vectors);

        assert!(holds_lock(LockRank::GpuVectorsSnapshot));
        assert!(holds_lock(LockRank::Vectors));

        record_lock_release(LockRank::Vectors);
        record_lock_release(LockRank::GpuVectorsSnapshot);

        // Stack back to empty.
        assert!(!holds_lock(LockRank::GpuVectorsSnapshot));
        assert!(!holds_lock(LockRank::Vectors));
    }
}
