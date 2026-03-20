use super::{dot::dot_product_native, simd_level, SimdLevel};

/// Squared L2 distance with runtime SIMD dispatch.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn squared_l2_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 1024 => {
            // SAFETY: AVX-512 8-acc squared-L2 kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: 8-accumulator variant for very large dimensions (stride 128).
            unsafe { crate::simd_native::squared_l2_avx512_8acc(a, b) }
        }
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
///
/// F-07: The scaling phase now uses SIMD (AVX2/AVX-512) instead of a scalar loop.
/// For 768D vectors, this is ~4-8x faster on the scaling phase.
#[allow(clippy::inline_always)]
#[inline(always)]
pub fn normalize_inplace_native(v: &mut [f32]) {
    let n = norm_native(v);
    if n > 0.0 {
        let inv_norm = 1.0 / n;
        scale_inplace_native(v, inv_norm);
    }
}

/// Scales all elements of a mutable slice by a constant factor using SIMD.
///
/// F-07: Replaces scalar `for x in v { *x *= factor }` with SIMD broadcast+mul.
/// Note: AVX-512 store intrinsics require Rust 1.89+ (MSRV is 1.83),
/// so AVX-512 and AVX2 both use the AVX2 kernel (sufficient for scale).
#[inline]
fn scale_inplace_native(v: &mut [f32], factor: f32) {
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 | SimdLevel::Avx2 if v.len() >= 8 => {
            // SAFETY: AVX2 scale kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` confirmed AVX2+ after runtime detection.
            // Reason: broadcast factor and multiply 8 floats per iteration.
            unsafe { scale_inplace_avx2(v, factor) };
        }
        _ => {
            for x in v.iter_mut() {
                *x *= factor;
            }
        }
    }
}

/// AVX2 in-place scale: `v[i] *= factor` for all elements.
///
/// # Safety
///
/// Caller must ensure CPU supports AVX2 (enforced by runtime detection).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn scale_inplace_avx2(v: &mut [f32], factor: f32) {
    use std::arch::x86_64::{_mm256_loadu_ps, _mm256_mul_ps, _mm256_set1_ps, _mm256_storeu_ps};

    let len = v.len();
    let simd_len = len / 8;
    let ptr = v.as_mut_ptr();
    let scale = _mm256_set1_ps(factor);

    for i in 0..simd_len {
        let offset = i * 8;
        // SAFETY: offset + 8 <= simd_len * 8 <= len, so within bounds.
        // `_mm256_loadu_ps` / `_mm256_storeu_ps` handle unaligned access.
        let val = _mm256_loadu_ps(ptr.add(offset));
        let scaled = _mm256_mul_ps(val, scale);
        _mm256_storeu_ps(ptr.add(offset), scaled);
    }

    // Scalar remainder (0-7 elements)
    let base = simd_len * 8;
    for i in base..len {
        // SAFETY: `i` is in range `base..len` where `base = simd_len * 8 <= len`,
        // so `i < len` is guaranteed. Bounds check would be elided by LLVM anyway.
        *v.get_unchecked_mut(i) *= factor;
    }
}

// Note: AVX-512 scale_inplace not implemented — _mm512_storeu_ps requires
// Rust 1.89+ but MSRV is 1.83. AVX2 kernel handles AVX-512 CPUs (subset).

/// Batch squared L2 distance with cross-platform multi-level prefetch hints.
#[inline]
#[must_use]
pub fn batch_squared_l2_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        super::dot::batch_prefetch_candidate(candidates, i);
        results.push(squared_l2_native(candidate, query));
    }

    results
}

/// Batch euclidean distance with cross-platform multi-level prefetch hints.
#[inline]
#[must_use]
pub fn batch_euclidean_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        super::dot::batch_prefetch_candidate(candidates, i);
        results.push(euclidean_native(candidate, query));
    }

    results
}

pub(super) fn resolve_squared_l2(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 1024 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 8-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_squared_l2`.
                // Reason: execute AVX-512 8-accumulator squared-L2 for very large dims.
                unsafe { crate::simd_native::squared_l2_avx512_8acc(a, b) }
            }
        }
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
