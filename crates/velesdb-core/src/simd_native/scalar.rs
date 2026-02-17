//! Scalar fallback implementations for SIMD distance metrics.
//!
//! These functions serve as:
//! - Fallback on platforms without SIMD support
//! - Reference implementations for testing SIMD correctness
//! - Tail-loop handlers for SIMD remainder processing

// Allow precision-loss casts for scalar fallbacks (count -> f32 for Hamming).
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::similar_names)]

// =============================================================================
// Newton-Raphson Fast Inverse Square Root (EPIC-PERF-001)
// =============================================================================

/// Fast approximate inverse square root using Newton-Raphson iteration.
///
/// Based on the famous Quake III algorithm, adapted for modern use.
/// Provides ~1-2% accuracy with significant speedup over `1.0 / x.sqrt()`.
///
/// # Performance
///
/// - Avoids expensive `sqrt()` call from libc
/// - Uses bit manipulation + one Newton-Raphson iteration
/// - ~2x faster than standard sqrt on most CPUs
///
/// # References
///
/// - SimSIMD v5.4.0: Newton-Raphson substitution
/// - arXiv: "Bang for the Buck: Vector Search on Cloud CPUs"
#[inline]
#[must_use]
pub fn fast_rsqrt(x: f32) -> f32 {
    // SAFETY: Bit manipulation is safe for f32
    // Magic constant from Quake III, refined for f32
    let i = x.to_bits();
    let i = 0x5f37_5a86_u32.wrapping_sub(i >> 1);
    let y = f32::from_bits(i);

    // One Newton-Raphson iteration: y = y * (1.5 - 0.5 * x * y * y)
    // This gives ~1% accuracy, sufficient for cosine similarity
    let half_x = 0.5 * x;
    y * (1.5 - half_x * y * y)
}

/// Fast cosine similarity using Newton-Raphson rsqrt.
///
/// Optimized version that avoids two `sqrt()` calls by using fast_rsqrt.
/// Accuracy is within 2% of exact computation, acceptable for similarity ranking.
///
/// # Performance
///
/// - ~20-50% faster than standard cosine_similarity_native
/// - Uses single-pass dot product + norms computation
/// - Avoids libc sqrt() overhead
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[inline]
#[must_use]
pub fn cosine_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Compute dot product and squared norms in single pass
    let mut dot = 0.0_f32;
    let mut norm_a_sq = 0.0_f32;
    let mut norm_b_sq = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
    }

    // Guard against zero vectors
    if norm_a_sq == 0.0 || norm_b_sq == 0.0 {
        return 0.0;
    }

    // Use fast_rsqrt: cos = dot * rsqrt(norm_a_sq) * rsqrt(norm_b_sq)
    dot * fast_rsqrt(norm_a_sq) * fast_rsqrt(norm_b_sq)
}

/// Scalar cosine similarity (single-pass dot + norms).
///
/// Used as fallback when no SIMD path matches.
#[inline]
pub(super) fn cosine_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut norm_a_sq = 0.0_f32;
    let mut norm_b_sq = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
    }

    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

/// Scalar Hamming distance implementation.
///
/// Uses binary threshold at 0.5 for consistency with SIMD versions.
/// This is the standard interpretation for binary/categorical vectors.
#[inline]
pub(super) fn hamming_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
        .count() as f32
}

/// Scalar Jaccard similarity implementation.
#[inline]
pub(super) fn jaccard_scalar(a: &[f32], b: &[f32]) -> f32 {
    let (intersection, union) = jaccard_scalar_accum(a, b);
    if union == 0.0 {
        1.0
    } else {
        intersection / union
    }
}

/// Helper to compute Jaccard accumulator values.
#[inline]
pub(super) fn jaccard_scalar_accum(a: &[f32], b: &[f32]) -> (f32, f32) {
    a.iter()
        .zip(b.iter())
        .fold((0.0_f32, 0.0_f32), |(inter, uni), (x, y)| {
            (inter + x.min(*y), uni + x.max(*y))
        })
}
