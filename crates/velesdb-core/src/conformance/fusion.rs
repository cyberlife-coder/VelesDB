//! Reciprocal Rank Fusion (RRF) conformance vectors (Requirement 7.2).

use super::Divergence;

/// Maximum absolute score difference tolerated between a candidate RRF score
/// and the frozen golden score.
///
/// RRF scores are `f32` and may differ in the last bits between
/// implementations that accumulate in `f32` vs `f64`. Document IDs and their
/// descending order must match exactly; only the score magnitude is compared
/// with this tolerance. Chosen small enough that a genuine formula divergence
/// (e.g. 0-based vs 1-based ranks) is always detected.
const RRF_SCORE_EPSILON: f32 = 1e-6;

/// One RRF reference case: the per-branch ranked inputs, the `k` constant, and
/// the frozen golden fused output (document IDs in descending fused-score
/// order with their scores).
#[derive(Debug, Clone, PartialEq)]
pub struct FusionVector {
    /// Per-branch ranked `(doc_id, score)` lists. RRF is rank-based, so the
    /// input score magnitudes only determine intra-branch ordering.
    pub inputs: Vec<Vec<(u64, f32)>>,
    /// The RRF ranking constant.
    pub k: u32,
    /// The frozen golden fused result: `(doc_id, fused_score)` in descending
    /// score order.
    pub expected: Vec<(u64, f32)>,
}

/// The frozen golden table for RRF fusion.
///
/// Scores follow the RRF contract `score = Σ 1/(k + rank + 1)` (1-based rank)
/// as implemented by [`crate::fusion::FusionStrategy::RRF`]. Cases are chosen
/// so every fused score is distinct, making the descending order deterministic
/// regardless of hash-map iteration order or unstable-sort tie handling.
#[must_use]
pub fn rrf_reference_vectors() -> Vec<FusionVector> {
    vec![
        // Single branch, k=60: three docs at ranks 0,1,2.
        FusionVector {
            inputs: vec![vec![(10, 0.9), (20, 0.8), (30, 0.7)]],
            k: 60,
            expected: vec![(10, 0.016_393_4), (20, 0.016_129_0), (30, 0.015_873_0)],
        },
        // Two branches, k=60: doc 2 appears in both (highest fused score).
        FusionVector {
            inputs: vec![vec![(1, 0.9), (2, 0.8)], vec![(2, 0.7), (3, 0.6)]],
            k: 60,
            expected: vec![(2, 0.032_522_5), (1, 0.016_393_4), (3, 0.016_129_0)],
        },
        // Single branch, k=10: smaller k weights top ranks more heavily.
        FusionVector {
            inputs: vec![vec![(100, 0.5), (200, 0.4)]],
            k: 10,
            expected: vec![(100, 0.090_909_1), (200, 0.083_333_3)],
        },
    ]
}

/// Returns `true` when `actual` matches `expected`: identical length, identical
/// document IDs in identical order, and scores within [`RRF_SCORE_EPSILON`].
fn fused_matches(expected: &[(u64, f32)], actual: &[(u64, f32)]) -> bool {
    expected.len() == actual.len()
        && expected
            .iter()
            .zip(actual)
            .all(|(e, a)| e.0 == a.0 && (e.1 - a.1).abs() <= RRF_SCORE_EPSILON)
}

/// Runs every RRF reference case against `fuse_fn` and returns the diverging
/// cases (empty = full agreement).
///
/// `fuse_fn` takes the per-branch ranked inputs and the `k` constant and
/// returns the fused `(doc_id, score)` list in descending order. Core is
/// verified by passing a closure over
/// [`crate::fusion::FusionStrategy::RRF`]; premium passes its own fusion
/// (Requirement 7.5).
#[must_use]
pub fn check_rrf(
    fuse_fn: impl Fn(Vec<Vec<(u64, f32)>>, u32) -> Vec<(u64, f32)>,
) -> Vec<Divergence> {
    let mut divergences = Vec::new();
    for vector in rrf_reference_vectors() {
        let actual = fuse_fn(vector.inputs.clone(), vector.k);
        if !fused_matches(&vector.expected, &actual) {
            divergences.push(Divergence {
                case: format!("rrf(k={}, inputs={:?})", vector.k, vector.inputs),
                expected: format!("{:?}", vector.expected),
                actual: format!("{actual:?}"),
            });
        }
    }
    divergences
}
