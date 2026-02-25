use super::SimdLevel;

/// Hamming distance with runtime SIMD dispatch.
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
            return unsafe { crate::simd_native::hamming_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
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
            return unsafe { crate::simd_native::jaccard_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            return unsafe { crate::simd_native::jaccard_avx2(a, b) };
        }
    }
    crate::simd_native::scalar::jaccard_scalar(a, b)
}

pub(super) fn resolve_hamming(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 16 => {
            |a, b| unsafe { crate::simd_native::hamming_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 8 => |a, b| unsafe { crate::simd_native::hamming_avx2(a, b) },
        _ => crate::simd_native::scalar::hamming_scalar,
    }
}

pub(super) fn resolve_jaccard(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
    match level {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if dim >= 16 => {
            |a, b| unsafe { crate::simd_native::jaccard_avx512(a, b) }
        }
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if dim >= 8 => |a, b| unsafe { crate::simd_native::jaccard_avx2(a, b) },
        _ => crate::simd_native::scalar::jaccard_scalar,
    }
}
