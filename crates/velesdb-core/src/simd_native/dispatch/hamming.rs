use super::SimdLevel;

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

#[inline]
fn hamming_simd(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            // SAFETY: AVX-512 hamming kernel requires CPU feature + minimum dim.
            // - Condition 1: `is_x86_feature_detected!("avx512f")` is true.
            // Reason: call specialized kernel for higher throughput.
            return unsafe { crate::simd_native::hamming_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            // SAFETY: AVX2 hamming kernel requires CPU feature + minimum dim.
            // - Condition 1: `is_x86_feature_detected!("avx2")` is true.
            // Reason: call specialized kernel for higher throughput.
            return unsafe { crate::simd_native::hamming_avx2(a, b) };
        }
    }
    crate::simd_native::scalar::hamming_scalar(a, b)
}

#[inline]
fn jaccard_simd(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            // SAFETY: AVX-512 jaccard kernel requires CPU feature + minimum dim.
            // - Condition 1: `is_x86_feature_detected!("avx512f")` is true.
            // Reason: call specialized kernel for higher throughput.
            return unsafe { crate::simd_native::jaccard_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            // SAFETY: AVX2 jaccard kernel requires CPU feature + minimum dim.
            // - Condition 1: `is_x86_feature_detected!("avx2")` is true.
            // Reason: call specialized kernel for higher throughput.
            return unsafe { crate::simd_native::jaccard_avx2(a, b) };
        }
    }
    crate::simd_native::scalar::jaccard_scalar(a, b)
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
        SimdLevel::Avx2 if dim >= 8 => |a, b| {
            // SAFETY: Resolver emitted AVX2 implementation for this dimension.
            // - Condition 1: caller chose this function pointer via `resolve_hamming`.
            // Reason: execute AVX2 specialized hamming implementation.
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
        SimdLevel::Avx2 if dim >= 8 => |a, b| {
            // SAFETY: Resolver emitted AVX2 implementation for this dimension.
            // - Condition 1: caller chose this function pointer via `resolve_jaccard`.
            // Reason: execute AVX2 specialized jaccard implementation.
            unsafe { crate::simd_native::jaccard_avx2(a, b) }
        },
        _ => crate::simd_native::scalar::jaccard_scalar,
    }
}
