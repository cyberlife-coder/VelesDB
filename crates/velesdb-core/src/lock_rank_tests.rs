//! Tests for the compiled lock-rank invariant ([`super::LockRank`]).
//!
//! Unit tests pin the reserved premium-range boundaries and the core
//! acquisition order. The property-based test validates Correctness Property 7:
//! lock ranks form a total order with a reserved premium range.

use super::{assert_lock_order, LockRank};
use proptest::prelude::*;

/// Highest core rank (`NEIGHBORS`); every premium rank must exceed it.
const MAX_CORE_ORDINAL: u8 = 30;

/// Constructs a rank from an arbitrary ordinal for order testing.
///
/// Available here because this module is a descendant of the module that
/// defines the private `LockRank(u8)` field.
fn rank(value: u8) -> LockRank {
    LockRank(value)
}

// =============================================================================
// Unit tests — reserved premium range boundaries
// =============================================================================

#[test]
fn test_premium_accepts_inclusive_lower_bound() {
    assert!(LockRank::premium(40).is_some());
}

#[test]
fn test_premium_accepts_inclusive_upper_bound() {
    assert!(LockRank::premium(59).is_some());
}

#[test]
fn test_premium_rejects_just_below_range() {
    assert!(LockRank::premium(39).is_none());
}

#[test]
fn test_premium_rejects_just_above_range() {
    assert!(LockRank::premium(60).is_none());
}

#[test]
fn test_premium_rejects_zero() {
    assert!(LockRank::premium(0).is_none());
}

// =============================================================================
// Unit tests — core acquisition order
// =============================================================================

#[test]
fn test_core_ranks_are_strictly_ascending() {
    let order = [
        LockRank::GPU_VECTORS_SNAPSHOT,
        LockRank::VECTORS,
        LockRank::COLUMNAR,
        LockRank::LAYERS,
        LockRank::NEIGHBORS,
    ];
    for pair in order.windows(2) {
        // Ascending acquisition across adjacent core ranks must be allowed.
        assert_lock_order(pair[0], pair[1]);
        assert!(pair[0] < pair[1]);
    }
}

#[test]
fn test_max_core_ordinal_matches_neighbors() {
    assert_eq!(LockRank::NEIGHBORS.ordinal(), MAX_CORE_ORDINAL);
}

// =============================================================================
// Property 7 — Lock ranks are totally ordered with a reserved premium range
// Feature: core-control-plane-boundary, Property 7
// **Validates: Requirements 5.1, 5.2**
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// `premium(v)` is `Some` iff `40 <= v <= 59`, and every premium rank is
    /// strictly greater than the maximum core rank (neighbors = 30).
    ///
    /// **Validates: Requirements 5.1, 5.2**
    #[test]
    fn prop_premium_range_iff_and_above_core(v in any::<u8>()) {
        let in_range = (LockRank::PREMIUM_MIN..=LockRank::PREMIUM_MAX).contains(&v);
        match LockRank::premium(v) {
            Some(r) => {
                prop_assert!(in_range);
                prop_assert_eq!(r.ordinal(), v);
                // Every premium rank orders strictly after every core rank.
                prop_assert!(r.ordinal() > MAX_CORE_ORDINAL);
                prop_assert!(r > LockRank::NEIGHBORS);
            }
            None => prop_assert!(!in_range),
        }
    }

    /// For `a < b`, ascending acquisition `assert_lock_order(a, b)` holds and
    /// the ranks compare consistently under the derived total order.
    ///
    /// **Validates: Requirements 5.1, 5.2**
    #[test]
    fn prop_ascending_order_holds(x in any::<u8>(), y in any::<u8>()) {
        prop_assume!(x != y);
        let (lo, hi) = (x.min(y), x.max(y));
        let (low, high) = (rank(lo), rank(hi));

        // Total order: the smaller ordinal is strictly less than the larger.
        prop_assert!(low < high);
        prop_assert!(high > low);

        // Ascending acquisition must not trip the debug assertion.
        assert_lock_order(low, high);
    }
}

// In debug builds, descending acquisition (holding a higher rank while
// acquiring a strictly lower one) must trip the `debug_assert!`. In release
// builds `assert_lock_order` compiles to nothing, so this expectation only
// holds under `debug_assertions`.
#[cfg(debug_assertions)]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// For `a < b`, `assert_lock_order(b, a)` fails (panics) in debug builds.
    ///
    /// **Validates: Requirements 5.1, 5.2**
    #[test]
    fn prop_descending_order_fails_in_debug(x in any::<u8>(), y in any::<u8>()) {
        prop_assume!(x != y);
        let (lo, hi) = (x.min(y), x.max(y));
        let (low, high) = (rank(lo), rank(hi));

        // Silence the panic hook so the expected violation is not noisy.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = std::panic::catch_unwind(|| assert_lock_order(high, low));
        std::panic::set_hook(prev_hook);

        prop_assert!(outcome.is_err());
    }
}
