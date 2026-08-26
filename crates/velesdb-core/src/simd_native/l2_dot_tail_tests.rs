//! Masked-tail coverage for the squared-L2 and dot-product kernels.
//!
//! Companion to `cosine_tail_tests.rs`, which closed the same gap for cosine
//! (#2106 item 14). The dispatch thresholds are identical for all three —
//! `>= 1024` picks the 8-accumulator kernel, `>= 512` the 4-accumulator one,
//! anything smaller the 2-accumulator one — but the remainder handling is not
//! the same shape, and the dimensions that reach it differ per kernel:
//!
//! | kernel | main loop | remainder |
//! |---|---|---|
//! | 2-acc (`< 512`) | `len / 16` chunks of 16 | one masked chunk, `len % 16` |
//! | 4-acc (`512..1024`) | `len / 64 * 64` | 16-wide bridge loop, then one masked chunk |
//! | 8-acc (`>= 1024`) | `len / 128 * 128` | 16-wide bridge loop, then one masked chunk |
//!
//! The two-stage remainder is what the existing suite missed twice over. A
//! dimension that is an exact multiple of the stride skips both stages; a
//! dimension whose leftover is an exact multiple of 16 runs the bridge but
//! never the mask; and a leftover below 16 runs the mask but never the bridge.
//! Only a leftover that is neither exercises both.
//!
//! What the suite covered before this file, checked rather than assumed:
//!
//! - `distance_engine_tests.rs` uses 128, 256, 384, 512, 768, 1024, 1536 and
//!   3072. Every one is a multiple of 16, and every one at or above 512 is an
//!   exact multiple of its kernel's stride — so none reaches any remainder.
//! - `simd_native_dispatch_tests.rs` adds 100, 127 and 255, which do reach the
//!   **2-acc** mask. Nothing it names at or above 512 is a non-multiple.
//! - `warmup_tests.rs` looks like it reaches the 4-acc tail at 767, but that
//!   767 is a *value* inside the generator; the vector is 768 long, a multiple
//!   of 64.
//!
//! So the 2-acc mask was already exercised and the 4-acc and 8-acc remainders
//! — both stages of each — were not.
//!
//! One qualification, because the equivalent claim was overstated once in the
//! #2106 record: `tests/simd_property_tests.simd-property-regressions` holds a
//! checked-in proptest seed with 1509-element vectors, and proptest replays
//! persisted seeds keyed by *source file*, so the 8-acc tails do get some
//! deterministic coverage today. What they do not get is a named test that
//! says which boundary it protects and fails legibly when it breaks.

#![allow(clippy::cast_precision_loss)]

use super::{batch_dot_product_native, dot_product_native, squared_l2_native};

/// Dimensions chosen so that every remainder stage of every kernel runs.
///
/// | dim | kernel | leftover after the main loop | reaches |
/// |---|---|---|---|
/// | 17 | 2-acc | 1 | mask |
/// | 100 | 2-acc | 4 | mask |
/// | 511 | 2-acc | 15 | mask (widest) |
/// | 513 | 4-acc | 1 | mask only |
/// | 533 | 4-acc | 21 | bridge once, then mask of 5 |
/// | 544 | 4-acc | 32 | bridge twice, mask skipped |
/// | 575 | 4-acc | 63 | bridge three times, mask of 15 |
/// | 1025 | 8-acc | 1 | mask only |
/// | 1041 | 8-acc | 17 | bridge once, then mask of 1 |
/// | 1152 | 8-acc | 0 | control — neither stage |
/// | 1279 | 8-acc | 127 | bridge seven times, mask of 15 |
///
/// The exact multiples (512, 1024, 1152, 2048) are controls: they skip the
/// remainder entirely, so a failure there is a main-loop bug and the pair
/// localizes which half broke.
const TAIL_DIMS: &[usize] = &[
    // 2-acc, stride 16
    1, 15, 16, 17, 100, 255, 511, // 4-acc, stride 64
    512, 513, 533, 544, 575, 767, 769, 1023, // 8-acc, stride 128
    1024, 1025, 1041, 1151, 1152, 1279, 2047, 2048,
];

/// Distinct, non-degenerate vectors.
///
/// Not equal, not orthogonal, and nowhere near zero: each of those makes at
/// least one of the two metrics insensitive to a dropped element. Every
/// component contributes a term of order one, so omitting one moves the sum by
/// order one — far above f32 rounding at these magnitudes.
fn distinct_pair(dim: usize) -> (Vec<f32>, Vec<f32>) {
    let a = (0..dim).map(|i| ((i as f32) * 0.13).sin() + 1.5).collect();
    let b = (0..dim)
        .map(|i| ((i as f32) * 0.17 + 1.0).cos() + 1.5)
        .collect();
    (a, b)
}

/// The true squared L2, accumulated in `f64`.
///
/// The reference is deliberately not an f32 scalar loop: that would be one
/// more approximation with its own summation order, and a disagreement would
/// not say which side was wrong. `f64` is the value both are approximating.
fn squared_l2_reference(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let d = f64::from(*x) - f64::from(*y);
            d * d
        })
        .sum()
}

/// The true dot product, accumulated in `f64`. See [`squared_l2_reference`].
fn dot_reference(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum()
}

/// Relative bound between a dispatched f32 kernel and the `f64` truth.
///
/// These are sums of up to ~2048 positive terms, so the absolute error grows
/// with the result and an absolute bound would either be vacuous at dim 2048
/// or impossible at dim 17. `1e-5` relative leaves roughly two orders of
/// magnitude over f32 accumulation noise while staying far below the ~1/dim
/// effect of a dropped element.
const RELATIVE_TOLERANCE: f64 = 1e-5;

fn assert_close(dispatched: f32, reference: f64, dim: usize, what: &str) {
    let dispatched = f64::from(dispatched);
    let error = (dispatched - reference).abs();
    let bound = reference.abs() * RELATIVE_TOLERANCE;
    assert!(
        error <= bound,
        "{what} at dim {dim}: dispatched {dispatched} vs reference {reference} \
         (error {error}, bound {bound})"
    );
}

/// Squared L2 must agree with the truth at every dimension that reaches a tail.
#[test]
fn squared_l2_matches_the_reference_across_every_tail_boundary() {
    for &dim in TAIL_DIMS {
        let (a, b) = distinct_pair(dim);
        assert_close(
            squared_l2_native(&a, &b),
            squared_l2_reference(&a, &b),
            dim,
            "squared L2",
        );
    }
}

/// Dot product must agree with the truth at every dimension that reaches a tail.
#[test]
fn dot_product_matches_the_reference_across_every_tail_boundary() {
    for &dim in TAIL_DIMS {
        let (a, b) = distinct_pair(dim);
        assert_close(
            dot_product_native(&a, &b),
            dot_reference(&a, &b),
            dim,
            "dot product",
        );
    }
}

/// The tail must carry its elements, not silently drop them.
///
/// Comparing dimension `n` against `n - 1` on the same prefix isolates the
/// tail: a kernel that skipped its last element would return the same number
/// twice. This catches a dropped element even if the reference were wrong in
/// the same way, which the parity tests above cannot.
#[test]
fn one_more_element_changes_both_metrics_at_every_tail_boundary() {
    for &dim in TAIL_DIMS {
        if dim < 2 {
            continue;
        }
        let (a, b) = distinct_pair(dim);

        let l2_full = squared_l2_native(&a, &b);
        let l2_short = squared_l2_native(&a[..dim - 1], &b[..dim - 1]);
        assert!(
            (l2_full - l2_short).abs() > f32::EPSILON,
            "dim {dim}: dropping the last element left squared L2 unchanged \
             ({l2_full} vs {l2_short}) — the tail element is not consumed"
        );

        let dot_full = dot_product_native(&a, &b);
        let dot_short = dot_product_native(&a[..dim - 1], &b[..dim - 1]);
        assert!(
            (dot_full - dot_short).abs() > f32::EPSILON,
            "dim {dim}: dropping the last element left the dot product \
             unchanged ({dot_full} vs {dot_short}) — the tail is not consumed"
        );
    }
}

/// The batch dot entry point agrees with the single one at every boundary.
///
/// `batch_dot_product_native` resolves the kernel once from the dimension and
/// then applies it to every candidate, which is a second dispatch site with
/// the same thresholds and its own opportunity to pick the wrong arm. Nothing
/// tied the two together, so a divergence would have surfaced only as a
/// ranking difference between a batched and an unbatched search.
#[test]
fn batch_dot_agrees_with_single_dot_at_every_tail_boundary() {
    for &dim in TAIL_DIMS {
        let (a, b) = distinct_pair(dim);
        // A second candidate that is not the first: a one-element batch could
        // pass while the per-candidate stride was wrong.
        let c: Vec<f32> = a.iter().map(|x| x * 0.5 + 0.25).collect();

        let candidates: Vec<&[f32]> = vec![&a, &c];
        let batched = batch_dot_product_native(&candidates, &b);

        assert_eq!(batched.len(), 2, "dim {dim}: one score per candidate");
        for (index, (candidate, score)) in candidates.iter().zip(&batched).enumerate() {
            let single = dot_product_native(candidate, &b);
            assert!(
                (score - single).abs() <= single.abs() * 1e-5 + f32::EPSILON,
                "dim {dim}, candidate {index}: batch {score} vs single {single}"
            );
        }
    }
}
