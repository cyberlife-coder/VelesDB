//! Compiled lock-rank invariant for the global lock-acquisition order.
//!
//! Lock ordering was historically documented as convention in
//! [`docs/CONCURRENCY_MODEL.md`]. This module promotes that convention to a
//! typed [`LockRank`] newtype so the global acquisition order is expressed in
//! code, with a debug-only [`assert_lock_order`] check that compiles to
//! nothing in release builds (zero release overhead).
//!
//! Locks MUST be acquired in strictly ascending rank. Core ranks occupy the
//! low ordinals (`gpu < vectors < columnar < layers < neighbors`); the
//! inclusive range `[40, 59]` is reserved for premium-owned lock classes so
//! premium can order its locks relative to core without collision.

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
    /// Columnar (`ColumnStore`) lock.
    pub const COLUMNAR: LockRank = LockRank(15);
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
