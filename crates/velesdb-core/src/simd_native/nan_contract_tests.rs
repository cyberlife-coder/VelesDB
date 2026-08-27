//! What the distance kernels actually do with a NaN, measured (#2106 items 4, 16).
//!
//! These tests assert **current x86 behaviour**, not desired behaviour. That is
//! deliberate. The audit recorded NaN divergence as an aarch64 concern — NEON's
//! `vminq_f32` propagates NaN while the scalar path's `f32::min` returns the
//! non-NaN operand. Measuring it on x86 showed something the record did not
//! have: the divergence is live on the default platform too, and it is worse
//! than a scalar/SIMD split.
//!
//! `jaccard_similarity_native(a, b)` on x86 returns a finite score when the NaN
//! sits in `a`, and `NaN` when the identical NaN sits in `b` — from dimension 8
//! upward, where the AVX2 kernel takes over from the scalar path. The cause is
//! that `_mm256_min_ps(va, vb)` yields `vb` whenever either operand is NaN,
//! whereas `f32::min` yields whichever operand is *not* NaN. Jaccard is
//! symmetric by definition, so `J(a, b) != J(b, a)` is a broken contract.
//!
//! The fix is not to branch on NaN inside the hot loops — that taxes every
//! well-formed query to serve a malformed one. It is
//! [`crate::validation::validate_vector_is_finite`], which refuses the value at
//! the cold boundary so no kernel ever sees it. These tests exist so that guard
//! can never be deleted as "probably unnecessary": they are the evidence of
//! what returns the moment it goes.
//!
//! Scoped to x86_64 deliberately. The aarch64 half of item 4 stays *unmeasured*
//! rather than asserted from the intrinsic's documentation: `vminq_f32` is
//! specified to propagate NaN, which would make NEON Jaccard return NaN from
//! either argument, but nothing here has run on aarch64 and a test asserting an
//! unverified behaviour is worse than no test. The boundary guard makes the
//! question moot in practice on both architectures; whoever gets an aarch64
//! runner should point these same probes at it and record what they see.

#![allow(clippy::cast_precision_loss)]

use super::{hamming_distance_native, jaccard_similarity_native};

/// Dimensions straddling the scalar/AVX2 handover, where the split appears.
const DIMS: &[usize] = &[3, 4, 8, 16, 17, 64, 520, 1030];

fn pair(dim: usize) -> (Vec<f32>, Vec<f32>) {
    (
        (0..dim).map(|i| (i % 3) as f32 * 0.4).collect(),
        (0..dim).map(|i| ((i + 1) % 3) as f32 * 0.4).collect(),
    )
}

/// Jaccard is not symmetric under a NaN, and the asymmetry is dimension-
/// dependent. This is the defect the boundary guard exists to make unreachable.
#[test]
fn jaccard_is_asymmetric_under_a_nan_which_is_why_nan_is_refused_at_the_edge() {
    let mut split_seen = false;

    for &dim in DIMS {
        let (base_a, base_b) = pair(dim);

        let mut a = base_a.clone();
        a[0] = f32::NAN;
        let nan_in_a = jaccard_similarity_native(&a, &base_b);

        let mut b = base_b.clone();
        b[0] = f32::NAN;
        let nan_in_b = jaccard_similarity_native(&base_a, &b);

        // The NaN never survives when it is the *first* argument: `f32::min`
        // and `_mm256_min_ps` both hand back the other operand there.
        assert!(
            !nan_in_a.is_nan(),
            "dim {dim}: NaN in the first argument produced {nan_in_a}; if this \
             ever starts propagating, the guard's rationale has changed"
        );

        if nan_in_b.is_nan() {
            split_seen = true;
            assert!(
                dim >= 8,
                "dim {dim}: the split is expected only once the AVX2 kernel \
                 takes over at 8 lanes, but it appeared here"
            );
        }
    }

    assert!(
        split_seen,
        "no dimension propagated a NaN from the second argument. Either the \
         kernels were fixed — in which case delete this test and say so — or \
         the dispatch no longer reaches the AVX2 Jaccard kernel at all, which \
         is a coverage regression worth knowing about."
    );
}

/// Hamming absorbs a NaN on every path, in either argument.
///
/// Its threshold comparison is ordered (`x > 0.5` is `false` for NaN, and
/// `_mm256_cmp_ps` with `_CMP_GT_OQ` agrees), so a NaN reads as a zero bit
/// rather than poisoning the count. Pinned to keep the two metrics' differing
/// stories straight: only Jaccard has the asymmetry.
#[test]
fn hamming_absorbs_a_nan_in_either_argument() {
    for &dim in DIMS {
        let (base_a, base_b) = pair(dim);

        let mut a = base_a.clone();
        a[0] = f32::NAN;
        let mut b = base_b.clone();
        b[0] = f32::NAN;

        assert!(
            !hamming_distance_native(&a, &base_b).is_nan(),
            "dim {dim}, a"
        );
        assert!(
            !hamming_distance_native(&base_a, &b).is_nan(),
            "dim {dim}, b"
        );
    }
}
