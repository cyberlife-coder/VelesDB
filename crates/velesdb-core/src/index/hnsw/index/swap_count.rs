//! Bench-only counter of the work a vacuum's swap does under the write guard.
//!
//! Compiled only with the `internal-bench` feature: release and default test
//! builds carry zero instrumentation.
//!
//! # The unit
//!
//! One *reconciled id* is one id whose vector `reconcile` copies into the
//! replacement graph **while the write guard is held** — an id written
//! between the last catch-up round and the moment the guard was granted. Ids
//! the catch-up already carried cost a map lookup and are not counted: they
//! are the bounded part.
//!
//! It exists because the number has no upper bound today (#2335). A batch
//! insert in flight maps its ids after the last `written_since` check and
//! before the guard is granted, so a catch-up that stopped with nothing left
//! can still hand `reconcile` a whole batch to copy — and searches and writes
//! wait on that guard while it runs. A counter is the only way to assert a
//! bound: wall-clock on a shared runner is noise, and the copy is invisible
//! from outside.
//!
//! # Corollary
//!
//! With `internal-bench` enabled, a relaxed atomic increment sits in the swap
//! path — wall-clock measured under this feature is not a benchmark of the
//! real engine, exactly as `index::hnsw::eval_count` says for the search side.

use std::sync::atomic::{AtomicU64, Ordering};

/// Ids reconciled under the write guard since the last reset.
static RECONCILED_UNDER_GUARD: AtomicU64 = AtomicU64::new(0);

/// Ids settled under the seal, outside the graph guard, since the last reset.
///
/// This is the control the bound needs. Once the seal works, the guarded
/// count is zero on a correct run *and* on a run no write ever raced — the
/// two are indistinguishable from it alone. This one separates them: it is
/// the work the racing writes actually caused, moved off the guard rather
/// than removed.
static SETTLED_UNDER_SEAL: AtomicU64 = AtomicU64::new(0);

/// The largest single swap's count since the last reset.
///
/// The total answers "how much work in all", the peak answers the question
/// #2335 actually asks: how long can *one* swap hold the guard.
static PEAK_UNDER_GUARD: AtomicU64 = AtomicU64::new(0);

/// Records that one swap reconciled `n` ids under the write guard.
#[inline]
pub(crate) fn record_reconciled(n: usize) {
    let n = n as u64;
    RECONCILED_UNDER_GUARD.fetch_add(n, Ordering::Relaxed);
    PEAK_UNDER_GUARD.fetch_max(n, Ordering::Relaxed);
}

/// Records that a sealed settle carried `n` ids outside the graph guard.
#[inline]
pub(crate) fn record_settled(n: usize) {
    SETTLED_UNDER_SEAL.fetch_add(n as u64, Ordering::Relaxed);
}

/// Ids settled under the seal since the last reset.
#[must_use]
pub(crate) fn settled_under_seal() -> u64 {
    SETTLED_UNDER_SEAL.load(Ordering::Relaxed)
}

/// Ids reconciled under the write guard since the last reset.
#[must_use]
pub(crate) fn reconciled_under_guard() -> u64 {
    RECONCILED_UNDER_GUARD.load(Ordering::Relaxed)
}

/// The largest single swap's count since the last reset.
#[must_use]
pub(crate) fn peak_reconciled_under_guard() -> u64 {
    PEAK_UNDER_GUARD.load(Ordering::Relaxed)
}

/// Resets every counter to zero.
pub(crate) fn reset_reconciled_under_guard() {
    RECONCILED_UNDER_GUARD.store(0, Ordering::Relaxed);
    PEAK_UNDER_GUARD.store(0, Ordering::Relaxed);
    SETTLED_UNDER_SEAL.store(0, Ordering::Relaxed);
}
