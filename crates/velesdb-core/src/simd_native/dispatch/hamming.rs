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
        SimdLevel::Avx512 if a.len() >= 512 => {
            // SAFETY: AVX-512 4-acc hamming kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: 4-accumulator kernel for large vectors.
            unsafe { crate::simd_native::hamming_avx512_4acc(a, b) }
        }
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
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => crate::simd_native::hamming_neon(a, b),
        _ => crate::simd_native::scalar::hamming_scalar(a, b),
    }
}

/// F-08: Use cached `simd_level()` for consistency with other metrics.
#[inline]
fn jaccard_simd(a: &[f32], b: &[f32]) -> f32 {
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 1024 => {
            // SAFETY: AVX-512 8-acc jaccard kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: 8-accumulator kernel for very large vectors (>= 1024 dims).
            unsafe { crate::simd_native::jaccard_avx512_8acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => {
            // SAFETY: AVX-512 4-acc jaccard kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: 4-accumulator kernel for large vectors.
            unsafe { crate::simd_native::jaccard_avx512_4acc(a, b) }
        }
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
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => crate::simd_native::jaccard_neon(a, b),
        _ => crate::simd_native::scalar::jaccard_scalar(a, b),
    }
}

#[allow(unused_variables)]
pub(super) fn resolve_hamming(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 512 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 4-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_hamming`.
                // Reason: execute AVX-512 4-accumulator hamming for large vectors.
                unsafe { crate::simd_native::hamming_avx512_4acc(a, b) }
            }
        }
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
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if dim >= 4 => |a, b| crate::simd_native::hamming_neon(a, b),
        _ => crate::simd_native::scalar::hamming_scalar,
    }
}

#[allow(unused_variables)]
pub(super) fn resolve_jaccard(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 1024 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 8-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_jaccard`.
                // Reason: execute AVX-512 8-accumulator jaccard for very large vectors.
                unsafe { crate::simd_native::jaccard_avx512_8acc(a, b) }
            }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 512 => {
            |a, b| {
                // SAFETY: Resolver emitted AVX-512 4-acc implementation for this dimension.
                // - Condition 1: caller chose this function pointer via `resolve_jaccard`.
                // Reason: execute AVX-512 4-accumulator jaccard for large vectors.
                unsafe { crate::simd_native::jaccard_avx512_4acc(a, b) }
            }
        }
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
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if dim >= 4 => |a, b| crate::simd_native::jaccard_neon(a, b),
        _ => crate::simd_native::scalar::jaccard_scalar,
    }
}

// =============================================================================
// Binary Hamming distance (packed u64)
// =============================================================================

/// Binary Hamming distance with runtime SIMD dispatch.
///
/// Operates on packed u64 binary vectors where each bit is a dimension.
/// XOR + popcount gives the number of differing bits.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[inline]
#[must_use]
#[allow(clippy::cast_possible_truncation)]
// Reason: return type is u32; max bits = len * 64 which fits in u32 for any
// practical binary vector dimension (up to ~67M words).
pub fn hamming_binary_native(a: &[u64], b: &[u64]) -> u32 {
    assert_eq!(
        a.len(),
        b.len(),
        "Binary vector length mismatch: {} vs {}",
        a.len(),
        b.len()
    );

    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 8 => {
            // SAFETY: AVX-512 binary hamming kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` selected `Avx512` after runtime detection.
            // Reason: AVX-512 XOR on 8 packed u64 per iteration for higher throughput.
            unsafe { crate::simd_native::hamming_binary_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 | SimdLevel::Avx2 if a.len() >= 4 => {
            // SAFETY: AVX2 binary hamming kernel requires CPU feature + minimum dim.
            // - Condition 1: `simd_level()` confirmed AVX2+ (Avx512 implies Avx2 support).
            // Reason: AVX2 XOR on 4 packed u64 per iteration for higher throughput.
            unsafe { crate::simd_native::hamming_binary_avx2(a, b) }
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 2 => {
            // NEON binary hamming uses vcntq_u8 for byte-popcount on 2 u64 per iteration.
            crate::simd_native::hamming_binary_neon(a, b)
        }
        _ => crate::simd_native::scalar::hamming_binary_scalar(a, b),
    }
}

// =============================================================================
// Batch operations with prefetch (Phase 4)
// =============================================================================

/// Batch Hamming distance with cross-platform multi-level prefetch hints.
///
/// Computes Hamming distance between each candidate and the query vector,
/// using software prefetching for better cache utilization on large batches.
#[inline]
#[must_use]
pub fn batch_hamming_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        super::dot::batch_prefetch_candidate(candidates, i);
        results.push(hamming_distance_native(candidate, query));
    }

    results
}

/// Batch Jaccard similarity with cross-platform multi-level prefetch hints.
///
/// Computes Jaccard similarity between each candidate and the query vector,
/// using software prefetching for better cache utilization on large batches.
#[inline]
#[must_use]
pub fn batch_jaccard_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        super::dot::batch_prefetch_candidate(candidates, i);
        results.push(jaccard_similarity_native(candidate, query));
    }

    results
}
