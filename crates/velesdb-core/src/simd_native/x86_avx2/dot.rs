//! AVX2+FMA dot product kernel implementations for x86_64.
//!
//! Contains hand-tuned AVX2 SIMD kernels for dot product
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

use crate::simd_4acc_dot_loop;
use crate::simd_native::reduction::hsum_avx256;
use crate::sum_remainder_unrolled_8;

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
    // - Pointer arithmetic stays within bounds: end_main = len / 32 * 32 ≤ len
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 32 * 32);

    // SAFETY: 4-accumulator ILP loop. Pointer bounds guaranteed by end_main.
    let (combined, _, _) = simd_4acc_dot_loop!(
        a_ptr,
        b_ptr,
        end_main,
        _mm256_setzero_ps(),
        _mm256_loadu_ps,
        _mm256_fmadd_ps,
        _mm256_add_ps,
        8
    );

    let mut result = hsum_avx256(combined);

    // Handle remainder (max 31 elements) with unrolled tail
    let base = len / 32 * 32;
    let remainder = len - base;
    result += dot_avx2_remainder(a, b, a_ptr, b_ptr, base, remainder);

    result
}

/// Process AVX2 dot product remainder (0-31 elements after main 4-acc loop).
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA
/// - `a_ptr` and `b_ptr` point to valid memory for `base + remainder` elements
/// - `base + remainder <= a.len()` and `base + remainder <= b.len()`
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn dot_avx2_remainder(
    a: &[f32],
    b: &[f32],
    a_ptr: *const f32,
    b_ptr: *const f32,
    base: usize,
    remainder: usize,
) -> f32 {
    // SAFETY: All pointer operations stay within bounds validated by caller.
    // Parent function guarantees base + remainder == a.len() == b.len().
    use std::arch::x86_64::*;

    let mut result = 0.0_f32;

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
        result += hsum_avx256(sum0);

        // Handle remaining 0-15 elements
        if remainder > 16 {
            let rbase = base + 16;
            let r = remainder - 16;
            result += dot_avx2_tail_under16(a, b, a_ptr, b_ptr, rbase, r);
        }
    } else if remainder >= 8 {
        let va = _mm256_loadu_ps(a_ptr.add(base));
        let vb = _mm256_loadu_ps(b_ptr.add(base));
        let tmp = _mm256_fmadd_ps(va, vb, _mm256_setzero_ps());
        result += hsum_avx256(tmp);

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

/// Process tail of 0-15 elements using AVX2 8-wide + scalar remainder.
///
/// # Safety
///
/// Same as `dot_avx2_remainder`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
unsafe fn dot_avx2_tail_under16(
    a: &[f32],
    b: &[f32],
    a_ptr: *const f32,
    b_ptr: *const f32,
    base: usize,
    remainder: usize,
) -> f32 {
    // SAFETY: Pointer operations bounded by caller validation.
    use std::arch::x86_64::*;

    let mut result = 0.0_f32;

    if remainder >= 8 {
        let va = _mm256_loadu_ps(a_ptr.add(base));
        let vb = _mm256_loadu_ps(b_ptr.add(base));
        let tmp = _mm256_fmadd_ps(va, vb, _mm256_setzero_ps());
        result += hsum_avx256(tmp);

        if remainder > 8 {
            let rbase = base + 8;
            let r = remainder - 8;
            sum_remainder_unrolled_8!(a, b, rbase, r, result);
        }
    } else {
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
        sum = _mm256_fmadd_ps(va, vb, sum);
    }

    let mut result = hsum_avx256(sum);

    // Handle remainder (max 7 elements)
    let base = simd_len * 8;
    let remainder = len - base;

    sum_remainder_unrolled_8!(a, b, base, remainder, result);

    result
}

/// AVX2 dot product with 2 accumulators for ILP.
///
/// Best for vectors 64-127 elements where 4-acc overhead isn't worth it.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
#[allow(clippy::too_many_lines)] // Remainder unrolling adds lines for performance
pub(crate) unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: See dot_product_avx2_4acc for detailed safety justification.
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
        sum0 = _mm256_fmadd_ps(va0, vb0, sum0);

        let va1 = _mm256_loadu_ps(a_ptr.add(offset + 8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(offset + 8));
        sum1 = _mm256_fmadd_ps(va1, vb1, sum1);
    }

    let combined = _mm256_add_ps(sum0, sum1);
    let mut result = hsum_avx256(combined);

    // Handle remainder (max 15 elements)
    let base = simd_len * 16;
    let remainder = len - base;

    if remainder >= 8 {
        let va = _mm256_loadu_ps(a_ptr.add(base));
        let vb = _mm256_loadu_ps(b_ptr.add(base));
        let tmp = _mm256_fmadd_ps(va, vb, _mm256_setzero_ps());
        result += hsum_avx256(tmp);

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
