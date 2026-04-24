//! AVX2+FMA squared L2 distance kernel implementations for x86_64.
//!
//! Contains hand-tuned AVX2 SIMD kernels for squared L2 distance
//! with 1-acc, 2-acc, and 4-acc variants for different vector sizes.
//!
//! All functions require runtime AVX2+FMA detection before calling.
//! Dispatch is handled by `dispatch.rs` after `simd_level()` confirms support.

// Reason: Numeric casts in this file are intentional and safe:
// - All casts are from well-bounded values (vector dimensions, loop indices)
// - All casts are validated by extensive SIMD tests (simd_native_tests.rs)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::incompatible_msrv)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::similar_names)]

use crate::simd_4acc_l2_loop;
use crate::simd_native::reduction::hsum_avx256;
use crate::sum_squared_remainder_unrolled_8;

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
    let mut result = hsum_avx256(combined);

    let base = simd_len * 16;
    let remainder = len - base;

    if remainder >= 8 {
        // Process 8 more elements with SIMD
        let va = _mm256_loadu_ps(a_ptr.add(base));
        let vb = _mm256_loadu_ps(b_ptr.add(base));
        let diff = _mm256_sub_ps(va, vb);
        let tmp_sum = _mm256_fmadd_ps(diff, diff, _mm256_setzero_ps());
        result += hsum_avx256(tmp_sum);

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

    let mut result = hsum_avx256(sum);

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
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 32 * 32);
    let end_ptr = a_ptr.add(len);

    // SAFETY: 4-accumulator ILP loop. Pointer bounds guaranteed by end_main.
    let (combined, mut a_p, mut b_p) = simd_4acc_l2_loop!(
        a_ptr,
        b_ptr,
        end_main,
        _mm256_setzero_ps(),
        _mm256_loadu_ps,
        _mm256_sub_ps,
        _mm256_fmadd_ps,
        _mm256_add_ps,
        8
    );

    let mut result = hsum_avx256(combined);

    // Handle remainder with scalar
    while a_p < end_ptr {
        let d = *a_p - *b_p;
        result += d * d;
        a_p = a_p.add(1);
        b_p = b_p.add(1);
    }

    result
}
