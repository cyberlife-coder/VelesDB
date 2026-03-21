use super::{simd_level, SimdLevel};

/// Dot product with runtime SIMD dispatch.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 1024 => {
            // SAFETY: AVX-512 8-acc dot kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: 8-accumulator variant for very large dimensions (stride 128).
            unsafe { crate::simd_native::dot_product_avx512_8acc(a, b) }
        }
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

/// Batch dot product with cross-platform multi-level prefetch hints.
///
/// Prefetches multiple cache lines per vector for better coverage on
/// high-dimensional vectors (e.g., 768d = 3072 bytes = 48 cache lines).
#[inline]
#[must_use]
pub fn batch_dot_product_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    super::batch_with_prefetch(candidates, query, dot_product_native)
}

/// Cross-platform multi-level prefetch for batch distance computations.
///
/// Prefetches the i+4 vector (multi-cache-line) and the i+8 vector (single line)
/// using the platform-agnostic prefetch utilities (x86_64 + aarch64 + no-op).
#[inline]
pub(super) fn batch_prefetch_candidate(candidates: &[&[f32]], i: usize) {
    if i + 4 < candidates.len() {
        crate::simd_native::prefetch_vector_multi_cache_line(candidates[i + 4]);
    }
    if i + 8 < candidates.len() {
        crate::simd_native::prefetch_vector(candidates[i + 8]);
    }
}

pub(super) fn resolve_dot_product(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 1024 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 8-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_dot_product`.
                // Reason: execute AVX-512 8-accumulator dot-product for very large dims.
                unsafe { crate::simd_native::dot_product_avx512_8acc(a, b) }
            }
        }
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
