#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
//! Property-based equivalence tests for native SIMD distance primitives.
//!
//! These tests compare public SIMD entrypoints against **f64 ground-truth
//! references** using Higham's proven forward error bound.  The f64 references
//! eliminate the false-failure problem where f32 scalar and f32 SIMD accumulate
//! rounding errors in different directions.
//!
//! Reference: Higham, "Accuracy and Stability of Numerical Algorithms", 2002.

use proptest::{
    collection::vec,
    prelude::{prop_assert, prop_assert_eq, prop_oneof, Just, Strategy},
    proptest,
    test_runner::{Config as ProptestConfig, FileFailurePersistence},
};
use velesdb_core::simd_native::{
    cosine_similarity_native, dot_product_native, euclidean_native, hamming_distance_native,
    jaccard_similarity_native, squared_l2_native,
};

const SIMD_PROP_CASES: u32 = 256;
const SIMD_PROP_MAX_SHRINK_ITERS: u32 = 2048;
const SIMD_PROP_REGRESSION_SUFFIX: &str = "simd-property-regressions";

// ---------------------------------------------------------------------------
// f64 ground-truth reference functions
// ---------------------------------------------------------------------------

/// f64 ground truth for dot product.
/// Returns `(exact_sum, condition_number)` where `condition_number = Σ|a[i]×b[i]|`.
fn reference_dot_f64(a: &[f32], b: &[f32]) -> (f64, f64) {
    let mut sum = 0.0_f64;
    let mut abs_sum = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let p = f64::from(*x) * f64::from(*y);
        sum += p;
        abs_sum += p.abs();
    }
    (sum, abs_sum)
}

/// f64 ground truth for squared L2 distance.
/// All terms are non-negative, so `condition_number == sum`.
fn reference_squared_l2_f64(a: &[f32], b: &[f32]) -> (f64, f64) {
    let mut sum = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = f64::from(*x) - f64::from(*y);
        sum += d * d;
    }
    (sum, sum)
}

/// f64 ground truth for cosine similarity.
fn reference_cosine_f64(a: &[f32], b: &[f32]) -> f64 {
    let (dot, _) = reference_dot_f64(a, b);
    let norm_a = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let norm_b = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }
}

/// f64 ground truth for Hamming distance (exact integer arithmetic).
fn reference_hamming_f64(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
        .count() as f64
}

/// f64 ground truth for Jaccard similarity.
///
/// Returns all intermediate values needed for a proper ratio error bound:
/// `result = intersection / union`, plus the absolute-term sums for each
/// accumulation and the raw union value for denominator scaling.
struct JaccardRef {
    result: f64,
    union_val: f64,
    abs_inter: f64,
    abs_union: f64,
}

fn reference_jaccard_f64(a: &[f32], b: &[f32]) -> JaccardRef {
    let mut intersection = 0.0_f64;
    let mut union_val = 0.0_f64;
    let mut abs_inter = 0.0_f64;
    let mut abs_union = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let fx = f64::from(*x);
        let fy = f64::from(*y);
        let min_val = fx.min(fy);
        let max_val = fx.max(fy);
        intersection += min_val;
        union_val += max_val;
        abs_inter += min_val.abs();
        abs_union += max_val.abs();
    }
    let result = if union_val == 0.0 {
        1.0
    } else {
        intersection / union_val
    };
    JaccardRef {
        result,
        union_val,
        abs_inter,
        abs_union,
    }
}

/// Compute the error bound for Jaccard = I/U through ratio error propagation.
///
/// |error| ≤ (ΔI + |I/U| × ΔU) / |U| + u × |I/U|
///
/// where ΔI = γ(N) × `Σ|min_terms`|, ΔU = γ(N) × `Σ|max_terms`|.
fn jaccard_error_bound(n: usize, jref: &JaccardRef) -> f64 {
    if jref.union_val.abs() < f64::EPSILON {
        return f64::from(f32::EPSILON);
    }
    let inter_err = higham_bound(n, jref.abs_inter);
    let union_err = higham_bound(n, jref.abs_union);
    let u = f64::from(f32::EPSILON) / 2.0;
    let abs_result = jref.result.abs();
    let abs_union = jref.union_val.abs();
    // Propagate numerator + denominator errors through division
    ((inter_err + abs_result * union_err) / abs_union + u * abs_result).max(f64::from(f32::EPSILON))
}

// ---------------------------------------------------------------------------
// f32 scalar references (kept for sanity test on small vectors)
// ---------------------------------------------------------------------------

fn scalar_dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn scalar_hamming(a: &[f32], b: &[f32]) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    {
        a.iter()
            .zip(b.iter())
            .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
            .count() as f32
    }
}

// ---------------------------------------------------------------------------
// Higham error bound
// ---------------------------------------------------------------------------

/// Higham's forward error bound for floating-point summation.
///
/// For a sum of N f32 terms computed in **any** order:
///   `|error| ≤ γ(N) × condition_number`
/// where `γ(N) = N × u / (1 - N × u)`, `u = f32::EPSILON / 2`.
///
/// Reference: Higham, "Accuracy and Stability of Numerical Algorithms", 2002.
fn higham_bound(n: usize, condition_number: f64) -> f64 {
    let u = f64::from(f32::EPSILON) / 2.0; // unit roundoff
    #[allow(clippy::cast_precision_loss)]
    let n_f64 = n as f64;
    let gamma = n_f64 * u / (1.0 - n_f64 * u);
    // Floor at single-precision epsilon for degenerate cases (N=0, condition=0)
    (gamma * condition_number).max(f64::from(f32::EPSILON))
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn bounded_dimension_strategy() -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(0_usize),
        Just(1_usize),
        Just(2_usize),
        Just(3_usize),
        Just(7_usize),
        Just(8_usize),
        Just(15_usize),
        Just(16_usize),
        Just(17_usize),
        Just(31_usize),
        Just(32_usize),
        Just(33_usize),
        Just(63_usize),
        Just(64_usize),
        Just(65_usize),
        Just(127_usize),
        Just(128_usize),
        Just(129_usize),
        Just(255_usize),
        Just(256_usize),
        Just(257_usize),
        Just(511_usize),
        Just(512_usize),
        Just(513_usize),
        0_usize..=1536,
    ]
}

fn finite_vector_pair_strategy() -> impl Strategy<Value = (Vec<f32>, Vec<f32>)> {
    bounded_dimension_strategy().prop_flat_map(|len| {
        let a = vec(-100.0_f32..100.0_f32, len);
        let b = vec(-100.0_f32..100.0_f32, len);
        (a, b)
    })
}

fn simd_proptest_config() -> ProptestConfig {
    ProptestConfig {
        cases: SIMD_PROP_CASES,
        max_shrink_iters: SIMD_PROP_MAX_SHRINK_ITERS,
        // Integration tests do not have a nearby lib.rs/main.rs, so set an
        // explicit persistence root for reproducible counterexamples.
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            SIMD_PROP_REGRESSION_SUFFIX,
        ))),
        ..ProptestConfig::default()
    }
}

// ---------------------------------------------------------------------------
// Property tests — f64 reference + Higham bound
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(simd_proptest_config())]

    #[test]
    fn test_dot_product_native_matches_f64_reference((a, b) in finite_vector_pair_strategy()) {
        let simd = f64::from(dot_product_native(&a, &b));
        let (reference, condition) = reference_dot_f64(&a, &b);
        let bound = higham_bound(a.len(), condition);
        prop_assert!(
            (simd - reference).abs() <= bound,
            "dot mismatch len={} simd={} ref={} delta={} bound={}",
            a.len(), simd, reference, (simd - reference).abs(), bound
        );
    }

    #[test]
    fn test_squared_l2_and_euclidean_native_match_f64_reference((a, b) in finite_vector_pair_strategy()) {
        let simd_sq = f64::from(squared_l2_native(&a, &b));
        let (ref_sq, cond_sq) = reference_squared_l2_f64(&a, &b);
        // 3x multiplier: each (a-b)² term has 3 rounding sources — subtraction
        // error appears squared (2u) plus multiplication rounding (u).
        let bound_sq = 3.0 * higham_bound(a.len(), cond_sq);
        prop_assert!(
            (simd_sq - ref_sq).abs() <= bound_sq,
            "squared_l2 mismatch len={} simd={} ref={} delta={} bound={}",
            a.len(), simd_sq, ref_sq, (simd_sq - ref_sq).abs(), bound_sq
        );

        let simd_euc = f64::from(euclidean_native(&a, &b));
        let ref_euc = ref_sq.sqrt();
        // Propagated bound through sqrt: |sqrt(x) - sqrt(y)| ≤ |x-y| / (2·sqrt(min(x,y)))
        // Inherits 2x multiplier from squared_l2 bound.
        let euc_bound = if ref_euc > 0.0 {
            (bound_sq / (2.0 * ref_euc)).max(f64::from(f32::EPSILON))
        } else {
            bound_sq.sqrt().max(f64::from(f32::EPSILON))
        };
        prop_assert!(
            (simd_euc - ref_euc).abs() <= euc_bound,
            "euclidean mismatch len={} simd={} ref={} delta={} bound={}",
            a.len(), simd_euc, ref_euc, (simd_euc - ref_euc).abs(), euc_bound
        );
    }

    #[test]
    fn test_cosine_similarity_native_matches_f64_reference((a, b) in finite_vector_pair_strategy()) {
        let simd = f64::from(cosine_similarity_native(&a, &b));
        let reference = reference_cosine_f64(&a, &b);
        // Cosine = dot / (|a|·|b|) — error propagates through division and sqrt.
        // Conservative bound: 3 × γ(N) × norm_condition to cover dot + 2 norms.
        let norm_condition = {
            let sum_a2: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum();
            let sum_b2: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum();
            sum_a2.max(sum_b2).max(1.0)
        };
        let bound = 3.0 * higham_bound(a.len(), norm_condition);
        prop_assert!(
            (simd - reference).abs() <= bound,
            "cosine mismatch len={} simd={} ref={} delta={} bound={}",
            a.len(), simd, reference, (simd - reference).abs(), bound
        );
        prop_assert!((-1.0..=1.0).contains(&(simd as f32)), "cosine out of range: {}", simd);
    }

    #[test]
    fn test_hamming_distance_native_matches_f64_reference((a, b) in finite_vector_pair_strategy()) {
        let simd = f64::from(hamming_distance_native(&a, &b));
        let reference = reference_hamming_f64(&a, &b);
        // Hamming is exact integer arithmetic — tolerance is 0.
        prop_assert_eq!(simd, reference, "hamming mismatch len={}", a.len());
    }

    #[test]
    fn test_jaccard_similarity_native_matches_f64_reference((a, b) in finite_vector_pair_strategy()) {
        let simd = f64::from(jaccard_similarity_native(&a, &b));
        let jref = reference_jaccard_f64(&a, &b);
        let bound = jaccard_error_bound(a.len(), &jref);
        prop_assert!(
            (simd - jref.result).abs() <= bound,
            "jaccard mismatch len={} simd={} ref={} delta={} bound={}",
            a.len(), simd, jref.result, (simd - jref.result).abs(), bound
        );
    }
}

// ---------------------------------------------------------------------------
// Sanity test — small known vectors, f32 scalar + f64 reference cross-check
// ---------------------------------------------------------------------------

#[test]
fn test_tolerance_matrix_sanity() {
    let a = [1.0_f32, 2.0, 3.0, 4.0];
    let b = [4.0_f32, 3.0, 2.0, 1.0];
    let n = a.len();

    // Dot product
    let simd_dot = f64::from(dot_product_native(&a, &b));
    let (ref_dot, cond_dot) = reference_dot_f64(&a, &b);
    let bound_dot = higham_bound(n, cond_dot);
    assert!(
        (simd_dot - ref_dot).abs() <= bound_dot,
        "dot sanity: simd={simd_dot} ref={ref_dot} bound={bound_dot}"
    );
    // Cross-check: f32 scalar should also be close on small vectors
    assert!(
        (f64::from(scalar_dot(&a, &b)) - ref_dot).abs() <= bound_dot,
        "dot scalar cross-check failed"
    );

    // Squared L2
    let simd_sq = f64::from(squared_l2_native(&a, &b));
    let (ref_sq, cond_sq) = reference_squared_l2_f64(&a, &b);
    let bound_sq = 3.0 * higham_bound(n, cond_sq);
    assert!(
        (simd_sq - ref_sq).abs() <= bound_sq,
        "squared_l2 sanity: simd={simd_sq} ref={ref_sq} bound={bound_sq}"
    );

    // Euclidean
    let simd_euc = f64::from(euclidean_native(&a, &b));
    let ref_euc = ref_sq.sqrt();
    let euc_bound = if ref_euc > 0.0 {
        (bound_sq / (2.0 * ref_euc)).max(f64::from(f32::EPSILON))
    } else {
        bound_sq.sqrt().max(f64::from(f32::EPSILON))
    };
    assert!(
        (simd_euc - ref_euc).abs() <= euc_bound,
        "euclidean sanity: simd={simd_euc} ref={ref_euc} bound={euc_bound}"
    );

    // Cosine
    let simd_cos = f64::from(cosine_similarity_native(&a, &b));
    let ref_cos = reference_cosine_f64(&a, &b);
    let norm_cond: f64 = a
        .iter()
        .map(|x| f64::from(*x).powi(2))
        .sum::<f64>()
        .max(b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>())
        .max(1.0);
    let cos_bound = 3.0 * higham_bound(n, norm_cond);
    assert!(
        (simd_cos - ref_cos).abs() <= cos_bound,
        "cosine sanity: simd={simd_cos} ref={ref_cos} bound={cos_bound}"
    );

    // Jaccard
    let simd_jac = f64::from(jaccard_similarity_native(&a, &b));
    let jref = reference_jaccard_f64(&a, &b);
    let jac_bound = jaccard_error_bound(n, &jref);
    assert!(
        (simd_jac - jref.result).abs() <= jac_bound,
        "jaccard sanity: simd={simd_jac} ref={} bound={jac_bound}",
        jref.result
    );

    // Hamming (exact)
    assert_eq!(
        hamming_distance_native(&a, &b),
        scalar_hamming(&a, &b),
        "hamming sanity mismatch"
    );
}
