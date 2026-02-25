use super::{dot::dot_product_native, simd_level, SimdLevel};

/// Cosine for pre-normalized vectors with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn cosine_normalized_native(a: &[f32], b: &[f32]) -> f32 {
    dot_product_native(a, b)
}

/// Cosine similarity with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    #[cfg(target_arch = "x86_64")]
    {
        match simd_level() {
            SimdLevel::Avx512 if a.len() >= 16 => {
                return unsafe { crate::simd_native::cosine_fused_avx512(a, b) }
            }
            SimdLevel::Avx2 if a.len() >= 1024 => {
                return unsafe { crate::simd_native::cosine_fused_avx2(a, b) }
            }
            SimdLevel::Avx2 if a.len() >= 64 => {
                return unsafe { crate::simd_native::cosine_fused_avx2_2acc(a, b) }
            }
            SimdLevel::Avx2 if a.len() >= 8 => {
                return unsafe { crate::simd_native::cosine_fused_avx2(a, b) }
            }
            _ => {}
        }
    }
    crate::simd_native::scalar::cosine_scalar(a, b)
}

pub(super) fn resolve_cosine(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 16 => {
            |a, b| unsafe { crate::simd_native::cosine_fused_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 1024 => {
            |a, b| unsafe { crate::simd_native::cosine_fused_avx2(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 64 => {
            |a, b| unsafe { crate::simd_native::cosine_fused_avx2_2acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 8 => {
            |a, b| unsafe { crate::simd_native::cosine_fused_avx2(a, b) }
        }
        _ => crate::simd_native::scalar::cosine_scalar,
    }
}
