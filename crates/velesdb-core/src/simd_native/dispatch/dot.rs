use super::{simd_level, SimdLevel};

/// Dot product with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => {
            // SAFETY: AVX-512 dot kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::dot_product_avx512_4acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => {
            // SAFETY: AVX-512 dot kernel requires CPU feature.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::dot_product_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 256 => {
            // SAFETY: AVX2 dot kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::dot_product_avx2_4acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => {
            // SAFETY: AVX2 dot kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::dot_product_avx2(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => {
            // SAFETY: AVX2 dot kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::dot_product_avx2_1acc(a, b) }
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => crate::simd_native::dot_product_neon(a, b),
        _ => super::dot_product_scalar(a, b),
    }
}

/// Batch dot product with prefetch hints on x86_64.
#[inline]
#[must_use]
pub fn batch_dot_product_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());
    for (i, candidate) in candidates.iter().enumerate() {
        #[cfg(target_arch = "x86_64")]
        if i + 4 < candidates.len() {
            // SAFETY: `_mm_prefetch` is a non-faulting cache hint.
            // - Condition 1: pointer originates from a valid slice in `candidates`.
            // Reason: warm cache lines for upcoming dot-product reads.
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                _mm_prefetch(candidates[i + 4].as_ptr().cast::<i8>(), _MM_HINT_T0);
            }
        }
        results.push(dot_product_native(candidate, query));
    }
    results
}

pub(super) fn resolve_dot_product(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 512 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_dot_product`.
                // Reason: execute AVX-512 specialized dot-product implementation.
                unsafe { crate::simd_native::dot_product_avx512_4acc(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => |a, b| {
            // SAFETY: Resolver emitted AVX-512 implementation for this dimension.
            // - Condition 1: caller chose this function pointer via `resolve_dot_product`.
            // Reason: execute AVX-512 specialized dot-product implementation.
            unsafe { crate::simd_native::dot_product_avx512(a, b) }
        },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 256 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX2 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_dot_product`.
                // Reason: execute AVX2 specialized dot-product implementation.
                unsafe { crate::simd_native::dot_product_avx2_4acc(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 64 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX2 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_dot_product`.
                // Reason: execute AVX2 specialized dot-product implementation.
                unsafe { crate::simd_native::dot_product_avx2(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 8 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX2 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_dot_product`.
                // Reason: execute AVX2 specialized dot-product implementation.
                unsafe { crate::simd_native::dot_product_avx2_1acc(a, b) }
            }
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if dim >= 4 => |a, b| crate::simd_native::dot_product_neon(a, b),
        _ => super::dot_product_scalar,
    }
}
