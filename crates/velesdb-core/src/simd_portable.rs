//! Portable SIMD evaluation module (EPIC-054/US-004).
//!
//! This module provides experimental implementations using Rust's `portable_simd`
//! feature to evaluate its viability as a replacement for architecture-specific
//! intrinsics (AVX2, NEON).
//!
//! # Requirements
//!
//! - Rust nightly toolchain
//! - `#![feature(portable_simd)]` enabled
//!
//! # Evaluation Criteria
//!
//! 1. Performance: Within 10% of intrinsics
//! 2. Code reduction: >40% fewer lines
//! 3. Cross-platform: Single implementation for all targets
//!
//! # Usage
//!
//! ```ignore
//! // Enable in lib.rs with:
//! #![cfg_attr(feature = "portable-simd", feature(portable_simd))]
//!
//! use velesdb_core::simd_portable::*;
//! let dist = l2_distance_portable(&vec_a, &vec_b);
//! ```

#![allow(unused)]

#[cfg(feature = "portable-simd")]
use std::simd::prelude::*;

/// L2 (Euclidean) distance using portable_simd.
///
/// Computes sqrt(sum((a[i] - b[i])^2)) using 8-wide SIMD lanes.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(feature = "portable-simd")]
#[must_use]
pub fn l2_distance_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");

    let mut sum = f32x8::splat(0.0);
    let chunks = a.len() / 8;

    for i in 0..chunks {
        let offset = i * 8;
        let va = f32x8::from_slice(&a[offset..]);
        let vb = f32x8::from_slice(&b[offset..]);
        let diff = va - vb;
        sum += diff * diff;
    }

    let mut result = sum.reduce_sum();

    // Handle remainder (tail elements)
    for i in (chunks * 8)..a.len() {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result.sqrt()
}

/// Dot product using portable_simd.
///
/// Computes sum(a[i] * b[i]) using 8-wide SIMD lanes.
/// May utilize FMA (fused multiply-add) if available.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(feature = "portable-simd")]
#[must_use]
pub fn dot_product_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");

    let mut sum = f32x8::splat(0.0);
    let chunks = a.len() / 8;

    for i in 0..chunks {
        let offset = i * 8;
        let va = f32x8::from_slice(&a[offset..]);
        let vb = f32x8::from_slice(&b[offset..]);
        sum += va * vb; // FMA if available
    }

    let mut result = sum.reduce_sum();

    // Handle remainder
    for i in (chunks * 8)..a.len() {
        result += a[i] * b[i];
    }

    result
}

/// Cosine similarity using portable_simd.
///
/// Computes dot(a, b) / (||a|| * ||b||).
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(feature = "portable-simd")]
#[must_use]
pub fn cosine_similarity_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");

    let mut dot_sum = f32x8::splat(0.0);
    let mut a_norm_sum = f32x8::splat(0.0);
    let mut b_norm_sum = f32x8::splat(0.0);

    let chunks = a.len() / 8;

    for i in 0..chunks {
        let offset = i * 8;
        let va = f32x8::from_slice(&a[offset..]);
        let vb = f32x8::from_slice(&b[offset..]);

        dot_sum += va * vb;
        a_norm_sum += va * va;
        b_norm_sum += vb * vb;
    }

    let mut dot = dot_sum.reduce_sum();
    let mut a_norm = a_norm_sum.reduce_sum();
    let mut b_norm = b_norm_sum.reduce_sum();

    // Handle remainder
    for i in (chunks * 8)..a.len() {
        dot += a[i] * b[i];
        a_norm += a[i] * a[i];
        b_norm += b[i] * b[i];
    }

    let denom = (a_norm * b_norm).sqrt();
    if denom > f32::EPSILON {
        dot / denom
    } else {
        0.0
    }
}

/// Squared L2 distance (no sqrt) using portable_simd.
///
/// Faster than `l2_distance_portable` when only relative distances matter.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(feature = "portable-simd")]
#[must_use]
pub fn l2_squared_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");

    let mut sum = f32x8::splat(0.0);
    let chunks = a.len() / 8;

    for i in 0..chunks {
        let offset = i * 8;
        let va = f32x8::from_slice(&a[offset..]);
        let vb = f32x8::from_slice(&b[offset..]);
        let diff = va - vb;
        sum += diff * diff;
    }

    let mut result = sum.reduce_sum();

    for i in (chunks * 8)..a.len() {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}

// Scalar fallback implementations for when portable-simd feature is disabled

/// L2 distance scalar fallback.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(not(feature = "portable-simd"))]
#[must_use]
pub fn l2_distance_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// Dot product scalar fallback.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(not(feature = "portable-simd"))]
#[must_use]
pub fn dot_product_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Cosine similarity scalar fallback.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(not(feature = "portable-simd"))]
#[must_use]
pub fn cosine_similarity_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let a_norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let b_norm: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denom = a_norm * b_norm;
    if denom > f32::EPSILON {
        dot / denom
    } else {
        0.0
    }
}

/// Squared L2 distance scalar fallback.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[cfg(not(feature = "portable-simd"))]
#[must_use]
pub fn l2_squared_portable(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");
    a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32, epsilon: f32) -> bool {
        (a - b).abs() < epsilon
    }

    #[test]
    fn test_l2_distance_portable_identical() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let dist = l2_distance_portable(&a, &b);
        assert!(approx_eq(dist, 0.0, 1e-6));
    }

    #[test]
    fn test_l2_distance_portable_known() {
        let a = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let dist = l2_distance_portable(&a, &b);
        assert!(approx_eq(dist, 5.0, 1e-6)); // 3-4-5 triangle
    }

    #[test]
    fn test_l2_distance_portable_odd_length() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // Not multiple of 8
        let b = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let dist = l2_distance_portable(&a, &b);
        assert!(approx_eq(dist, 0.0, 1e-6));
    }

    #[test]
    fn test_dot_product_portable_basic() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let dot = dot_product_portable(&a, &b);
        assert!(approx_eq(dot, 36.0, 1e-6)); // 1+2+3+4+5+6+7+8 = 36
    }

    #[test]
    fn test_dot_product_portable_orthogonal() {
        let a = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let dot = dot_product_portable(&a, &b);
        assert!(approx_eq(dot, 0.0, 1e-6));
    }

    #[test]
    fn test_cosine_similarity_portable_identical() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let sim = cosine_similarity_portable(&a, &b);
        assert!(approx_eq(sim, 1.0, 1e-6));
    }

    #[test]
    fn test_cosine_similarity_portable_orthogonal() {
        let a = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let sim = cosine_similarity_portable(&a, &b);
        assert!(approx_eq(sim, 0.0, 1e-6));
    }

    #[test]
    fn test_l2_squared_portable() {
        let a = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let dist_sq = l2_squared_portable(&a, &b);
        assert!(approx_eq(dist_sq, 25.0, 1e-6)); // 9 + 16 = 25
    }

    #[test]
    fn test_large_vectors() {
        let a: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let b: Vec<f32> = (0..768).map(|i| i as f32 * 0.001 + 0.1).collect();

        let dist = l2_distance_portable(&a, &b);
        assert!(dist > 0.0);

        let dot = dot_product_portable(&a, &b);
        assert!(dot > 0.0);

        let sim = cosine_similarity_portable(&a, &b);
        assert!(sim > 0.9); // Very similar vectors
    }
}
