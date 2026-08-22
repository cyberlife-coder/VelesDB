//! The ordinal registry for the global lock-acquisition order — and the
//! declared premium extension point for lock ranks.
//!
//! Two things live here, and naming them precisely is the point (#2013):
//!
//! 1. **The authoritative ordinal table.** Core's enforced ordering
//!    mechanism is the *private* `HnswLockRank` enum in
//!    `index/hnsw/native/graph/locking.rs` (a debug-only, warn-only tracker
//!    on the HNSW hot path; see `CONCURRENCY_MODEL.md` for exactly what it
//!    does and does not check). That enum's discriminants are defined FROM
//!    the constants below, so the two tables cannot diverge without a
//!    compile error — this module owns the numbers, the private enum owns
//!    the mechanism.
//! 2. **The premium ordinal reservation.** The inclusive range `[40, 59]`
//!    and the [`LockRank::premium`] constructor exist for out-of-tree
//!    premium lock classes to order themselves relative to core without
//!    collision. Alongside the observer port (`core/src/observer/`), this
//!    is one of the two declared premium extension points in core — which
//!    is why the type is public despite having no in-tree production
//!    caller: its consumer is `velesdb-private`. Zero in-tree usage is the
//!    expected state of a reservation, not dead code.
//!
//! [`assert_lock_order`] is offered to implementations that adopt this
//! registry (it is what premium builds its checks on); core's own hot path
//! deliberately keeps its private tracker instead of taking a dependency on
//! a public type it would then freeze.
//!
//! Locks MUST be acquired in strictly ascending rank:
//! `gpu < vectors < columnar < layers < neighbors`, then premium `[40, 59]`.

#[cfg(test)]
#[path = "lock_rank_tests.rs"]
mod lock_rank_tests;

/// Ordinal encoding the global lock-acquisition order.
///
/// Locks MUST be acquired in strictly ascending rank; the debug-only
/// [`assert_lock_order`] enforces this in debug builds. The type is a thin
/// newtype over `u8` and derives a total ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockRank(u8);

impl LockRank {
    /// GPU vector snapshot lock — lowest core rank.
    pub const GPU_VECTORS_SNAPSHOT: LockRank = LockRank(5);
    /// Dense-vector storage lock.
    pub const VECTORS: LockRank = LockRank(10);
    /// HNSW layer-structure lock.
    pub const LAYERS: LockRank = LockRank(20);
    /// HNSW neighbor-list lock — highest core rank.
    pub const NEIGHBORS: LockRank = LockRank(30);

    /// Inclusive lower bound of the reserved premium rank range.
    ///
    /// Core never assigns ranks at or above this value; premium declares
    /// cluster-state / tenant-store / server-level ranks within `[40, 59]`
    /// without colliding with core.
    pub const PREMIUM_MIN: u8 = 40;
    /// Inclusive upper bound of the reserved premium rank range.
    pub const PREMIUM_MAX: u8 = 59;

    /// Returns the underlying ordinal value.
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        self.0
    }

    /// Constructs a premium-owned rank, clamped to the reserved range.
    ///
    /// Returns `None` if `value` is outside the inclusive range
    /// `[PREMIUM_MIN, PREMIUM_MAX]` (i.e. `[40, 59]`).
    #[must_use]
    pub const fn premium(value: u8) -> Option<LockRank> {
        if value >= Self::PREMIUM_MIN && value <= Self::PREMIUM_MAX {
            Some(LockRank(value))
        } else {
            None
        }
    }
}

/// Debug-only acquisition-order assertion.
///
/// Asserts that `about_to_acquire` has a strictly greater rank than
/// `previously_held`. Compiles to nothing in release builds, so it carries
/// zero release overhead.
///
/// # Panics
/// In debug builds, panics if `about_to_acquire <= previously_held`, signaling
/// a lock-order violation.
#[inline]
pub fn assert_lock_order(previously_held: LockRank, about_to_acquire: LockRank) {
    debug_assert!(
        about_to_acquire > previously_held,
        "lock-order violation: acquiring rank {} while holding rank {}",
        about_to_acquire.ordinal(),
        previously_held.ordinal()
    );
}
