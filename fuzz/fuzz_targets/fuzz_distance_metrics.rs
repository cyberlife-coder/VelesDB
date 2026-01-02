//! Fuzz target for SIMD distance calculations.
//!
//! This target tests distance metric calculations with arbitrary vectors to find:
//! - Panics on edge cases (NaN, Inf, very large/small values)
//! - Numerical stability issues
//! - SIMD alignment problems
//!
//! # Running
//!
//! ```bash
//! cd fuzz
//! cargo +nightly fuzz run fuzz_distance_metrics
//! ```

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use velesdb_core::simd::{
    cosine_similarity_fast, dot_product_fast, euclidean_distance_fast, hamming_distance_fast,
    jaccard_similarity_fast,
};

/// Fuzzing input for distance calculations.
#[derive(Arbitrary, Debug)]
struct DistanceInput {
    /// First vector (limited to reasonable size)
    vec_a: Vec<f32>,
    /// Second vector (will be truncated/padded to match vec_a length)
    vec_b: Vec<f32>,
}

fuzz_target!(|input: DistanceInput| {
    // Skip empty vectors
    if input.vec_a.is_empty() {
        return;
    }

    // Limit vector size to prevent OOM
    let max_dim = 2048;
    let dim = input.vec_a.len().min(max_dim);

    let a: Vec<f32> = input.vec_a.into_iter().take(dim).collect();

    // Make vec_b the same dimension
    let mut b: Vec<f32> = input.vec_b.into_iter().take(dim).collect();
    b.resize(dim, 0.0);

    // Test all distance metrics - none should panic
    let _ = cosine_similarity_fast(&a, &b);
    let _ = euclidean_distance_fast(&a, &b);
    let _ = dot_product_fast(&a, &b);
    let _ = hamming_distance_fast(&a, &b);
    let _ = jaccard_similarity_fast(&a, &b);
});
