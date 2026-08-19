#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Zero-overhead SIMD function dispatch.
//!
//! This module provides a thin wrapper around `simd_native` functions,
//! offering a stable public API while `simd_native` handles the
//! architecture-specific SIMD implementations internally.
//!
//! # EPIC-C.2: TS-SIMD-002

// Reason: Numeric casts in SIMD dispatch are intentional:
// - usize->u32 for Hamming distance: vector dimensions bounded by implementation
// - Maximum dimension is 65536, result fits in u32

// =============================================================================
// Public dispatch API - Direct calls to simd_native
// =============================================================================

/// Compute dot product with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn dot_product_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::dot_product_native(a, b)
}

/// Compute Euclidean distance with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn euclidean_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::euclidean_native(a, b)
}

/// Compute cosine similarity with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn cosine_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::cosine_similarity_native(a, b)
}

/// Compute cosine similarity for pre-normalized vectors.
#[inline]
#[must_use]
pub fn cosine_normalized_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::cosine_normalized_native(a, b)
}

/// Compute Hamming distance with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn hamming_dispatched(a: &[f32], b: &[f32]) -> u32 {
    #[allow(clippy::cast_sign_loss)]
    // Reason: hamming_distance_native returns count of differing bits (non-negative),
    // and vector dimensions are bounded by u32::MAX, so result always fits in u32
    {
        crate::simd_native::hamming_distance_native(a, b) as u32
    }
}

/// Returns information about which SIMD features are available.
#[must_use]
pub fn simd_features_info() -> SimdFeatures {
    SimdFeatures::detect()
}

/// Information about available SIMD features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SimdFeatures {
    /// AVX-512 foundation instructions available.
    pub avx512f: bool,
    /// AVX-512 VPOPCNTDQ (population count) available.
    pub avx512_popcnt: bool,
    /// AVX2 instructions available.
    pub avx2: bool,
    /// POPCNT instruction available.
    pub popcnt: bool,
}

impl SimdFeatures {
    /// Detects available SIMD features on the current CPU.
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self {
                avx512f: is_x86_feature_detected!("avx512f"),
                avx512_popcnt: is_x86_feature_detected!("avx512vpopcntdq"),
                avx2: is_x86_feature_detected!("avx2"),
                popcnt: is_x86_feature_detected!("popcnt"),
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                avx512f: false,
                avx512_popcnt: false,
                avx2: false,
                popcnt: false,
            }
        }
    }

    /// Returns the best available instruction set name.
    #[must_use]
    pub const fn best_instruction_set(&self) -> &'static str {
        if self.avx512f {
            "AVX-512"
        } else if self.avx2 {
            "AVX2"
        } else {
            "Scalar"
        }
    }
}

// =============================================================================
// Prefetch constants - EPIC-C.1
// =============================================================================

// Scalar implementations for tests
#[cfg(test)]
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
fn euclidean_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum::<f32>()
        .sqrt()
}

#[cfg(test)]
fn cosine_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom > 0.0 {
        dot / denom
    } else {
        0.0
    }
}

#[cfg(test)]
fn hamming_scalar(a: &[f32], b: &[f32]) -> u32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    #[allow(clippy::cast_possible_truncation)]
    let count = a
        .iter()
        .zip(b.iter())
        .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
        .count() as u32;
    count
}

#[cfg(test)]
fn cosine_normalized_scalar(a: &[f32], b: &[f32]) -> f32 {
    // For normalized vectors, cosine = dot product
    dot_product_scalar(a, b)
}

/// Cache line size in bytes (standard for modern x86/ARM CPUs).
pub const CACHE_LINE_SIZE: usize = 64;

/// Prefetch distance for 768-dimensional vectors (3072 bytes).
/// Calculated at compile time: `768 * 4 / 64 = 48` cache lines.
pub const PREFETCH_DISTANCE_768D: usize = 768 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Prefetch distance for 384-dimensional vectors.
pub const PREFETCH_DISTANCE_384D: usize = 384 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Prefetch distance for 1536-dimensional vectors.
pub const PREFETCH_DISTANCE_1536D: usize = 1536 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Calculates prefetch distance for a given dimension at compile time.
#[inline]
#[must_use]
pub const fn prefetch_distance(dimension: usize) -> usize {
    (dimension * std::mem::size_of::<f32>()) / CACHE_LINE_SIZE
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[path = "simd_dispatch_unit_tests.rs"]
mod tests;
