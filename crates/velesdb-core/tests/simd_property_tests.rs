//! Property-based equivalence tests for native SIMD distance primitives.
//!
//! These tests compare public SIMD entrypoints against scalar references over
//! randomized vectors and dimension boundaries to protect future refactors.

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

#[derive(Clone, Copy)]
struct Tolerance {
    abs: f32,
    rel: f32,
}

// Tolerance matrix: operation-specific envelopes for non-associative f32 math.
const DOT_TOLERANCE: Tolerance = Tolerance {
    abs: 1.0e-4,
    rel: 2.0e-4,
};
const SQUARED_L2_TOLERANCE: Tolerance = Tolerance {
    abs: 1.0e-4,
    rel: 2.0e-4,
};
const EUCLIDEAN_TOLERANCE: Tolerance = Tolerance {
    abs: 1.0e-4,
    rel: 2.0e-4,
};
const COSINE_TOLERANCE: Tolerance = Tolerance {
    abs: 2.0e-4,
    rel: 2.0e-4,
};
const JACCARD_TOLERANCE: Tolerance = Tolerance {
    abs: 2.0e-6,
    rel: 2.0e-6,
};

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
            "simd-property-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

fn assert_close(metric: &str, actual: f32, expected: f32, tolerance: Tolerance) {
    let delta = (actual - expected).abs();
    let rel_limit = tolerance.rel * expected.abs().max(1.0);
    let allowed = tolerance.abs.max(rel_limit);
    assert!(
        delta <= allowed,
        "{metric} mismatch: actual={actual}, expected={expected}, delta={delta}, allowed={allowed}"
    );
}

fn scalar_dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn scalar_squared_l2(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

fn scalar_cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot = scalar_dot(a, b);
    let norm_a = scalar_dot(a, a).sqrt();
    let norm_b = scalar_dot(b, b).sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }
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

fn scalar_jaccard(a: &[f32], b: &[f32]) -> f32 {
    let (intersection, union) = a
        .iter()
        .zip(b.iter())
        .fold((0.0_f32, 0.0_f32), |(inter, uni), (x, y)| {
            (inter + x.min(*y), uni + x.max(*y))
        });
    if union == 0.0 {
        1.0
    } else {
        intersection / union
    }
}

proptest! {
    #![proptest_config(simd_proptest_config())]

    #[test]
    fn test_dot_product_native_matches_scalar((a, b) in finite_vector_pair_strategy()) {
        let simd = dot_product_native(&a, &b);
        let scalar = scalar_dot(&a, &b);
        prop_assert!(
            (simd - scalar).abs() <= DOT_TOLERANCE.abs.max(DOT_TOLERANCE.rel * scalar.abs().max(1.0)),
            "dot mismatch len={} simd={} scalar={}",
            a.len(),
            simd,
            scalar
        );
    }

    #[test]
    fn test_squared_l2_native_and_euclidean_native_match_scalar((a, b) in finite_vector_pair_strategy()) {
        let simd_squared = squared_l2_native(&a, &b);
        let scalar_squared = scalar_squared_l2(&a, &b);
        prop_assert!(
            (simd_squared - scalar_squared).abs()
                <= SQUARED_L2_TOLERANCE.abs.max(SQUARED_L2_TOLERANCE.rel * scalar_squared.abs().max(1.0)),
            "squared_l2 mismatch len={} simd={} scalar={}",
            a.len(),
            simd_squared,
            scalar_squared
        );

        let simd_euclidean = euclidean_native(&a, &b);
        let scalar_euclidean = scalar_squared.sqrt();
        prop_assert!(
            (simd_euclidean - scalar_euclidean).abs()
                <= EUCLIDEAN_TOLERANCE.abs.max(EUCLIDEAN_TOLERANCE.rel * scalar_euclidean.abs().max(1.0)),
            "euclidean mismatch len={} simd={} scalar={}",
            a.len(),
            simd_euclidean,
            scalar_euclidean
        );
    }

    #[test]
    fn test_cosine_similarity_native_matches_scalar((a, b) in finite_vector_pair_strategy()) {
        let simd = cosine_similarity_native(&a, &b);
        let scalar = scalar_cosine(&a, &b);
        prop_assert!(
            (simd - scalar).abs()
                <= COSINE_TOLERANCE.abs.max(COSINE_TOLERANCE.rel * scalar.abs().max(1.0)),
            "cosine mismatch len={} simd={} scalar={}",
            a.len(),
            simd,
            scalar
        );
        prop_assert!((-1.0..=1.0).contains(&simd), "cosine out of range: {}", simd);
    }

    #[test]
    fn test_hamming_distance_native_matches_scalar((a, b) in finite_vector_pair_strategy()) {
        let simd = hamming_distance_native(&a, &b);
        let scalar = scalar_hamming(&a, &b);
        prop_assert_eq!(simd, scalar, "hamming mismatch len={}", a.len());
    }

    #[test]
    fn test_jaccard_similarity_native_matches_scalar((a, b) in finite_vector_pair_strategy()) {
        let simd = jaccard_similarity_native(&a, &b);
        let scalar = scalar_jaccard(&a, &b);
        prop_assert!(
            (simd - scalar).abs()
                <= JACCARD_TOLERANCE.abs.max(JACCARD_TOLERANCE.rel * scalar.abs().max(1.0)),
            "jaccard mismatch len={} simd={} scalar={}",
            a.len(),
            simd,
            scalar
        );
    }
}

#[test]
fn test_tolerance_matrix_sanity() {
    let a = [1.0_f32, 2.0, 3.0, 4.0];
    let b = [4.0_f32, 3.0, 2.0, 1.0];

    assert_close(
        "dot",
        dot_product_native(&a, &b),
        scalar_dot(&a, &b),
        DOT_TOLERANCE,
    );
    assert_close(
        "squared_l2",
        squared_l2_native(&a, &b),
        scalar_squared_l2(&a, &b),
        SQUARED_L2_TOLERANCE,
    );
    assert_close(
        "euclidean",
        euclidean_native(&a, &b),
        scalar_squared_l2(&a, &b).sqrt(),
        EUCLIDEAN_TOLERANCE,
    );
    assert_close(
        "cosine",
        cosine_similarity_native(&a, &b),
        scalar_cosine(&a, &b),
        COSINE_TOLERANCE,
    );
    assert_close(
        "jaccard",
        jaccard_similarity_native(&a, &b),
        scalar_jaccard(&a, &b),
        JACCARD_TOLERANCE,
    );
    assert_eq!(
        hamming_distance_native(&a, &b),
        scalar_hamming(&a, &b),
        "hamming sanity mismatch"
    );
}
