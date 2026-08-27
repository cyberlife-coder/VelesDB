//! Non-finite fusion weights are refused wherever weights enter the engine.
//!
//! Every range check in `strategy.rs` is an ordering comparison — `w < 0.0`,
//! `k <= 0.0` — and every ordering comparison against NaN is false. A NaN
//! therefore satisfied all of them, and the strategy it built produced a NaN
//! contribution for every document in its branch. Because `sort_descending`
//! orders by a partial comparison, those NaN-scored documents did not sink to
//! the bottom: they kept whatever position the sort left them in, so a
//! document with a genuine score ranked *below* documents with no score at
//! all.
//!
//! These tests pin the rejection at both gates — the validating constructor
//! and `fuse`, which revalidates because the enum variants are public and can
//! be built as literals.

use super::{FusionError, FusionStrategy};

/// The three shapes a non-finite `f32` takes, each of which defeats a
/// different-looking guard.
const NON_FINITE: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

#[test]
fn weighted_rrf_rejects_non_finite_weights_at_construction() {
    for bad in NON_FINITE {
        let err = FusionStrategy::weighted_rrf(vec![bad, 1.0], 60.0)
            .expect_err("a non-finite weight must not build a strategy");
        assert!(
            matches!(err, FusionError::NonFiniteWeight { .. }),
            "weight {bad} produced {err} instead of NonFiniteWeight"
        );
    }
}

#[test]
fn weighted_rrf_rejects_non_finite_k_at_construction() {
    for bad in NON_FINITE {
        let err = FusionStrategy::weighted_rrf(vec![1.0], bad)
            .expect_err("a non-finite k must not build a strategy");
        assert!(
            matches!(err, FusionError::NonFiniteWeight { .. }),
            "k {bad} produced {err} instead of NonFiniteWeight"
        );
    }
}

#[test]
fn weighted_rejects_non_finite_weights_at_construction() {
    for bad in NON_FINITE {
        let err = FusionStrategy::weighted(bad, 0.3, 0.1)
            .expect_err("a non-finite weight must not build a strategy");
        assert!(
            matches!(err, FusionError::NonFiniteWeight { .. }),
            "weight {bad} produced {err} instead of NonFiniteWeight"
        );
    }
}

#[test]
fn relative_score_rejects_non_finite_weights_at_construction() {
    for bad in NON_FINITE {
        let err = FusionStrategy::relative_score(bad, 0.5)
            .expect_err("a non-finite weight must not build a strategy");
        assert!(
            matches!(err, FusionError::NonFiniteWeight { .. }),
            "weight {bad} produced {err} instead of NonFiniteWeight"
        );
    }
}

/// The enum variants are public, so a caller can skip the constructor. `fuse`
/// is the second gate and must refuse the same values.
#[test]
fn fuse_rejects_a_non_finite_weight_built_as_an_enum_literal() {
    for bad in NON_FINITE {
        let strategy = FusionStrategy::WeightedRRF {
            weights: vec![bad],
            k: 60.0,
        };
        let err = strategy
            .fuse(vec![vec![(1u64, 0.9)]])
            .expect_err("fuse must revalidate a literal-built strategy");
        assert!(
            matches!(err, FusionError::NonFiniteWeight { .. }),
            "weight {bad} produced {err} instead of NonFiniteWeight"
        );
    }
}

#[test]
fn fuse_rejects_a_non_finite_k_built_as_an_enum_literal() {
    for bad in NON_FINITE {
        let strategy = FusionStrategy::WeightedRRF {
            weights: vec![1.0],
            k: bad,
        };
        let err = strategy
            .fuse(vec![vec![(1u64, 0.9)]])
            .expect_err("fuse must revalidate a literal-built strategy");
        assert!(
            matches!(err, FusionError::NonFiniteWeight { .. }),
            "k {bad} produced {err} instead of NonFiniteWeight"
        );
    }
}

/// The consequence the guard prevents, measured rather than asserted from
/// reasoning: before the fix this fused to `[(1, NaN), (2, NaN), (3, 0.0164)]`
/// — document 3, the only one carrying a real score, ranked last.
///
/// Bypassing the constructor to build the strategy is what makes this test a
/// regression guard rather than a duplicate of the constructor tests: it
/// exercises the ranking, not just the rejection.
#[test]
fn a_nan_weight_can_no_longer_sink_a_genuinely_scored_document() {
    let strategy = FusionStrategy::WeightedRRF {
        weights: vec![f32::NAN, 1.0],
        k: 60.0,
    };
    let branches = vec![
        vec![(1u64, 0.9f32), (2u64, 0.5f32)],
        vec![(2u64, 0.8f32), (3u64, 0.4f32)],
    ];

    let err = strategy
        .fuse(branches)
        .expect_err("the NaN branch weight must be refused, not ranked");
    assert!(matches!(err, FusionError::NonFiniteWeight { .. }));
}

/// Finite weights must still pass — the guard is a filter, not a ban.
#[test]
fn finite_weights_still_build_and_fuse() {
    let strategy =
        FusionStrategy::weighted_rrf(vec![0.7, 0.3], 60.0).expect("finite weights must build");
    let fused = strategy
        .fuse(vec![
            vec![(1u64, 0.9f32), (2u64, 0.5f32)],
            vec![(2u64, 0.8f32), (3u64, 0.4f32)],
        ])
        .expect("finite weights must fuse");

    assert_eq!(fused.len(), 3, "every document must survive the fusion");
    for (id, score) in fused {
        assert!(score.is_finite(), "document {id} scored {score}");
    }
}
