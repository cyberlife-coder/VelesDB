//! Masked-tail coverage for the cosine kernels.
//!
//! Every cosine kernel processes whole blocks and then a remainder: the
//! AVX-512 2-acc consumes 32 f32 per iteration, the 4-acc 64, the 8-acc 128.
//! A dimension that is an exact multiple of the stride never enters the tail
//! at all — `end_main == end_ptr` and the masked-load branch is dead. A bug
//! that drops or double-counts one tail element silently corrupts every
//! distance at that dimension.
//!
//! `cosine_fused_tests.rs` sizes are `[16, 32, 64, 128, 256, 384, 512, 768,
//! 1024]`. Every one above 512 is an exact multiple of its kernel's stride
//! (512 and 768 of 64; 1024 of 128), so neither the 4-acc nor the 8-acc tail
//! ran. `hamming_jaccard_tests.rs` already has the right shape for this — its
//! `THRESHOLD_DIMS` carries 1023/1024/1025 — and cosine had no equivalent.
//!
//! Two further reasons the gap was invisible:
//!
//! - `internal_bench_tests.rs` does test 767/768/769 against a scalar
//!   reference at 1e-5, which would have caught a 4-acc tail bug. It is gated
//!   behind the `internal-bench` feature, which no CI job enables, so it never
//!   runs. (It also stops at 1536 = 12 x 128, so it never reached the 8-acc
//!   tail either.)
//! - A `cosine_similarity_native(&a, &a)` assertion cannot detect a dropped
//!   element *at any tolerance*: dropping element k removes `a_k^2` from the
//!   dot product and from both norms, and `(S - a_k^2) / (S - a_k^2)` is still
//!   1.0. Identical-vector tests are not tail coverage. See
//!   [`self_similarity_cannot_detect_a_dropped_tail_element`].
//!
//! The reference here is [`scalar::cosine_scalar`] — the same formula without
//! the vectorization, which is what the kernels are supposed to reproduce.

#![allow(clippy::cast_precision_loss)]

use super::{cosine_similarity_native, scalar};

/// Tolerance between the dispatched kernel and the scalar reference.
///
/// The same 1e-5 `internal_bench_tests.rs` already holds these kernels to at
/// dims 769 and 1536. It is tight enough to see a dropped tail element by a
/// wide margin: at dim 1025 dropping one element moves the cosine by ~1.7e-4
/// median, an order of magnitude above this bound. `cosine_fused_tests.rs`
/// uses 5e-3, which is looser than the effect it would need to detect.
const TOLERANCE: f32 = 1e-5;

/// Dimensions chosen so that every kernel's remainder branch actually runs.
///
/// | dim | kernel | why |
/// |---|---|---|
/// | 17, 49 | AVX-512 2-acc | `rem0 = 16`, second masked load at `base + 16` |
/// | 100 | AVX-512 2-acc | `rem0 = 4`, partial mask, `rem1 = 0` |
/// | 511 | AVX-512 2-acc | widest 2-acc remainder |
/// | 513 | AVX-512 4-acc | main loop to 512, masked tail of 1 |
/// | 575 | AVX-512 4-acc | three 16-chunks then a 15-wide mask |
/// | 769 | AVX-512 4-acc | main loop to 768, masked tail of 1 |
/// | 1025 | AVX-512 8-acc | main loop to 1024, masked tail of 1 |
/// | 1151 | AVX-512 8-acc | seven 16-chunks then a 15-wide mask |
/// | 1279 | AVX-512 8-acc | 1152 main + 127 remainder |
///
/// The exact multiples (512, 768, 1024, 1536, 2048) are kept as controls: they
/// skip the tail entirely, so a failure there is a main-loop bug, not a tail
/// bug, and the pair localizes which one broke.
const TAIL_DIMS: &[usize] = &[
    // scalar / short-vector dispatch
    0, 1, 3, 7, 8, 15, // AVX-512 2-acc, stride 32
    16, 17, 33, 49, 100, 255, 511, // AVX-512 4-acc, stride 64
    512, 513, 575, 576, 767, 768, 769, 1023, // AVX-512 8-acc, stride 128
    1024, 1025, 1151, 1152, 1279, 1536, 2047, 2048,
];

/// Distinct, non-degenerate vectors.
///
/// Deliberately not `a == b`, not orthogonal, and not mostly zero: each of
/// those makes the cosine insensitive to a dropped element regardless of
/// tolerance. Irrational-ish phases keep every element contributing.
fn distinct_pair(dim: usize) -> (Vec<f32>, Vec<f32>) {
    let a = (0..dim).map(|i| ((i as f32) * 0.13).sin() + 1.5).collect();
    let b = (0..dim)
        .map(|i| ((i as f32) * 0.17 + 1.0).cos() + 1.5)
        .collect();
    (a, b)
}

/// The dispatched kernel must equal the scalar reference at every dimension
/// that reaches a masked tail.
#[test]
fn cosine_matches_scalar_across_every_tail_boundary() {
    for &dim in TAIL_DIMS {
        let (a, b) = distinct_pair(dim);
        let dispatched = cosine_similarity_native(&a, &b);
        let reference = scalar::cosine_scalar(&a, &b);
        assert!(
            (dispatched - reference).abs() <= TOLERANCE,
            "dim {dim}: dispatched {dispatched} vs scalar {reference} \
             (delta {})",
            (dispatched - reference).abs()
        );
    }
}

/// The tail must carry its elements, not silently drop them.
///
/// Comparing dimension `n` against dimension `n - 1` on the same prefix data
/// isolates the tail: if the kernel skipped its last element the two would
/// agree, and they must not, because the extra element changes the cosine.
/// This catches a dropped tail element even if the scalar reference were
/// wrong in the same way.
#[test]
fn one_more_element_changes_the_cosine_at_every_tail_boundary() {
    for &dim in TAIL_DIMS {
        if dim < 2 {
            continue;
        }
        let (a, b) = distinct_pair(dim);
        let full = cosine_similarity_native(&a, &b);
        let short = cosine_similarity_native(&a[..dim - 1], &b[..dim - 1]);
        assert!(
            (full - short).abs() > f32::EPSILON,
            "dim {dim}: dropping the last element left the cosine unchanged \
             ({full} vs {short}) — the tail element is not being consumed"
        );
    }
}

/// Why identical-vector assertions are not tail coverage.
///
/// This is a demonstration, not a defect: for `a == b` the kernel computes
/// `dot = norm_a_sq = norm_b_sq = S`, so omitting element k gives
/// `(S - a_k^2) / sqrt((S - a_k^2)^2)` — still 1.0. Truncating the vector
/// stands in for the omission: a `cosine(&a, &a)` assertion passes at BOTH
/// lengths, so it cannot tell them apart and is blind to a dropped tail
/// element at any tolerance. That is why the tests above use distinct
/// vectors.
///
/// The two results are not bit-identical — `S / sqrt(S * S)` lands within an
/// ulp or two of 1.0 and the ulp differs with the accumulation order — so the
/// point is made the way a real test would make it: both satisfy the same
/// `≈ 1.0` assertion.
#[test]
fn self_similarity_cannot_detect_a_dropped_tail_element() {
    for &dim in TAIL_DIMS {
        if dim < 2 {
            continue;
        }
        let (a, _) = distinct_pair(dim);
        let full = cosine_similarity_native(&a, &a);
        let truncated = cosine_similarity_native(&a[..dim - 1], &a[..dim - 1]);

        // The assertion a self-similarity test would make, at both lengths.
        for (label, value) in [("full", full), ("truncated", truncated)] {
            assert!(
                (value - 1.0).abs() < TOLERANCE,
                "dim {dim} ({label}): self-cosine is 1.0, got {value}"
            );
        }
    }
}

/// The tolerance the fused-cosine suite uses is looser than what it must catch.
///
/// `cosine_fused_tests.rs` compares against a scalar reference at `5e-3`.
/// Dropping one element at these dimensions moves the cosine by roughly
/// `1/dim`, so at dim 1025 the effect is ~1.7e-4 — two orders of magnitude
/// *below* that tolerance and therefore invisible. This pins the arithmetic
/// so the constant cannot be loosened back without the reason being restated.
#[test]
fn a_dropped_element_moves_the_cosine_less_than_the_legacy_tolerance() {
    const LEGACY_TOLERANCE: f32 = 5e-3;

    for &dim in &[513_usize, 769, 1025, 1151] {
        let (a, b) = distinct_pair(dim);
        let full = cosine_similarity_native(&a, &b);
        let dropped = cosine_similarity_native(&a[..dim - 1], &b[..dim - 1]);
        let effect = (full - dropped).abs();

        assert!(
            effect < LEGACY_TOLERANCE,
            "dim {dim}: a dropped element moves the cosine by {effect}, which \
             {LEGACY_TOLERANCE} would have caught — the premise of this test \
             no longer holds"
        );
        assert!(
            effect > TOLERANCE,
            "dim {dim}: a dropped element moves the cosine by {effect}, below \
             this suite's own {TOLERANCE} — the tolerance here needs tightening"
        );
    }
}
