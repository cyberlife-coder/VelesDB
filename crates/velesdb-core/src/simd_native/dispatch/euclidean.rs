use super::{dot::dot_product_native, simd_level, SimdLevel};

/// Squared L2 distance with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn squared_l2_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => {
            // SAFETY: AVX-512 squared-L2 kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::squared_l2_avx512_4acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => {
            // SAFETY: AVX-512 squared-L2 kernel requires CPU feature.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::squared_l2_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 256 => {
            // SAFETY: AVX2 squared-L2 kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::squared_l2_avx2_4acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => {
            // SAFETY: AVX2 squared-L2 kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::squared_l2_avx2(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => {
            // SAFETY: AVX2 squared-L2 kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
            // Reason: call specialized kernel for higher throughput.
            unsafe { crate::simd_native::squared_l2_avx2_1acc(a, b) }
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => crate::simd_native::squared_l2_neon(a, b),
        _ => super::squared_l2_scalar(a, b),
    }
}

/// Euclidean distance with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn euclidean_native(a: &[f32], b: &[f32]) -> f32 {
    squared_l2_native(a, b).sqrt()
}

/// L2 norm with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn norm_native(v: &[f32]) -> f32 {
    dot_product_native(v, v).sqrt()
}

/// In-place normalization with runtime SIMD dispatch.
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

pub(super) fn resolve_squared_l2(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 512 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_squared_l2`.
                // Reason: execute AVX-512 specialized squared-L2 implementation.
                unsafe { crate::simd_native::squared_l2_avx512_4acc(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => |a, b| {
            // SAFETY: Resolver emitted AVX-512 implementation for this dimension.
            // - Condition 1: caller chose this function pointer via `resolve_squared_l2`.
            // Reason: execute AVX-512 specialized squared-L2 implementation.
            unsafe { crate::simd_native::squared_l2_avx512(a, b) }
        },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 256 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX2 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_squared_l2`.
                // Reason: execute AVX2 specialized squared-L2 implementation.
                unsafe { crate::simd_native::squared_l2_avx2_4acc(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 64 => |a, b| {
            // SAFETY: Resolver emitted AVX2 implementation for this dimension.
            // - Condition 1: caller chose this function pointer via `resolve_squared_l2`.
            // Reason: execute AVX2 specialized squared-L2 implementation.
            unsafe { crate::simd_native::squared_l2_avx2(a, b) }
        },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 8 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX2 implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_squared_l2`.
                // Reason: execute AVX2 specialized squared-L2 implementation.
                unsafe { crate::simd_native::squared_l2_avx2_1acc(a, b) }
            }
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if dim >= 4 => |a, b| crate::simd_native::squared_l2_neon(a, b),
        _ => super::squared_l2_scalar,
    }
}
