//! Property-based tests for the cross-implementation conformance harness.
//!
//! Feature: core-control-plane-boundary, Property 10.
//! These tests live in their own module (separate from the executor golden-case
//! unit tests) so the property coverage stays clearly delineated.

use super::{check_executor, check_rrf, check_stable_hash};
use crate::fusion::FusionStrategy;
use crate::{hash_edge_id, hash_id};
use proptest::prelude::*;

/// Core's reference RRF fusion rendered as the harness candidate signature
/// `Fn(inputs, k) -> Vec<(u64, f32)>`. Delegates to the real
/// [`FusionStrategy::RRF`] so an agreeing candidate reproduces the frozen
/// golden table exactly.
fn reference_rrf(inputs: Vec<Vec<(u64, f32)>>, k: u32) -> Vec<(u64, f32)> {
    FusionStrategy::RRF { k }.fuse(inputs).unwrap_or_default()
}

// Feature: core-control-plane-boundary, Property 10: Conformance harness
// detects any divergence — a candidate differing on ≥1 input yields a non-empty
// divergence set identifying the case, while a fully-agreeing candidate (the
// real reference function) yields none.
// **Validates: Requirements 7.1, 7.2, 7.4**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Stable-hash (Req 7.1): the real `hash_id` agrees with the frozen golden
    /// table (empty divergence set), while a candidate XORing every output with
    /// a non-zero mask differs on every input and is detected, with the
    /// divergence entry identifying the diverging case.
    #[test]
    fn prop_hash_divergence_detected(mask in 1_u64..=u64::MAX) {
        // Agreeing candidate: the real reference function yields no divergence.
        prop_assert!(check_stable_hash(hash_id).is_empty());

        // Perturbed candidate: `mask != 0` guarantees a different output for
        // every input, so the harness must flag a non-empty divergence set.
        let perturbed = |s: &str| hash_id(s) ^ mask;
        let divergences = check_stable_hash(perturbed);
        prop_assert!(!divergences.is_empty());

        // The divergence set identifies the diverging case with distinct
        // expected/actual renderings (Req 7.4).
        let first = &divergences[0];
        prop_assert!(!first.case.is_empty());
        prop_assert_ne!(&first.expected, &first.actual);
    }

    /// Shared executor (Req 7.3, reported per Req 7.4): the real `hash_edge_id`
    /// agrees with the golden table, while a candidate offsetting every output
    /// by a non-zero delta is detected as divergent.
    #[test]
    fn prop_executor_divergence_detected(delta in 1_u64..=u64::MAX) {
        prop_assert!(check_executor(hash_edge_id).is_empty());

        // `wrapping_add(delta)` with `delta != 0` changes every output.
        let perturbed = |source: u64, target: u64, label: &str| {
            hash_edge_id(source, target, label).wrapping_add(delta)
        };
        let divergences = check_executor(perturbed);
        prop_assert!(!divergences.is_empty());

        let first = &divergences[0];
        prop_assert!(!first.case.is_empty());
        prop_assert_ne!(&first.expected, &first.actual);
    }

    /// Score fusion (Req 7.2): the real RRF agrees with the golden table, while
    /// a candidate that remaps every fused document id by a non-zero delta
    /// (clearly divergent: the reference ids no longer match) is detected.
    #[test]
    fn prop_rrf_divergence_detected(id_delta in 1_u64..=u64::MAX) {
        prop_assert!(check_rrf(reference_rrf).is_empty());

        // Remap every fused doc id; `id_delta != 0` guarantees the ids no
        // longer match the golden output, so ordering-preserving scores cannot
        // mask the divergence.
        let perturbed = |inputs: Vec<Vec<(u64, f32)>>, k: u32| {
            reference_rrf(inputs, k)
                .into_iter()
                .map(|(id, score)| (id.wrapping_add(id_delta), score))
                .collect::<Vec<_>>()
        };
        let divergences = check_rrf(perturbed);
        prop_assert!(!divergences.is_empty());

        let first = &divergences[0];
        prop_assert!(!first.case.is_empty());
        prop_assert_ne!(&first.expected, &first.actual);
    }
}
