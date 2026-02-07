//! AVX2+FMA kernel implementations for x86_64.
//!
//! Contains hand-tuned AVX2 SIMD kernels for dot product and squared L2 distance
//! with 1-acc, 2-acc, and 4-acc variants for different vector sizes.
//!
//! All functions require runtime AVX2+FMA detection before calling.

// SAFETY: Numeric casts in this file are intentional and safe:
// - All casts are from well-bounded values (vector dimensions, loop indices)
// - All casts are validated by extensive SIMD tests (simd_native_tests.rs)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::incompatible_msrv)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::similar_names)]

use crate::sum_remainder_unrolled_8;
use crate::sum_squared_remainder_unrolled_8;

// =============================================================================
// Dot Product
// =============================================================================

/// AVX2 dot product with 4 accumulators for ILP on large vectors.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
/// - `a.len() >= 128` for optimal performance (amortizes accumulator combining cost)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
#[allow(clippy::too_many_lines)] // Remainder unrolling adds lines for performance
pub(crate) unsafe fn dot_product_avx2_4acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX2+FMA.
    // - `_mm256_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic stays within bounds: offset = i * 32 where i < simd_len = len / 32
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 32; // Process 32 per iteration (4×8)

    let mut sum0 = _mm256_setzero_ps();
    let mut sum1 = _mm256_setzero_ps();
    let mut sum2 = _mm256_setzero_ps();
    let mut sum3 = _mm256_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 32;

        let va0 = _mm256_loadu_ps(a_ptr.add(offset));
        let vb0 = _mm256_loadu_ps(b_ptr.add(offset));
        sum0 = _mm256_fmadd_ps(va0, vb0, sum0);

        let va1 = _mm256_loadu_ps(a_ptr.add(offset + 8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(offset + 8));
        sum1 = _mm256_fmadd_ps(va1, vb1, sum1);

        let va2 = _mm256_loadu_ps(a_ptr.add(offset + 16));
        let vb2 = _mm256_loadu_ps(b_ptr.add(offset + 16));
        sum2 = _mm256_fmadd_ps(va2, vb2, sum2);

        let va3 = _mm256_loadu_ps(a_ptr.add(offset + 24));
        let vb3 = _mm256_loadu_ps(b_ptr.add(offset + 24));
        sum3 = _mm256_fmadd_ps(va3, vb3, sum3);
    }

    // Combine 4 accumulators into 1
    let sum01 = _mm256_add_ps(sum0, sum1);
    let sum23 = _mm256_add_ps(sum2, sum3);
    let combined = _mm256_add_ps(sum01, sum23);

    // Horizontal sum
    let hi = _mm256_extractf128_ps(combined, 1);
    let lo = _mm256_castps256_ps128(combined);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    // Handle remainder (max 31 elements) with unrolled tail
    let base = simd_len * 32;
    let remainder = len - base;

    if remainder >= 16 {
        // Process 16 more elements with 2-acc SIMD
        let offset = base;
        let va0 = _mm256_loadu_ps(a_ptr.add(offset));
        let vb0 = _mm256_loadu_ps(b_ptr.add(offset));
        let mut sum0 = _mm256_fmadd_ps(va0, vb0, _mm256_setzero_ps());

        let va1 = _mm256_loadu_ps(a_ptr.add(offset + 8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(offset + 8));
        let sum1 = _mm256_fmadd_ps(va1, vb1, _mm256_setzero_ps());

        sum0 = _mm256_add_ps(sum0, sum1);
        let hi = _mm256_extractf128_ps(sum0, 1);
        let lo = _mm256_castps256_ps128(sum0);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        result += _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

        // Handle remaining 0-15 elements
        if remainder > 16 {
            let rbase = base + 16;
            let r = remainder - 16;
            if r >= 8 {
                let va = _mm256_loadu_ps(a_ptr.add(rbase));
                let vb = _mm256_loadu_ps(b_ptr.add(rbase));
                let tmp = _mm256_fmadd_ps(va, vb, _mm256_setzero_ps());
                let hi = _mm256_extractf128_ps(tmp, 1);
                let lo = _mm256_castps256_ps128(tmp);
                let sum128 = _mm_add_ps(lo, hi);
                let shuf = _mm_movehdup_ps(sum128);
                let sums = _mm_add_ps(sum128, shuf);
                let shuf2 = _mm_movehl_ps(sums, sums);
                result += _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

                if r > 8 {
                    let rrbase = rbase + 8;
                    let rr = r - 8;
                    sum_remainder_unrolled_8!(a, b, rrbase, rr, result);
                }
            } else {
                sum_remainder_unrolled_8!(a, b, rbase, r, result);
            }
        }
    } else if remainder >= 8 {
        let va = _mm256_loadu_ps(a_ptr.add(base));
        let vb = _mm256_loadu_ps(b_ptr.add(base));
        let tmp = _mm256_fmadd_ps(va, vb, _mm256_setzero_ps());
        let hi = _mm256_extractf128_ps(tmp, 1);
        let lo = _mm256_castps256_ps128(tmp);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        result += _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

        let r = remainder - 8;
        if r > 0 {
            let rbase = base + 8;
            sum_remainder_unrolled_8!(a, b, rbase, r, result);
        }
    } else if remainder > 0 {
        sum_remainder_unrolled_8!(a, b, base, remainder, result);
    }

    result
}

/// AVX2 dot product with single accumulator for small vectors.
///
/// Optimized for vectors 16-63 elements where 2-acc overhead isn't worth it.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
/// - Vector length >= 8 (use scalar for < 8)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn dot_product_avx2_1acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX2+FMA.
    // - `_mm256_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic stays within bounds
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 8; // Process 8 per iteration

    let mut sum = _mm256_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 8;
        let va = _mm256_loadu_ps(a_ptr.add(offset));
        let vb = _mm256_loadu_ps(b_ptr.add(offset));
        sum = _mm256_fmadd_ps(va, vb, sum);
    }

    // Horizontal sum: [a0,a1,a2,a3,a4,a5,a6,a7] -> scalar
    let hi = _mm256_extractf128_ps(sum, 1);
    let lo = _mm256_castps256_ps128(sum);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    // Handle remainder (max 7 elements) with unrolled tail
    let base = simd_len * 8;
    let remainder = len - base;

    // Unrolled tail loop for better performance
    sum_remainder_unrolled_8!(a, b, base, remainder, result);

    result
}

/// AVX2 dot product with 2 accumulators for ILP.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX2+FMA.
    // - `_mm256_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic stays within bounds: offset = i * 16 where i < simd_len = len / 16
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16; // Process 16 per iteration (2×8)

    let mut sum0 = _mm256_setzero_ps();
    let mut sum1 = _mm256_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va0 = _mm256_loadu_ps(a_ptr.add(offset));
        let vb0 = _mm256_loadu_ps(b_ptr.add(offset));
        sum0 = _mm256_fmadd_ps(va0, vb0, sum0);

        let va1 = _mm256_loadu_ps(a_ptr.add(offset + 8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(offset + 8));
        sum1 = _mm256_fmadd_ps(va1, vb1, sum1);
    }

    // Combine accumulators
    let combined = _mm256_add_ps(sum0, sum1);

    // Horizontal sum: [a0,a1,a2,a3,a4,a5,a6,a7] -> scalar
    let hi = _mm256_extractf128_ps(combined, 1);
    let lo = _mm256_castps256_ps128(combined);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    // Handle remainder (max 15 elements) with unrolled tail
    let base = simd_len * 16;
    let remainder = len - base;

    if remainder >= 8 {
        // Process 8 more elements with SIMD
        let va = _mm256_loadu_ps(a_ptr.add(base));
        let vb = _mm256_loadu_ps(b_ptr.add(base));
        let tmp_sum = _mm256_fmadd_ps(va, vb, _mm256_setzero_ps());
        let hi = _mm256_extractf128_ps(tmp_sum, 1);
        let lo = _mm256_castps256_ps128(tmp_sum);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        result += _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

        // Handle remaining 0-7 elements
        if remainder > 8 {
            let rbase = base + 8;
            let r = remainder - 8;
            sum_remainder_unrolled_8!(a, b, rbase, r, result);
        }
    } else if remainder > 0 {
        sum_remainder_unrolled_8!(a, b, base, remainder, result);
    }

    result
}

// =============================================================================
// Squared L2 Distance
// =============================================================================

/// AVX2 squared L2 distance.
///
/// # Safety
///
/// Same requirements as `dot_product_avx2`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
#[allow(clippy::too_many_lines)] // Remainder unrolling adds lines for performance
pub(crate) unsafe fn squared_l2_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: See dot_product_avx2 for detailed safety justification.
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16;

    let mut sum0 = _mm256_setzero_ps();
    let mut sum1 = _mm256_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va0 = _mm256_loadu_ps(a_ptr.add(offset));
        let vb0 = _mm256_loadu_ps(b_ptr.add(offset));
        let diff0 = _mm256_sub_ps(va0, vb0);
        sum0 = _mm256_fmadd_ps(diff0, diff0, sum0);

        let va1 = _mm256_loadu_ps(a_ptr.add(offset + 8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(offset + 8));
        let diff1 = _mm256_sub_ps(va1, vb1);
        sum1 = _mm256_fmadd_ps(diff1, diff1, sum1);
    }

    let combined = _mm256_add_ps(sum0, sum1);
    let hi = _mm256_extractf128_ps(combined, 1);
    let lo = _mm256_castps256_ps128(combined);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    let base = simd_len * 16;
    let remainder = len - base;

    if remainder >= 8 {
        // Process 8 more elements with SIMD
        let va = _mm256_loadu_ps(a_ptr.add(base));
        let vb = _mm256_loadu_ps(b_ptr.add(base));
        let diff = _mm256_sub_ps(va, vb);
        let tmp_sum = _mm256_fmadd_ps(diff, diff, _mm256_setzero_ps());
        let hi = _mm256_extractf128_ps(tmp_sum, 1);
        let lo = _mm256_castps256_ps128(tmp_sum);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        result += _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

        // Handle remaining 0-7 elements
        if remainder > 8 {
            let rbase = base + 8;
            let r = remainder - 8;
            sum_squared_remainder_unrolled_8!(a, b, rbase, r, result);
        }
    } else if remainder > 0 {
        sum_squared_remainder_unrolled_8!(a, b, base, remainder, result);
    }

    result
}

/// AVX2 squared L2 with single accumulator for small vectors.
///
/// Optimized for vectors 16-63 elements where 2-acc overhead isn't worth it.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
/// - Vector length >= 8 (use scalar for < 8)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn squared_l2_avx2_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 8;

    let mut sum = _mm256_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 8;
        let va = _mm256_loadu_ps(a_ptr.add(offset));
        let vb = _mm256_loadu_ps(b_ptr.add(offset));
        let diff = _mm256_sub_ps(va, vb);
        sum = _mm256_fmadd_ps(diff, diff, sum);
    }

    let hi = _mm256_extractf128_ps(sum, 1);
    let lo = _mm256_castps256_ps128(sum);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    // Handle remainder (max 7 elements)
    let base = simd_len * 8;
    let remainder = len - base;

    sum_squared_remainder_unrolled_8!(a, b, base, remainder, result);

    result
}

/// AVX2 squared L2 with 4 accumulators for very large vectors (256+).
///
/// Maximizes ILP by using 4 independent accumulators to hide FMA latency.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
/// - Vector length >= 256 (dispatch threshold)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn squared_l2_avx2_4acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX2+FMA.
    // - `_mm256_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic stays within bounds: checked by end_ptr comparison
    use std::arch::x86_64::*;

    let len = a.len();
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 32 * 32);
    let end_ptr = a.as_ptr().add(len);

    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    let mut acc2 = _mm256_setzero_ps();
    let mut acc3 = _mm256_setzero_ps();

    // Main loop: process 32 elements at a time (4 × 8 lanes)
    while a_ptr < end_main {
        let va0 = _mm256_loadu_ps(a_ptr);
        let vb0 = _mm256_loadu_ps(b_ptr);
        let diff0 = _mm256_sub_ps(va0, vb0);
        acc0 = _mm256_fmadd_ps(diff0, diff0, acc0);

        let va1 = _mm256_loadu_ps(a_ptr.add(8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(8));
        let diff1 = _mm256_sub_ps(va1, vb1);
        acc1 = _mm256_fmadd_ps(diff1, diff1, acc1);

        let va2 = _mm256_loadu_ps(a_ptr.add(16));
        let vb2 = _mm256_loadu_ps(b_ptr.add(16));
        let diff2 = _mm256_sub_ps(va2, vb2);
        acc2 = _mm256_fmadd_ps(diff2, diff2, acc2);

        let va3 = _mm256_loadu_ps(a_ptr.add(24));
        let vb3 = _mm256_loadu_ps(b_ptr.add(24));
        let diff3 = _mm256_sub_ps(va3, vb3);
        acc3 = _mm256_fmadd_ps(diff3, diff3, acc3);

        a_ptr = a_ptr.add(32);
        b_ptr = b_ptr.add(32);
    }

    // Combine accumulators
    let sum01 = _mm256_add_ps(acc0, acc1);
    let sum23 = _mm256_add_ps(acc2, acc3);
    let sum = _mm256_add_ps(sum01, sum23);

    // Horizontal sum
    let hi = _mm256_extractf128_ps(sum, 1);
    let lo = _mm256_castps256_ps128(sum);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    // Handle remainder with scalar
    while a_ptr < end_ptr {
        let d = *a_ptr - *b_ptr;
        result += d * d;
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }

    result
}
