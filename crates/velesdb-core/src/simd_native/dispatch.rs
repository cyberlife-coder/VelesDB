//! Runtime SIMD level detection and dispatch wiring.
//!
//! This module provides:
//! - `SimdLevel` enum for representing detected SIMD capability
//! - `simd_level()` for cached runtime detection
//! - `warmup_simd_cache()` for eliminating cold-start latency
//! - All public dispatch functions that route to ISA-specific kernels

use super::scalar;

// =============================================================================
// Cached SIMD Level Detection (EPIC-033 US-002)
// =============================================================================

/// SIMD capability level detected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    /// AVX-512F available (x86_64 only).
    Avx512,
    /// AVX2 + FMA available (x86_64 only).
    Avx2,
    /// NEON available (aarch64, always true).
    Neon,
    /// Scalar fallback.
    Scalar,
}

/// Cached SIMD level - detected once at first use.
static SIMD_LEVEL: std::sync::OnceLock<SimdLevel> = std::sync::OnceLock::new();

/// Detects the best available SIMD level for the current CPU.
fn detect_simd_level() -> SimdLevel {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return SimdLevel::Avx512;
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return SimdLevel::Avx2;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        return SimdLevel::Neon;
    }

    #[allow(unreachable_code)]
    SimdLevel::Scalar
}

/// Returns the cached SIMD capability level.
#[inline]
#[must_use]
pub fn simd_level() -> SimdLevel {
    *SIMD_LEVEL.get_or_init(detect_simd_level)
}

/// Warms up SIMD caches to eliminate cold-start latency.
///
/// Call this at application startup to ensure the first SIMD operations
/// are as fast as subsequent ones.
///
/// # Example
///
/// ```
/// use velesdb_core::simd_native::warmup_simd_cache;
/// warmup_simd_cache();
/// ```
#[inline]
pub fn warmup_simd_cache() {
    let _ = simd_level();
    let warmup_size = 768;
    let a: Vec<f32> = vec![0.01; warmup_size];
    let b: Vec<f32> = vec![0.01; warmup_size];
    for _ in 0..3 {
        let _ = dot_product_native(&a, &b);
        let _ = cosine_similarity_native(&a, &b);
    }
}

// =============================================================================
// Public API with cached dispatch
// =============================================================================

/// Dot product with automatic dispatch to best available SIMD.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => unsafe { super::dot_product_avx512_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 16 => unsafe { super::dot_product_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => unsafe { super::dot_product_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 256 => unsafe { super::dot_product_avx2_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => unsafe { super::dot_product_avx2(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 16 => unsafe { super::dot_product_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => unsafe { super::dot_product_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 => a.iter().zip(b.iter()).map(|(x, y)| x * y).sum(),
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => super::dot_product_neon(a, b),
        _ => a.iter().zip(b.iter()).map(|(x, y)| x * y).sum(),
    }
}

/// Squared L2 distance with automatic dispatch to best available SIMD.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn squared_l2_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => unsafe { super::squared_l2_avx512_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 16 => unsafe { super::squared_l2_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => unsafe { super::squared_l2_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 256 => unsafe { super::squared_l2_avx2_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => unsafe { super::squared_l2_avx2(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 16 => unsafe { super::squared_l2_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => unsafe { super::squared_l2_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 => a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum(),
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => super::squared_l2_neon(a, b),
        _ => a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum(),
    }
}

/// Euclidean distance with automatic dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn euclidean_native(a: &[f32], b: &[f32]) -> f32 {
    squared_l2_native(a, b).sqrt()
}

/// L2 norm with automatic dispatch to best available SIMD.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn norm_native(v: &[f32]) -> f32 {
    dot_product_native(v, v).sqrt()
}

/// Normalizes a vector in-place using native SIMD.
#[allow(clippy::inline_always)]
#[inline(always)]
pub fn normalize_inplace_native(v: &mut [f32]) {
    let n = norm_native(v);
    if n > 0.0 {
        let inv_norm = 1.0 / n;
        for x in v.iter_mut() {
            *x *= inv_norm;
        }
    }
}

/// Cosine similarity for pre-normalized vectors with automatic dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn cosine_normalized_native(a: &[f32], b: &[f32]) -> f32 {
    dot_product_native(a, b)
}

/// Full cosine similarity (with normalization) using native SIMD.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    #[cfg(target_arch = "x86_64")]
    {
        match simd_level() {
            SimdLevel::Avx512 if a.len() >= 16 => {
                return unsafe { super::cosine_fused_avx512(a, b) }
            }
            SimdLevel::Avx2 if a.len() >= 1024 => return unsafe { super::cosine_fused_avx2(a, b) },
            SimdLevel::Avx2 if a.len() >= 64 => {
                return unsafe { super::cosine_fused_avx2_2acc(a, b) }
            }
            SimdLevel::Avx2 if a.len() >= 8 => return unsafe { super::cosine_fused_avx2(a, b) },
            _ => {}
        }
    }
    scalar::cosine_scalar(a, b)
}

/// Batch dot products with prefetching.
#[inline]
#[must_use]
pub fn batch_dot_product_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());
    for (i, candidate) in candidates.iter().enumerate() {
        #[cfg(target_arch = "x86_64")]
        if i + 4 < candidates.len() {
            // SAFETY: _mm_prefetch is a hint that cannot fault.
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                _mm_prefetch(candidates[i + 4].as_ptr().cast::<i8>(), _MM_HINT_T0);
            }
        }
        results.push(dot_product_native(candidate, query));
    }
    results
}

/// Hamming distance between two vectors using SIMD.
#[inline]
#[must_use]
pub fn hamming_distance_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(
        a.len(),
        b.len(),
        "Vector length mismatch: {} vs {}",
        a.len(),
        b.len()
    );
    hamming_simd(a, b)
}

/// Jaccard similarity between two vectors using SIMD.
#[inline]
#[must_use]
pub fn jaccard_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(
        a.len(),
        b.len(),
        "Vector length mismatch: {} vs {}",
        a.len(),
        b.len()
    );
    jaccard_simd(a, b)
}

#[inline]
fn hamming_simd(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            return unsafe { super::hamming_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            return unsafe { super::hamming_avx2(a, b) };
        }
    }
    scalar::hamming_scalar(a, b)
}

#[inline]
fn jaccard_simd(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            return unsafe { super::jaccard_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            return unsafe { super::jaccard_avx2(a, b) };
        }
    }
    scalar::jaccard_scalar(a, b)
}
