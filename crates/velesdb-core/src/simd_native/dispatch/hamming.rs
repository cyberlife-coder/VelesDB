use super::{simd_level, SimdLevel};

/// Hamming distance with runtime SIMD dispatch.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
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

/// Jaccard similarity with runtime SIMD dispatch.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
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

/// F-08: Use cached `simd_level()` (OnceLock) instead of per-call `is_x86_feature_detected!`
/// for consistency with dot/cosine/euclidean dispatch paths.
#[inline]
fn hamming_simd(a: &[f32], b: &[f32]) -> f32 {
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 16 => {
            // SAFETY: AVX-512 hamming kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::hamming_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 | SimdLevel::Avx2 if a.len() >= 8 => {
            // SAFETY: AVX2 hamming kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` confirmed AVX2+ (Avx512 implies Avx2 support).
            // Reason: fallthrough for Avx512 with short vectors that don't meet 16-element minimum.
            unsafe { crate::simd_native::hamming_avx2(a, b) }
        }
        _ => crate::simd_native::scalar::hamming_scalar(a, b),
    }
}

/// F-08: Use cached `simd_level()` for consistency with other metrics.
#[inline]
fn jaccard_simd(a: &[f32], b: &[f32]) -> f32 {
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 16 => {
            // SAFETY: AVX-512 jaccard kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::jaccard_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 | SimdLevel::Avx2 if a.len() >= 8 => {
            // SAFETY: AVX2 jaccard kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` confirmed AVX2+ (Avx512 implies Avx2 support).
            // Reason: fallthrough for Avx512 with short vectors that don't meet 16-element minimum.
            unsafe { crate::simd_native::jaccard_avx2(a, b) }
        }
        _ => crate::simd_native::scalar::jaccard_scalar(a, b),
    }
}

#[allow(unused_variables)]
pub(super) fn resolve_hamming(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 16 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_hamming`.
                // Reason: execute AVX-512 specialized hamming implementation.
                unsafe { crate::simd_native::hamming_avx512(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 | SimdLevel::Avx2 if dim >= 8 => |a, b| {
            // SAFETY: Resolver emitted AVX2 implementation for this dimension.
            // - Condition 1: caller chose this function pointer via `resolve_hamming`.
            // Reason: execute AVX2 specialized hamming implementation (Avx512 implies Avx2).
            unsafe { crate::simd_native::hamming_avx2(a, b) }
        },
        _ => crate::simd_native::scalar::hamming_scalar,
    }
}

#[allow(unused_variables)]
pub(super) fn resolve_jaccard(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 16 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_jaccard`.
                // Reason: execute AVX-512 specialized jaccard implementation.
                unsafe { crate::simd_native::jaccard_avx512(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 | SimdLevel::Avx2 if dim >= 8 => |a, b| {
            // SAFETY: Resolver emitted AVX2 implementation for this dimension.
            // - Condition 1: caller chose this function pointer via `resolve_jaccard`.
            // Reason: execute AVX2 specialized jaccard implementation (Avx512 implies Avx2).
            unsafe { crate::simd_native::jaccard_avx2(a, b) }
        },
        _ => crate::simd_native::scalar::jaccard_scalar,
    }
}
