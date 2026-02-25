use super::{simd_level, SimdLevel};

/// Dot product with runtime SIMD dispatch.
#[allow(clippy::inline_always)]
#[inline(always)]
#[must_use]
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => unsafe {
            crate::simd_native::dot_product_avx512_4acc(a, b)
        },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => unsafe { crate::simd_native::dot_product_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 256 => unsafe {
            crate::simd_native::dot_product_avx2_4acc(a, b)
        },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => unsafe { crate::simd_native::dot_product_avx2(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => unsafe {
            crate::simd_native::dot_product_avx2_1acc(a, b)
        },
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
            |a, b| unsafe { crate::simd_native::dot_product_avx512_4acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => |a, b| unsafe { crate::simd_native::dot_product_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 256 => {
            |a, b| unsafe { crate::simd_native::dot_product_avx2_4acc(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 64 => {
            |a, b| unsafe { crate::simd_native::dot_product_avx2(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 8 => {
            |a, b| unsafe { crate::simd_native::dot_product_avx2_1acc(a, b) }
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if dim >= 4 => |a, b| crate::simd_native::dot_product_neon(a, b),
        _ => super::dot_product_scalar,
    }
}
