//! Shared benchmark utilities for `VelesDB` benchmarks.
//!
//! Provides deterministic vector generation and normalization helpers
//! used across multiple benchmark files.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    dead_code
)]

/// Generates a deterministic pseudo-random vector for benchmarking.
/// Uses a simple hash-based approach for reproducibility.
#[must_use]
pub fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let x = seed.wrapping_mul(2_654_435_761) ^ (i as u64).wrapping_mul(2_246_822_519);
            let normalized = (x as f32) / (u64::MAX as f32);
            normalized * 2.0 - 1.0 // Range [-1, 1]
        })
        .collect()
}

/// Normalizes a vector to unit length (for cosine similarity).
pub fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Generates a normalized vector suitable for cosine similarity.
#[must_use]
pub fn generate_normalized_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut v = generate_vector(dim, seed);
    normalize(&mut v);
    v
}
