//! Conformance tests for the fusion engine's reciprocal-rank-fusion family.
//!
//! Surface = engine API: [`velesdb_core::FusionStrategy::fuse`] over controlled
//! `(id, score)` branch lists. These pin the EXACT arithmetic of `RRF`
//! (1-based, unweighted) and `WeightedRRF` (0-based, per-branch weights),
//! including the fact that the two RRF implementations deliberately differ.
//!
//! Determinism note: `fuse` sorts UNSTABLE with no id tie-break, so we only
//! assert id ORDER for strictly-distinct scores and assert score VALUES via
//! `approx_eq` everywhere.

use velesdb_core::{FusionError, FusionStrategy};

use super::helpers::approx_eq;

/// The two fixed two-branch inputs used by the RRF / WeightedRRF scenarios.
/// Branch A ranks docs 10,20,30; branch B ranks docs 20,30,40.
fn two_branch_input() -> Vec<Vec<(u64, f32)>> {
    vec![
        vec![(10, 0.9), (20, 0.8), (30, 0.7)],
        vec![(20, 0.9), (30, 0.8), (40, 0.7)],
    ]
}

/// Look up a single document's fused score in the result list.
fn score_of(fused: &[(u64, f32)], id: u64) -> f32 {
    fused
        .iter()
        .find(|(d, _)| *d == id)
        .map(|(_, s)| *s)
        .expect("test: doc present in fused output")
}

/// The id order of the fused result list.
fn order_of(fused: &[(u64, f32)]) -> Vec<u64> {
    fused.iter().map(|(d, _)| *d).collect()
}

/// RRF{k=60} is 1-based: score = Σ 1/(60 + rank0 + 1).
/// Ground truth (f32): doc20=1/61+1/60=0.0325224, doc30=1/62+1/61=0.0320020,
/// doc10=1/61=0.0163934, doc40=1/63=0.0158730.
#[test]
fn rrf_k60_scores_and_order_are_one_based() {
    let fused = FusionStrategy::RRF { k: 60 }
        .fuse(two_branch_input())
        .expect("test: rrf k60 fuse");

    // Strictly-distinct scores -> id order is well defined.
    assert_eq!(order_of(&fused), vec![20, 30, 10, 40]);
    assert!(approx_eq(score_of(&fused, 20), 0.032_522_473, 1e-5));
    assert!(approx_eq(score_of(&fused, 30), 0.032_002_046, 1e-5));
    assert!(approx_eq(score_of(&fused, 10), 0.016_393_442, 1e-5));
    assert!(approx_eq(score_of(&fused, 40), 0.015_873_017, 1e-5));
}

/// RRF{k=1} is still 1-based: score = Σ 1/(1 + rank0 + 1).
/// Ground truth (f32): doc20=1/2+1/3=0.833333, doc30=1/3+1/4=0.583333,
/// doc10=1/2=0.5, doc40=1/4=0.25.
#[test]
fn rrf_k1_scores_and_order() {
    let fused = FusionStrategy::RRF { k: 1 }
        .fuse(two_branch_input())
        .expect("test: rrf k1 fuse");

    assert_eq!(order_of(&fused), vec![20, 30, 10, 40]);
    assert!(approx_eq(score_of(&fused, 20), 0.833_333_3, 1e-5));
    assert!(approx_eq(score_of(&fused, 30), 0.583_333_3, 1e-5));
    assert!(approx_eq(score_of(&fused, 10), 0.5, 1e-5));
    assert!(approx_eq(score_of(&fused, 40), 0.25, 1e-5));
}

/// `rrf_default()` is exactly `RRF{k:60}` — same scores as the k=60 case.
/// Ground truth: doc20 = 1/61 + 1/60 = 0.0325224 (f32).
#[test]
fn rrf_default_equals_k60() {
    let fused = FusionStrategy::rrf_default()
        .fuse(two_branch_input())
        .expect("test: rrf_default fuse");

    assert_eq!(FusionStrategy::rrf_default(), FusionStrategy::RRF { k: 60 });
    assert!(approx_eq(score_of(&fused, 20), 0.032_522_473, 1e-5));
}

/// `WeightedRRF` is 0-based: score = Σ weights[i]/(rank0_i + k); with equal
/// 0.5 weights and k=60 these scores DIFFER from the 1-based `RRF{60}`.
/// Ground truth (f32): doc20=0.5/61+0.5/60=0.0165301, doc30=0.5/62+0.5/61=0.0162612,
/// doc10=0.5/60=0.0083333, doc40=0.5/62=0.0080645. Distinct from RRF{60} doc20=0.0325224.
#[test]
fn weighted_rrf_is_zero_based_and_differs_from_rrf() {
    let fused = FusionStrategy::weighted_rrf(vec![0.5, 0.5], 60.0)
        .expect("test: weighted_rrf ctor")
        .fuse(two_branch_input())
        .expect("test: weighted_rrf fuse");

    assert_eq!(order_of(&fused), vec![20, 30, 10, 40]);
    assert!(approx_eq(score_of(&fused, 20), 0.016_530_056, 1e-6));
    assert!(approx_eq(score_of(&fused, 30), 0.016_261_237, 1e-6));
    assert!(approx_eq(score_of(&fused, 10), 0.008_333_334, 1e-6));
    assert!(approx_eq(score_of(&fused, 40), 0.008_064_516, 1e-6));

    // The 0-based WeightedRRF and the 1-based RRF{60} are genuinely different
    // formulas: doc20 is 0.0165301 here vs 0.0325224 under RRF{60}.
    let rrf60 = FusionStrategy::RRF { k: 60 }
        .fuse(two_branch_input())
        .expect("test: rrf60 fuse");
    assert!(!approx_eq(score_of(&fused, 20), score_of(&rrf60, 20), 1e-4));
}

/// `WeightedRRF` weights bias toward the higher-weighted branch (0-based).
/// Input: branchA[(1,_),(2,_)] weight 0.9, branchB[(2,_),(1,_)] weight 0.1, k=60.
/// Ground truth (f32): doc1=0.9/60+0.1/61=0.0166393, doc2=0.9/61+0.1/60=0.0164208;
/// doc1 wins because it is rank0 in the heavier branch.
#[test]
fn weighted_rrf_asymmetric_weights_favor_heavy_branch() {
    let input = vec![
        vec![(1u64, 9.0f32), (2, 8.0)],
        vec![(2u64, 9.0f32), (1, 8.0)],
    ];
    let fused = FusionStrategy::weighted_rrf(vec![0.9, 0.1], 60.0)
        .expect("test: weighted_rrf ctor")
        .fuse(input)
        .expect("test: weighted_rrf fuse");

    assert_eq!(order_of(&fused), vec![1, 2]);
    assert!(approx_eq(score_of(&fused, 1), 0.016_639_344, 1e-6));
    assert!(approx_eq(score_of(&fused, 2), 0.016_420_765, 1e-6));
}

/// Validation: a 1-weight `WeightedRRF` fed 2 branches errors at `fuse` with
/// `WeightCountMismatch` (the ctor does NOT check branch count, only `fuse` does).
/// Ground truth: `fuse_weighted_rrf` returns `Err(WeightCountMismatch{1,2})`.
#[test]
fn weighted_rrf_weight_count_mismatch_errors_at_fuse() {
    let strategy = FusionStrategy::weighted_rrf(vec![0.5], 60.0).expect("test: 1-weight ctor ok");
    let err = strategy.fuse(two_branch_input());
    assert!(matches!(
        err,
        Err(FusionError::WeightCountMismatch {
            weights: 1,
            branches: 2
        })
    ));
}

/// Validation: a negative weight is rejected by the `weighted_rrf` constructor.
/// Ground truth: `validate_non_negative` returns `Err(NegativeWeight)` before any fuse.
#[test]
fn weighted_rrf_negative_weight_rejected_at_construction() {
    let result = FusionStrategy::weighted_rrf(vec![-0.1, 1.1], 60.0);
    assert!(matches!(result, Err(FusionError::NegativeWeight { .. })));
}

/// Validation: k <= 0 is rejected by the `weighted_rrf` constructor
/// (k=0 with a rank-0 hit would otherwise produce an infinite score).
/// Ground truth: `weighted_rrf(vec![0.5,0.5], 0.0)` returns `Err(NegativeWeight{0.0})`.
#[test]
fn weighted_rrf_zero_k_rejected_at_construction() {
    let result = FusionStrategy::weighted_rrf(vec![0.5, 0.5], 0.0);
    assert!(matches!(result, Err(FusionError::NegativeWeight { .. })));
}

/// `fuse` on an empty branch list and on all-empty branches yields an empty
/// result (not an error) regardless of strategy — a degenerate-input guard.
/// Ground truth: `fuse` returns `Ok(Vec::new())` when no branch has any hit.
#[test]
fn rrf_empty_inputs_yield_empty_output() {
    let empty: Vec<Vec<(u64, f32)>> = Vec::new();
    let all_empty: Vec<Vec<(u64, f32)>> = vec![Vec::new(), Vec::new()];

    let r1 = FusionStrategy::RRF { k: 60 }
        .fuse(empty)
        .expect("test: empty fuse");
    let r2 = FusionStrategy::RRF { k: 60 }
        .fuse(all_empty)
        .expect("test: all-empty fuse");

    assert!(r1.is_empty());
    assert!(r2.is_empty());
}
