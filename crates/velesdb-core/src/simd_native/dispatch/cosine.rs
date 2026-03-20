use super::{dot::dot_product_native, simd_level, SimdLevel};

/// Cosine for pre-normalized vectors with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn cosine_normalized_native(a: &[f32], b: &[f32]) -> f32 {
    dot_product_native(a, b)
}

/// Cosine similarity with runtime SIMD dispatch.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    #[cfg(target_arch = "x86_64")]
    {
        match simd_level() {
            SimdLevel::Avx512 if a.len() >= 1024 => {
                // SAFETY: AVX-512 8-acc cosine kernel requires CPU feature + minimum dim.
                // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
                // Reason: 8-accumulator variant for very large dimensions (stride 128).
                return unsafe { crate::simd_native::cosine_fused_avx512_8acc(a, b) };
            }
            SimdLevel::Avx512 if a.len() >= 512 => {
                // SAFETY: AVX-512 4-acc cosine kernel requires CPU feature + minimum dim.
                // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
                // Reason: 4-accumulator variant for large dimensions (stride 64).
                return unsafe { crate::simd_native::cosine_fused_avx512_4acc(a, b) };
            }
            SimdLevel::Avx512 if a.len() >= 16 => {
                // SAFETY: AVX-512 2-acc cosine kernel requires CPU feature + minimum dim.
                // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
                // Reason: 2-accumulator variant for medium dimensions (stride 32).
                return unsafe { crate::simd_native::cosine_fused_avx512(a, b) };
            }
            SimdLevel::Avx2 if a.len() >= 512 => {
                // SAFETY: AVX2 4-acc cosine kernel requires CPU feature + minimum dim.
                // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
                // Reason: 4-accumulator variant for large dimensions (stride 32).
                return unsafe { crate::simd_native::cosine_fused_avx2(a, b) };
            }
            SimdLevel::Avx2 if a.len() >= 8 => {
                // SAFETY: AVX2 2-acc cosine kernel requires CPU feature + minimum dim.
                // - Condition 1: `simd_level()` selected `Avx2` after runtime detection.
                // Reason: 2-accumulator variant for small-to-medium dimensions (stride 16).
                return unsafe { crate::simd_native::cosine_fused_avx2_2acc(a, b) };
            }
            _ => {}
        }
    }
    // NEON cosine dispatch for aarch64 (EPIC-054 US-004).
    #[cfg(target_arch = "aarch64")]
    if a.len() >= 4 {
        // SAFETY: `cosine_neon` is a pure-Rust NEON function; no CPU feature detection
        // needed because NEON is always present on aarch64.
        // Reason: dispatch to the optimized NEON fused cosine kernel.
        return crate::simd_native::cosine_neon(a, b);
    }
    crate::simd_native::scalar::cosine_scalar(a, b)
}

/// Batch cosine similarity with cross-platform multi-level prefetch hints.
#[inline]
#[must_use]
pub fn batch_cosine_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        super::dot::batch_prefetch_candidate(candidates, i);
        results.push(cosine_similarity_native(candidate, query));
    }

    results
}

pub(super) fn resolve_cosine(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 1024 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 8-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_cosine`.
                // Reason: execute AVX-512 8-accumulator cosine for very large dimensions.
                unsafe { crate::simd_native::cosine_fused_avx512_8acc(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 512 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 4-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_cosine`.
                // Reason: execute AVX-512 4-accumulator cosine for large dimensions.
                unsafe { crate::simd_native::cosine_fused_avx512_4acc(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 16 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 2-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_cosine`.
                // Reason: execute AVX-512 2-accumulator cosine for medium dimensions.
                unsafe { crate::simd_native::cosine_fused_avx512(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 512 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX2 4-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_cosine`.
                // Reason: execute AVX2 4-accumulator cosine for large dimensions.
                unsafe { crate::simd_native::cosine_fused_avx2(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 8 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX2 2-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_cosine`.
                // Reason: execute AVX2 2-accumulator cosine for small-to-medium dims.
                unsafe { crate::simd_native::cosine_fused_avx2_2acc(a, b) }
            }
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if dim >= 4 => |a, b| crate::simd_native::cosine_neon(a, b),
        _ => crate::simd_native::scalar::cosine_scalar,
    }
}
