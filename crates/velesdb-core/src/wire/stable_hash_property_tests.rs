//! Property-based tests for the stable cross-engine ID hashing API.
//!
//! Feature: core-control-plane-boundary, Property 6.
//! These tests live in their own module (separate from the reference-vector
//! unit tests) so the property coverage stays clearly delineated.

use super::stable_hash::{hash_id, FNV_OFFSET_BASIS, FNV_PRIME};
use proptest::prelude::*;

/// Independent, self-contained FNV-1a reference implementation used as the
/// oracle for Property 6. Deliberately written separately from the production
/// `fnv1a_fold` so a regression in the shared core cannot mask itself: the two
/// implementations must agree byte-for-byte for every input.
fn fnv1a_reference(input: &str) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

// Feature: core-control-plane-boundary, Property 6: Stable hash is
// deterministic and platform-independent — `hash_id` returns the same `u64`
// for equal inputs across invocations, and its output matches the frozen
// FNV-1a reference derivation for every input.
// **Validates: Requirements 4.1, 4.2**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Determinism: equal inputs hash to equal outputs across repeated,
    /// independent invocations (Requirement 4.1).
    #[test]
    fn prop_hash_id_is_deterministic(input in ".*") {
        let first = hash_id(&input);
        let second = hash_id(&input);
        // A freshly cloned string is an independently-owned equal input.
        let third = hash_id(&input.clone());
        prop_assert_eq!(first, second);
        prop_assert_eq!(first, third);
    }

    /// Platform-independent FNV-1a contract: `hash_id` matches an independent
    /// FNV-1a reference derivation for every input, so its output is fixed by
    /// the algorithm rather than any process/run-specific state
    /// (Requirements 4.1, 4.2).
    #[test]
    fn prop_hash_id_matches_fnv1a_reference(input in ".*") {
        prop_assert_eq!(hash_id(&input), fnv1a_reference(&input));
    }

    /// Distinct inputs do not silently alias through some accidental identity:
    /// whenever two generated strings differ, agreement with the FNV-1a
    /// reference still holds for both (guards against a degenerate constant
    /// hash while staying a true property over the input space).
    #[test]
    fn prop_hash_id_tracks_reference_for_pairs(a in ".*", b in ".*") {
        prop_assert_eq!(hash_id(&a), fnv1a_reference(&a));
        prop_assert_eq!(hash_id(&b), fnv1a_reference(&b));
        if a != b {
            // Equal hashes are only permissible via a genuine FNV-1a collision;
            // in that case both must still equal the reference (checked above).
            prop_assert_eq!(hash_id(&a) == hash_id(&b), fnv1a_reference(&a) == fnv1a_reference(&b));
        }
    }
}
