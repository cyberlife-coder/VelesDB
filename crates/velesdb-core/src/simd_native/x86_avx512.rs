//! AVX-512F kernel implementations for x86_64.
//!
//! Contains hand-tuned AVX-512 SIMD kernels for dot product, squared L2,
//! cosine similarity, Hamming distance, and Jaccard similarity.
//!
//! All functions require runtime AVX-512F detection before calling.
//! Dispatch is handled by `dispatch.rs` after `simd_level()` confirms support.

// SAFETY: Numeric casts in this file are intentional and safe:
// - All casts are from well-bounded values (vector dimensions, loop indices)
// - usize->f64 casts are for statistical calculations where 52-bit mantissa is sufficient
// - All casts are validated by extensive SIMD tests (simd_native_tests.rs)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::incompatible_msrv)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::similar_names)]

use crate::simd_4acc_dot_loop;
use crate::simd_4acc_l2_loop;
use crate::simd_8acc_dot_loop;
use crate::simd_8acc_l2_loop;

use super::reduction::hsum_avx512;
use super::scalar;
use super::scalar::cosine_finish_fast;

// =============================================================================
// Dot Product
// =============================================================================

/// AVX-512 dot product using native intrinsics.
///
/// Processes 16 floats per iteration using `_mm512_fmadd_ps`.
/// Falls back to AVX2 or scalar if AVX-512 not available.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn dot_product_avx512(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_ps` and `_mm512_maskz_loadu_ps` handle unaligned loads safely
    // - Pointer arithmetic stays within bounds: offset = i * 16 where i < simd_len = len / 16
    // - Both slices have equal length (caller's responsibility via public API assert)
    // - Masked loads only read elements within bounds (mask controls which elements are loaded)
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16;
    let remainder = len % 16;

    let mut sum = _mm512_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va = _mm512_loadu_ps(a_ptr.add(offset));
        let vb = _mm512_loadu_ps(b_ptr.add(offset));
        sum = _mm512_fmadd_ps(va, vb, sum);
    }

    // Handle remainder with masked load (EPIC-PERF-002)
    // This eliminates the scalar tail loop for better performance
    if remainder > 0 {
        let base = simd_len * 16;
        // SAFETY: remainder is in 1..=16, mask computed without overflow
        let mask: __mmask16 = if remainder == 16 {
            !0
        } else {
            ((1u32 << remainder) - 1) as u16
        };
        let va = _mm512_maskz_loadu_ps(mask, a_ptr.add(base));
        let vb = _mm512_maskz_loadu_ps(mask, b_ptr.add(base));
        sum = _mm512_fmadd_ps(va, vb, sum);
    }

    hsum_avx512(sum)
}

/// Optimized 4-accumulator version without prefetch overhead
/// and simplified remainder handling.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]`)
/// - `a.len() == b.len()` (enforced by public API assert)
/// - `a.len() >= 64` for optimal performance (dispatch threshold is 512)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn dot_product_avx512_4acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic: stays within bounds, checked by end_ptr comparison
    // - Masked loads only read elements within bounds
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 64 * 64);
    let end_ptr = a_ptr.add(len);

    // SAFETY: 4-accumulator ILP loop. Pointer bounds guaranteed by end_main.
    let (mut acc, mut a_p, mut b_p) = simd_4acc_dot_loop!(
        a_ptr,
        b_ptr,
        end_main,
        _mm512_setzero_ps(),
        _mm512_loadu_ps,
        _mm512_fmadd_ps,
        _mm512_add_ps,
        16
    );

    // Process remaining 16-element chunks with same accumulator
    while a_p.add(16) <= end_ptr {
        let va = _mm512_loadu_ps(a_p);
        let vb = _mm512_loadu_ps(b_p);
        acc = _mm512_fmadd_ps(va, vb, acc);
        a_p = a_p.add(16);
        b_p = b_p.add(16);
    }

    // Final masked chunk if any
    let remaining = end_ptr.offset_from(a_p) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=16, mask computed without overflow
        let mask: __mmask16 = if remaining == 16 {
            !0
        } else {
            ((1u32 << remaining) - 1) as u16
        };
        let va = _mm512_maskz_loadu_ps(mask, a_p);
        let vb = _mm512_maskz_loadu_ps(mask, b_p);
        acc = _mm512_fmadd_ps(va, vb, acc);
    }

    hsum_avx512(acc)
}

/// AVX-512 dot product with 8 independent accumulators for very large vectors.
///
/// Processes 128 floats (8 x 16) per iteration using 8 independent FMA chains,
/// consuming 16 ZMM registers (8 accumulators + 8 temporaries per iteration).
/// Targets vectors >= 1024 dimensions where additional ILP hides FMA latency.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
/// - `a.len() >= 128` for optimal performance (dispatch threshold is 1024)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn dot_product_avx512_8acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic: stays within bounds, checked by end_main comparison
    // - Masked loads only read elements within bounds
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 128 * 128);
    let end_ptr = a_ptr.add(len);

    // SAFETY: 8-accumulator ILP loop. Pointer bounds guaranteed by end_main.
    let (mut acc, mut a_p, mut b_p) = simd_8acc_dot_loop!(
        a_ptr,
        b_ptr,
        end_main,
        _mm512_setzero_ps(),
        _mm512_loadu_ps,
        _mm512_fmadd_ps,
        _mm512_add_ps,
        16
    );

    // Process remaining 16-element chunks with single accumulator
    while a_p.add(16) <= end_ptr {
        acc = _mm512_fmadd_ps(_mm512_loadu_ps(a_p), _mm512_loadu_ps(b_p), acc);
        a_p = a_p.add(16);
        b_p = b_p.add(16);
    }

    // Final masked chunk if any
    let remaining = end_ptr.offset_from(a_p) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=15, mask computed without overflow
        let mask: __mmask16 = ((1u32 << remaining) - 1) as u16;
        let va = _mm512_maskz_loadu_ps(mask, a_p);
        let vb = _mm512_maskz_loadu_ps(mask, b_p);
        acc = _mm512_fmadd_ps(va, vb, acc);
    }

    hsum_avx512(acc)
}

// =============================================================================
// Squared L2 Distance
// =============================================================================

/// AVX-512 squared L2 distance with 4 accumulators for ILP.
///
/// # Safety
///
/// Same requirements as `dot_product_avx512_4acc`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn squared_l2_avx512_4acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: See module-level "Unsafe Invariants Reference" and
    // `dot_product_avx512_4acc` for per-loop bound guarantees.
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 64 * 64);
    let end_ptr = a_ptr.add(len);

    // SAFETY: 4-accumulator ILP loop. Pointer bounds guaranteed by end_main.
    let (mut acc, mut a_p, mut b_p) = simd_4acc_l2_loop!(
        a_ptr,
        b_ptr,
        end_main,
        _mm512_setzero_ps(),
        _mm512_loadu_ps,
        _mm512_sub_ps,
        _mm512_fmadd_ps,
        _mm512_add_ps,
        16
    );

    // Process remaining 16-element chunks
    while a_p.add(16) <= end_ptr {
        let va = _mm512_loadu_ps(a_p);
        let vb = _mm512_loadu_ps(b_p);
        let diff = _mm512_sub_ps(va, vb);
        acc = _mm512_fmadd_ps(diff, diff, acc);
        a_p = a_p.add(16);
        b_p = b_p.add(16);
    }

    // Final masked chunk if any
    let remaining = end_ptr.offset_from(a_p) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=16, mask computed without overflow
        let mask: __mmask16 = if remaining == 16 {
            !0
        } else {
            ((1u32 << remaining) - 1) as u16
        };
        let va = _mm512_maskz_loadu_ps(mask, a_p);
        let vb = _mm512_maskz_loadu_ps(mask, b_p);
        let diff = _mm512_sub_ps(va, vb);
        acc = _mm512_fmadd_ps(diff, diff, acc);
    }

    hsum_avx512(acc)
}

/// AVX-512 squared L2 distance (1-acc fallback for small vectors).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn squared_l2_avx512(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: See module-level "Unsafe Invariants Reference".
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16;
    let remainder = len % 16;

    let mut sum = _mm512_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va = _mm512_loadu_ps(a_ptr.add(offset));
        let vb = _mm512_loadu_ps(b_ptr.add(offset));
        let diff = _mm512_sub_ps(va, vb);
        sum = _mm512_fmadd_ps(diff, diff, sum);
    }

    if remainder > 0 {
        let base = simd_len * 16;
        // SAFETY: remainder is in 1..=16, mask computed without overflow
        let mask: __mmask16 = if remainder == 16 {
            !0
        } else {
            ((1u32 << remainder) - 1) as u16
        };
        let va = _mm512_maskz_loadu_ps(mask, a_ptr.add(base));
        let vb = _mm512_maskz_loadu_ps(mask, b_ptr.add(base));
        let diff = _mm512_sub_ps(va, vb);
        sum = _mm512_fmadd_ps(diff, diff, sum);
    }

    hsum_avx512(sum)
}

/// AVX-512 squared L2 distance with 8 independent accumulators for very large vectors.
///
/// Processes 128 floats (8 x 16) per iteration. Targets vectors >= 1024 dimensions.
///
/// # Safety
///
/// Same requirements as `dot_product_avx512_8acc`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn squared_l2_avx512_8acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic: stays within bounds, checked by end_main comparison
    // - Masked loads only read elements within bounds
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 128 * 128);
    let end_ptr = a_ptr.add(len);

    // SAFETY: 8-accumulator ILP loop. Pointer bounds guaranteed by end_main.
    let (mut acc, mut a_p, mut b_p) = simd_8acc_l2_loop!(
        a_ptr,
        b_ptr,
        end_main,
        _mm512_setzero_ps(),
        _mm512_loadu_ps,
        _mm512_sub_ps,
        _mm512_fmadd_ps,
        _mm512_add_ps,
        16
    );

    // Process remaining 16-element chunks
    while a_p.add(16) <= end_ptr {
        let diff = _mm512_sub_ps(_mm512_loadu_ps(a_p), _mm512_loadu_ps(b_p));
        acc = _mm512_fmadd_ps(diff, diff, acc);
        a_p = a_p.add(16);
        b_p = b_p.add(16);
    }

    // Final masked chunk if any
    let remaining = end_ptr.offset_from(a_p) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=15, mask computed without overflow
        let mask: __mmask16 = ((1u32 << remaining) - 1) as u16;
        let va = _mm512_maskz_loadu_ps(mask, a_p);
        let vb = _mm512_maskz_loadu_ps(mask, b_p);
        let diff = _mm512_sub_ps(va, vb);
        acc = _mm512_fmadd_ps(diff, diff, acc);
    }

    hsum_avx512(acc)
}

// =============================================================================
// Cosine Similarity (Fused)
// =============================================================================

/// AVX-512 fused cosine similarity - computes dot product and norms in single SIMD pass.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn cosine_fused_avx512(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: runtime feature detection confirms AVX-512F.
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_chunks = len / 32; // 32 floats per 2-way unroll
    let remainder = len % 32;

    let mut dot0 = _mm512_setzero_ps();
    let mut dot1 = _mm512_setzero_ps();
    let mut na0 = _mm512_setzero_ps();
    let mut na1 = _mm512_setzero_ps();
    let mut nb0 = _mm512_setzero_ps();
    let mut nb1 = _mm512_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_chunks {
        let base = i * 32;
        let va0 = _mm512_loadu_ps(a_ptr.add(base));
        let vb0 = _mm512_loadu_ps(b_ptr.add(base));
        dot0 = _mm512_fmadd_ps(va0, vb0, dot0);
        na0 = _mm512_fmadd_ps(va0, va0, na0);
        nb0 = _mm512_fmadd_ps(vb0, vb0, nb0);

        let va1 = _mm512_loadu_ps(a_ptr.add(base + 16));
        let vb1 = _mm512_loadu_ps(b_ptr.add(base + 16));
        dot1 = _mm512_fmadd_ps(va1, vb1, dot1);
        na1 = _mm512_fmadd_ps(va1, va1, na1);
        nb1 = _mm512_fmadd_ps(vb1, vb1, nb1);
    }

    // Remainder up to 31 elements with mask
    if remainder > 0 {
        let base = simd_chunks * 32;
        let rem0 = remainder.min(16);
        if rem0 > 0 {
            // SAFETY: rem0 is in 1..=16, mask computed without overflow
            let mask0: __mmask16 = if rem0 == 16 {
                !0
            } else {
                ((1u32 << rem0) - 1) as u16
            };
            let va = _mm512_maskz_loadu_ps(mask0, a_ptr.add(base));
            let vb = _mm512_maskz_loadu_ps(mask0, b_ptr.add(base));
            dot0 = _mm512_fmadd_ps(va, vb, dot0);
            na0 = _mm512_fmadd_ps(va, va, na0);
            nb0 = _mm512_fmadd_ps(vb, vb, nb0);
        }
        let rem1 = remainder.saturating_sub(16);
        if rem1 > 0 {
            // SAFETY: rem1 is in 1..=15 (since remainder <= 31), mask computed without overflow
            let mask1: __mmask16 = if rem1 == 16 {
                !0
            } else {
                ((1u32 << rem1) - 1) as u16
            };
            let va = _mm512_maskz_loadu_ps(mask1, a_ptr.add(base + 16));
            let vb = _mm512_maskz_loadu_ps(mask1, b_ptr.add(base + 16));
            dot1 = _mm512_fmadd_ps(va, vb, dot1);
            na1 = _mm512_fmadd_ps(va, va, na1);
            nb1 = _mm512_fmadd_ps(vb, vb, nb1);
        }
    }

    let dot = hsum_avx512(_mm512_add_ps(dot0, dot1));
    let norm_a_sq = hsum_avx512(_mm512_add_ps(na0, na1));
    let norm_b_sq = hsum_avx512(_mm512_add_ps(nb0, nb1));

    cosine_finish_fast(dot, norm_a_sq, norm_b_sq)
}

/// AVX-512 fused cosine similarity with 4-way unrolling for large dimensions.
///
/// Processes 64 floats per iteration using 12 accumulators (4x dot, 4x norm_a, 4x norm_b).
/// AVX-512 has 32 zmm registers, so 12 accumulators + 8 temporaries fit comfortably.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn cosine_fused_avx512_4acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: runtime feature detection confirms AVX-512F.
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 64 * 64);
    let end_ptr = a_ptr.add(len);

    let mut dot0 = _mm512_setzero_ps();
    let mut dot1 = _mm512_setzero_ps();
    let mut dot2 = _mm512_setzero_ps();
    let mut dot3 = _mm512_setzero_ps();
    let mut na0 = _mm512_setzero_ps();
    let mut na1 = _mm512_setzero_ps();
    let mut na2 = _mm512_setzero_ps();
    let mut na3 = _mm512_setzero_ps();
    let mut nb0 = _mm512_setzero_ps();
    let mut nb1 = _mm512_setzero_ps();
    let mut nb2 = _mm512_setzero_ps();
    let mut nb3 = _mm512_setzero_ps();

    let mut cur_a = a_ptr;
    let mut cur_b = b_ptr;

    while cur_a < end_main {
        let va0 = _mm512_loadu_ps(cur_a);
        let vb0 = _mm512_loadu_ps(cur_b);
        dot0 = _mm512_fmadd_ps(va0, vb0, dot0);
        na0 = _mm512_fmadd_ps(va0, va0, na0);
        nb0 = _mm512_fmadd_ps(vb0, vb0, nb0);

        let va1 = _mm512_loadu_ps(cur_a.add(16));
        let vb1 = _mm512_loadu_ps(cur_b.add(16));
        dot1 = _mm512_fmadd_ps(va1, vb1, dot1);
        na1 = _mm512_fmadd_ps(va1, va1, na1);
        nb1 = _mm512_fmadd_ps(vb1, vb1, nb1);

        let va2 = _mm512_loadu_ps(cur_a.add(32));
        let vb2 = _mm512_loadu_ps(cur_b.add(32));
        dot2 = _mm512_fmadd_ps(va2, vb2, dot2);
        na2 = _mm512_fmadd_ps(va2, va2, na2);
        nb2 = _mm512_fmadd_ps(vb2, vb2, nb2);

        let va3 = _mm512_loadu_ps(cur_a.add(48));
        let vb3 = _mm512_loadu_ps(cur_b.add(48));
        dot3 = _mm512_fmadd_ps(va3, vb3, dot3);
        na3 = _mm512_fmadd_ps(va3, va3, na3);
        nb3 = _mm512_fmadd_ps(vb3, vb3, nb3);

        cur_a = cur_a.add(64);
        cur_b = cur_b.add(64);
    }

    // Reduce 4 accumulators to 1
    let dot_acc = _mm512_add_ps(_mm512_add_ps(dot0, dot1), _mm512_add_ps(dot2, dot3));
    let mut na_acc = _mm512_add_ps(_mm512_add_ps(na0, na1), _mm512_add_ps(na2, na3));
    let mut nb_acc = _mm512_add_ps(_mm512_add_ps(nb0, nb1), _mm512_add_ps(nb2, nb3));

    // Remainder with masked loads (up to 63 elements)
    let mut rem_dot = dot_acc;
    while cur_a.add(16) <= end_ptr {
        let va = _mm512_loadu_ps(cur_a);
        let vb = _mm512_loadu_ps(cur_b);
        rem_dot = _mm512_fmadd_ps(va, vb, rem_dot);
        na_acc = _mm512_fmadd_ps(va, va, na_acc);
        nb_acc = _mm512_fmadd_ps(vb, vb, nb_acc);
        cur_a = cur_a.add(16);
        cur_b = cur_b.add(16);
    }

    // Final masked chunk if any
    let remaining = end_ptr.offset_from(cur_a) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=15, mask computed without overflow
        let mask: __mmask16 = ((1u32 << remaining) - 1) as u16;
        let va = _mm512_maskz_loadu_ps(mask, cur_a);
        let vb = _mm512_maskz_loadu_ps(mask, cur_b);
        rem_dot = _mm512_fmadd_ps(va, vb, rem_dot);
        na_acc = _mm512_fmadd_ps(va, va, na_acc);
        nb_acc = _mm512_fmadd_ps(vb, vb, nb_acc);
    }

    let dot = hsum_avx512(rem_dot);
    let norm_a_sq = hsum_avx512(na_acc);
    let norm_b_sq = hsum_avx512(nb_acc);

    cosine_finish_fast(dot, norm_a_sq, norm_b_sq)
}

/// AVX-512 fused cosine similarity with 8-way unrolling for very large vectors.
///
/// Processes 128 floats per iteration using 24 accumulators (8x dot, 8x norm_a,
/// 8x norm_b). AVX-512 has 32 ZMM registers, so 24 accumulators fit with 8
/// registers available for temporaries during load/compute phases.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn cosine_fused_avx512_8acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: runtime feature detection confirms AVX-512F.
    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 128 * 128);
    let end_ptr = a_ptr.add(len);

    let (mut dot_acc, mut na_acc, mut nb_acc, mut cur_a, mut cur_b) =
        cosine_8acc_main_loop(a_ptr, b_ptr, end_main);

    // Remainder: 16-element full chunks then masked tail
    cosine_8acc_remainder(
        &mut cur_a,
        &mut cur_b,
        end_ptr,
        &mut dot_acc,
        &mut na_acc,
        &mut nb_acc,
    );

    let dot = hsum_avx512(dot_acc);
    let norm_a_sq = hsum_avx512(na_acc);
    let norm_b_sq = hsum_avx512(nb_acc);
    cosine_finish_fast(dot, norm_a_sq, norm_b_sq)
}

/// Main loop for 8-accumulator fused cosine: 128 floats per iteration.
///
/// Returns `(dot_acc, na_acc, nb_acc, cur_a, cur_b)` after reduction from
/// 8 accumulators each down to 1.
///
/// # Safety
///
/// Caller must ensure AVX-512F is available and `end_main` is aligned to 128 floats.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(clippy::too_many_lines)] // Reason: 24-accumulator SIMD kernel; extracting further would hurt ILP clarity
unsafe fn cosine_8acc_main_loop(
    a_ptr: *const f32,
    b_ptr: *const f32,
    end_main: *const f32,
) -> (
    std::arch::x86_64::__m512,
    std::arch::x86_64::__m512,
    std::arch::x86_64::__m512,
    *const f32,
    *const f32,
) {
    use std::arch::x86_64::*;

    let z = _mm512_setzero_ps();
    let (mut d0, mut d1, mut d2, mut d3) = (z, z, z, z);
    let (mut d4, mut d5, mut d6, mut d7) = (z, z, z, z);
    let (mut a0, mut a1, mut a2, mut a3) = (z, z, z, z);
    let (mut a4, mut a5, mut a6, mut a7) = (z, z, z, z);
    let (mut b0, mut b1, mut b2, mut b3) = (z, z, z, z);
    let (mut b4, mut b5, mut b6, mut b7) = (z, z, z, z);
    let mut pa = a_ptr;
    let mut pb = b_ptr;

    // SAFETY: Loop guard ensures 128 elements remain for all eight 16-element loads
    while pa < end_main {
        cosine_8acc_body_lo(
            pa, pb, &mut d0, &mut d1, &mut d2, &mut d3, &mut a0, &mut a1, &mut a2, &mut a3,
            &mut b0, &mut b1, &mut b2, &mut b3,
        );
        cosine_8acc_body_hi(
            pa, pb, &mut d4, &mut d5, &mut d6, &mut d7, &mut a4, &mut a5, &mut a6, &mut a7,
            &mut b4, &mut b5, &mut b6, &mut b7,
        );
        pa = pa.add(128);
        pb = pb.add(128);
    }

    // Reduce 8 accumulators to 1 each via binary tree
    d0 = _mm512_add_ps(d0, d4);
    d1 = _mm512_add_ps(d1, d5);
    d2 = _mm512_add_ps(d2, d6);
    d3 = _mm512_add_ps(d3, d7);
    let dot_acc = _mm512_add_ps(_mm512_add_ps(d0, d1), _mm512_add_ps(d2, d3));

    a0 = _mm512_add_ps(a0, a4);
    a1 = _mm512_add_ps(a1, a5);
    a2 = _mm512_add_ps(a2, a6);
    a3 = _mm512_add_ps(a3, a7);
    let na_acc = _mm512_add_ps(_mm512_add_ps(a0, a1), _mm512_add_ps(a2, a3));

    b0 = _mm512_add_ps(b0, b4);
    b1 = _mm512_add_ps(b1, b5);
    b2 = _mm512_add_ps(b2, b6);
    b3 = _mm512_add_ps(b3, b7);
    let nb_acc = _mm512_add_ps(_mm512_add_ps(b0, b1), _mm512_add_ps(b2, b3));

    (dot_acc, na_acc, nb_acc, pa, pb)
}

/// Lower half (offsets 0..64) of the 8-acc cosine loop body.
///
/// # Safety
///
/// Caller must ensure 128 elements are readable from `pa`/`pb`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(clippy::too_many_arguments)] // Reason: SIMD kernel helper passes 12 accumulator refs + 2 pointers; no cleaner decomposition exists
unsafe fn cosine_8acc_body_lo(
    pa: *const f32,
    pb: *const f32,
    d0: &mut std::arch::x86_64::__m512,
    d1: &mut std::arch::x86_64::__m512,
    d2: &mut std::arch::x86_64::__m512,
    d3: &mut std::arch::x86_64::__m512,
    a0: &mut std::arch::x86_64::__m512,
    a1: &mut std::arch::x86_64::__m512,
    a2: &mut std::arch::x86_64::__m512,
    a3: &mut std::arch::x86_64::__m512,
    b0: &mut std::arch::x86_64::__m512,
    b1: &mut std::arch::x86_64::__m512,
    b2: &mut std::arch::x86_64::__m512,
    b3: &mut std::arch::x86_64::__m512,
) {
    use std::arch::x86_64::*;

    // SAFETY: Caller guarantees 128 readable elements from pa/pb
    let va0 = _mm512_loadu_ps(pa);
    let vb0 = _mm512_loadu_ps(pb);
    *d0 = _mm512_fmadd_ps(va0, vb0, *d0);
    *a0 = _mm512_fmadd_ps(va0, va0, *a0);
    *b0 = _mm512_fmadd_ps(vb0, vb0, *b0);

    let va1 = _mm512_loadu_ps(pa.add(16));
    let vb1 = _mm512_loadu_ps(pb.add(16));
    *d1 = _mm512_fmadd_ps(va1, vb1, *d1);
    *a1 = _mm512_fmadd_ps(va1, va1, *a1);
    *b1 = _mm512_fmadd_ps(vb1, vb1, *b1);

    let va2 = _mm512_loadu_ps(pa.add(32));
    let vb2 = _mm512_loadu_ps(pb.add(32));
    *d2 = _mm512_fmadd_ps(va2, vb2, *d2);
    *a2 = _mm512_fmadd_ps(va2, va2, *a2);
    *b2 = _mm512_fmadd_ps(vb2, vb2, *b2);

    let va3 = _mm512_loadu_ps(pa.add(48));
    let vb3 = _mm512_loadu_ps(pb.add(48));
    *d3 = _mm512_fmadd_ps(va3, vb3, *d3);
    *a3 = _mm512_fmadd_ps(va3, va3, *a3);
    *b3 = _mm512_fmadd_ps(vb3, vb3, *b3);
}

/// Upper half (offsets 64..128) of the 8-acc cosine loop body.
///
/// # Safety
///
/// Caller must ensure 128 elements are readable from `pa`/`pb`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(clippy::too_many_arguments)] // Reason: SIMD kernel helper passes 12 accumulator refs + 2 pointers; no cleaner decomposition exists
unsafe fn cosine_8acc_body_hi(
    pa: *const f32,
    pb: *const f32,
    d4: &mut std::arch::x86_64::__m512,
    d5: &mut std::arch::x86_64::__m512,
    d6: &mut std::arch::x86_64::__m512,
    d7: &mut std::arch::x86_64::__m512,
    a4: &mut std::arch::x86_64::__m512,
    a5: &mut std::arch::x86_64::__m512,
    a6: &mut std::arch::x86_64::__m512,
    a7: &mut std::arch::x86_64::__m512,
    b4: &mut std::arch::x86_64::__m512,
    b5: &mut std::arch::x86_64::__m512,
    b6: &mut std::arch::x86_64::__m512,
    b7: &mut std::arch::x86_64::__m512,
) {
    use std::arch::x86_64::*;

    // SAFETY: Caller guarantees 128 readable elements from pa/pb
    let va4 = _mm512_loadu_ps(pa.add(64));
    let vb4 = _mm512_loadu_ps(pb.add(64));
    *d4 = _mm512_fmadd_ps(va4, vb4, *d4);
    *a4 = _mm512_fmadd_ps(va4, va4, *a4);
    *b4 = _mm512_fmadd_ps(vb4, vb4, *b4);

    let va5 = _mm512_loadu_ps(pa.add(80));
    let vb5 = _mm512_loadu_ps(pb.add(80));
    *d5 = _mm512_fmadd_ps(va5, vb5, *d5);
    *a5 = _mm512_fmadd_ps(va5, va5, *a5);
    *b5 = _mm512_fmadd_ps(vb5, vb5, *b5);

    let va6 = _mm512_loadu_ps(pa.add(96));
    let vb6 = _mm512_loadu_ps(pb.add(96));
    *d6 = _mm512_fmadd_ps(va6, vb6, *d6);
    *a6 = _mm512_fmadd_ps(va6, va6, *a6);
    *b6 = _mm512_fmadd_ps(vb6, vb6, *b6);

    let va7 = _mm512_loadu_ps(pa.add(112));
    let vb7 = _mm512_loadu_ps(pb.add(112));
    *d7 = _mm512_fmadd_ps(va7, vb7, *d7);
    *a7 = _mm512_fmadd_ps(va7, va7, *a7);
    *b7 = _mm512_fmadd_ps(vb7, vb7, *b7);
}

/// Handles cosine 8-acc remainder: 16-element full chunks then a masked tail.
///
/// # Safety
///
/// Caller must ensure `cur_a`/`cur_b` point within valid slices and
/// `end_ptr` marks the end of the slice. All accumulators must be initialized.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
unsafe fn cosine_8acc_remainder(
    cur_a: &mut *const f32,
    cur_b: &mut *const f32,
    end_ptr: *const f32,
    dot_acc: &mut std::arch::x86_64::__m512,
    na_acc: &mut std::arch::x86_64::__m512,
    nb_acc: &mut std::arch::x86_64::__m512,
) {
    use std::arch::x86_64::*;

    // Process remaining 16-element full chunks
    while (*cur_a).add(16) <= end_ptr {
        // SAFETY: Loop guard ensures 16 elements remain
        let va = _mm512_loadu_ps(*cur_a);
        let vb = _mm512_loadu_ps(*cur_b);
        *dot_acc = _mm512_fmadd_ps(va, vb, *dot_acc);
        *na_acc = _mm512_fmadd_ps(va, va, *na_acc);
        *nb_acc = _mm512_fmadd_ps(vb, vb, *nb_acc);
        *cur_a = (*cur_a).add(16);
        *cur_b = (*cur_b).add(16);
    }

    // Final masked chunk for 1..15 remaining elements
    let remaining = end_ptr.offset_from(*cur_a) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=15, mask computed without overflow
        let mask: __mmask16 = ((1u32 << remaining) - 1) as u16;
        let va = _mm512_maskz_loadu_ps(mask, *cur_a);
        let vb = _mm512_maskz_loadu_ps(mask, *cur_b);
        *dot_acc = _mm512_fmadd_ps(va, vb, *dot_acc);
        *na_acc = _mm512_fmadd_ps(va, va, *na_acc);
        *nb_acc = _mm512_fmadd_ps(vb, vb, *nb_acc);
    }
}

// =============================================================================
// Hamming & Jaccard
// =============================================================================

/// AVX-512 Hamming distance.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub(crate) unsafe fn hamming_avx512(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    let mut diff_count: u64 = 0;
    let mut i = 0;

    // Threshold for binary comparison
    let threshold = _mm512_set1_ps(0.5);

    // Process 16 floats at a time using AVX-512
    while i + 16 <= len {
        let va = _mm512_loadu_ps(a_ptr.add(i));
        let vb = _mm512_loadu_ps(b_ptr.add(i));

        // Binary threshold: compare each value > 0.5
        let mask_a = _mm512_cmp_ps_mask(va, threshold, _CMP_GT_OQ);
        let mask_b = _mm512_cmp_ps_mask(vb, threshold, _CMP_GT_OQ);

        // XOR to find positions where binary values differ
        let diff_mask = mask_a ^ mask_b;
        diff_count += diff_mask.count_ones() as u64;

        i += 16;
    }

    // Handle remaining elements
    diff_count as f32 + scalar::hamming_scalar(&a[i..], &b[i..])
}

/// AVX-512 Jaccard similarity.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub(crate) unsafe fn jaccard_avx512(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    let mut acc_inter = _mm512_setzero_ps();
    let mut acc_union = _mm512_setzero_ps();

    let mut i = 0;
    // Process 16 floats at a time
    while i + 16 <= len {
        let va = _mm512_loadu_ps(a_ptr.add(i));
        let vb = _mm512_loadu_ps(b_ptr.add(i));

        // min for intersection, max for union
        acc_inter = _mm512_add_ps(acc_inter, _mm512_min_ps(va, vb));
        acc_union = _mm512_add_ps(acc_union, _mm512_max_ps(va, vb));

        i += 16;
    }

    // Horizontal sum
    let inter_sum = hsum_avx512(acc_inter);
    let union_sum = hsum_avx512(acc_union);

    // Handle remaining elements
    let (scalar_inter, scalar_union) = scalar::jaccard_scalar_accum(&a[i..], &b[i..]);

    let total_inter = inter_sum + scalar_inter;
    let total_union = union_sum + scalar_union;

    if total_union == 0.0 {
        1.0
    } else {
        total_inter / total_union
    }
}

/// AVX-512 Hamming distance with 4 independent accumulators for ILP.
///
/// Processes 64 elements (4 x 16) per iteration. Each chunk compares
/// binary-thresholded masks and counts differing bits via popcount on
/// the XOR result. The 4 independent `u64` accumulators break the
/// dependency chain so the CPU can execute multiple chunks in parallel.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn hamming_avx512_4acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic stays within bounds: main loop only runs while 64 elements remain
    // - Both slices have equal length (caller's responsibility via public API assert)
    // - `_mm512_cmp_ps_mask` returns a 16-bit mask; XOR and popcount are pure integer ops
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    let mut diff0: u64 = 0;
    let mut diff1: u64 = 0;
    let mut diff2: u64 = 0;
    let mut diff3: u64 = 0;

    let threshold = _mm512_set1_ps(0.5);
    let mut i = 0;

    // Main loop: 64 elements per iteration (4 x 16)
    while i + 64 <= len {
        // SAFETY: i + 64 <= len guarantees all four 16-element loads are in bounds
        diff0 += hamming_xor_popcount(a_ptr.add(i), b_ptr.add(i), threshold);
        diff1 += hamming_xor_popcount(a_ptr.add(i + 16), b_ptr.add(i + 16), threshold);
        diff2 += hamming_xor_popcount(a_ptr.add(i + 32), b_ptr.add(i + 32), threshold);
        diff3 += hamming_xor_popcount(a_ptr.add(i + 48), b_ptr.add(i + 48), threshold);
        i += 64;
    }

    // 16-element remainder chunks (up to 3 iterations)
    while i + 16 <= len {
        // SAFETY: i + 16 <= len guarantees the 16-element load is in bounds
        diff0 += hamming_xor_popcount(a_ptr.add(i), b_ptr.add(i), threshold);
        i += 16;
    }

    // Scalar tail for 0..15 remaining elements
    let simd_total = (diff0 + diff1 + diff2 + diff3) as f32;
    simd_total + scalar::hamming_scalar(&a[i..], &b[i..])
}

/// Loads one 16-element chunk, thresholds both vectors, XORs the masks,
/// and returns the popcount as a `u64`.
///
/// # Safety
///
/// Caller must ensure at least 16 elements are readable from both pointers.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
unsafe fn hamming_xor_popcount(
    a_ptr: *const f32,
    b_ptr: *const f32,
    threshold: std::arch::x86_64::__m512,
) -> u64 {
    use std::arch::x86_64::*;

    // SAFETY: Caller guarantees 16 elements are readable from both pointers.
    let va = _mm512_loadu_ps(a_ptr);
    let vb = _mm512_loadu_ps(b_ptr);
    let mask_a = _mm512_cmp_ps_mask(va, threshold, _CMP_GT_OQ);
    let mask_b = _mm512_cmp_ps_mask(vb, threshold, _CMP_GT_OQ);
    (mask_a ^ mask_b).count_ones() as u64
}

/// AVX-512 Jaccard similarity with 4 independent accumulators for ILP.
///
/// Processes 64 elements (4 x 16) per iteration using 8 `__m512`
/// accumulators (4 intersection via `min`, 4 union via `max`).
/// After the main loop, accumulators are reduced and remainder elements
/// are handled via 16-element chunks and a masked tail.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(clippy::too_many_lines)] // Reason: SIMD kernel with 8 accumulators; extracting more would hurt ILP clarity
pub(crate) unsafe fn jaccard_avx512_4acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic stays within bounds: main loop checks cur_a < end_main
    // - Both slices have equal length (caller's responsibility via public API assert)
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 64 * 64);
    let end_ptr = a_ptr.add(len);

    let zero = _mm512_setzero_ps();
    let mut inter0 = zero;
    let mut inter1 = zero;
    let mut inter2 = zero;
    let mut inter3 = zero;
    let mut union0 = zero;
    let mut union1 = zero;
    let mut union2 = zero;
    let mut union3 = zero;

    let mut cur_a = a_ptr;
    let mut cur_b = b_ptr;

    // Main loop: 64 elements per iteration (4 x 16)
    while cur_a < end_main {
        // SAFETY: cur_a < end_main guarantees 64 elements remain for all four loads
        let va0 = _mm512_loadu_ps(cur_a);
        let vb0 = _mm512_loadu_ps(cur_b);
        inter0 = _mm512_add_ps(inter0, _mm512_min_ps(va0, vb0));
        union0 = _mm512_add_ps(union0, _mm512_max_ps(va0, vb0));

        let va1 = _mm512_loadu_ps(cur_a.add(16));
        let vb1 = _mm512_loadu_ps(cur_b.add(16));
        inter1 = _mm512_add_ps(inter1, _mm512_min_ps(va1, vb1));
        union1 = _mm512_add_ps(union1, _mm512_max_ps(va1, vb1));

        let va2 = _mm512_loadu_ps(cur_a.add(32));
        let vb2 = _mm512_loadu_ps(cur_b.add(32));
        inter2 = _mm512_add_ps(inter2, _mm512_min_ps(va2, vb2));
        union2 = _mm512_add_ps(union2, _mm512_max_ps(va2, vb2));

        let va3 = _mm512_loadu_ps(cur_a.add(48));
        let vb3 = _mm512_loadu_ps(cur_b.add(48));
        inter3 = _mm512_add_ps(inter3, _mm512_min_ps(va3, vb3));
        union3 = _mm512_add_ps(union3, _mm512_max_ps(va3, vb3));

        cur_a = cur_a.add(64);
        cur_b = cur_b.add(64);
    }

    // Reduce 4 accumulators to 1 each
    let mut inter_acc = _mm512_add_ps(_mm512_add_ps(inter0, inter1), _mm512_add_ps(inter2, inter3));
    let mut union_acc = _mm512_add_ps(_mm512_add_ps(union0, union1), _mm512_add_ps(union2, union3));

    // Remainder: 16-element full chunks then masked tail
    jaccard_4acc_remainder(cur_a, cur_b, end_ptr, &mut inter_acc, &mut union_acc);

    let total_inter = hsum_avx512(inter_acc);
    let total_union = hsum_avx512(union_acc);

    if total_union == 0.0 {
        1.0
    } else {
        total_inter / total_union
    }
}

/// AVX-512 Jaccard similarity with 8 independent accumulators for very large vectors.
///
/// Processes 128 floats (8 x 16) per iteration with 8 intersection + 8 union
/// accumulators (16 ZMM registers out of 32 available).
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
/// - `a.len() >= 128` for optimal performance (dispatch threshold is 1024)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn jaccard_avx512_8acc(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_ps` handles unaligned loads safely
    // - Pointer arithmetic: stays within bounds, checked by end_main comparison
    // - Masked loads only read elements within bounds
    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a_ptr.add(len / 128 * 128);
    let end_ptr = a_ptr.add(len);

    let (mut inter_acc, mut union_acc, cur_a, cur_b) =
        jaccard_8acc_main_loop(a_ptr, b_ptr, end_main);

    // Remainder: 16-element full chunks then masked tail
    jaccard_8acc_remainder(cur_a, cur_b, end_ptr, &mut inter_acc, &mut union_acc);

    let total_inter = hsum_avx512(inter_acc);
    let total_union = hsum_avx512(union_acc);

    if total_union == 0.0 {
        1.0
    } else {
        total_inter / total_union
    }
}

/// Main loop for 8-accumulator Jaccard: processes 128 floats per iteration.
///
/// Returns `(inter_acc, union_acc, cur_a, cur_b)` after reducing
/// 8 accumulators each down to 1.
///
/// # Safety
///
/// Caller must ensure AVX-512F is available and `end_main` is aligned to 128 floats.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(clippy::too_many_lines)] // Reason: 16-accumulator SIMD kernel; extracting further would hurt ILP clarity
unsafe fn jaccard_8acc_main_loop(
    a_ptr: *const f32,
    b_ptr: *const f32,
    end_main: *const f32,
) -> (
    std::arch::x86_64::__m512,
    std::arch::x86_64::__m512,
    *const f32,
    *const f32,
) {
    use std::arch::x86_64::*;

    let z = _mm512_setzero_ps();
    let (mut i0, mut i1, mut i2, mut i3) = (z, z, z, z);
    let (mut i4, mut i5, mut i6, mut i7) = (z, z, z, z);
    let (mut u0, mut u1, mut u2, mut u3) = (z, z, z, z);
    let (mut u4, mut u5, mut u6, mut u7) = (z, z, z, z);
    let mut pa = a_ptr;
    let mut pb = b_ptr;

    // SAFETY: Loop guard ensures 128 elements remain for all eight 16-element loads
    while pa < end_main {
        jaccard_8acc_body_lo(
            pa, pb, &mut i0, &mut i1, &mut i2, &mut i3, &mut u0, &mut u1, &mut u2, &mut u3,
        );
        jaccard_8acc_body_hi(
            pa, pb, &mut i4, &mut i5, &mut i6, &mut i7, &mut u4, &mut u5, &mut u6, &mut u7,
        );
        pa = pa.add(128);
        pb = pb.add(128);
    }

    // Reduce 8 accumulators to 1 each via binary tree
    i0 = _mm512_add_ps(i0, i4);
    i1 = _mm512_add_ps(i1, i5);
    i2 = _mm512_add_ps(i2, i6);
    i3 = _mm512_add_ps(i3, i7);
    let inter_acc = _mm512_add_ps(_mm512_add_ps(i0, i1), _mm512_add_ps(i2, i3));

    u0 = _mm512_add_ps(u0, u4);
    u1 = _mm512_add_ps(u1, u5);
    u2 = _mm512_add_ps(u2, u6);
    u3 = _mm512_add_ps(u3, u7);
    let union_acc = _mm512_add_ps(_mm512_add_ps(u0, u1), _mm512_add_ps(u2, u3));

    (inter_acc, union_acc, pa, pb)
}

/// Lower half (offsets 0..64) of the 8-acc Jaccard loop body.
///
/// # Safety
///
/// Caller must ensure 128 elements are readable from `pa`/`pb`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(clippy::too_many_arguments)] // Reason: SIMD kernel helper passes 8 accumulator refs + 2 pointers; no cleaner decomposition exists
unsafe fn jaccard_8acc_body_lo(
    pa: *const f32,
    pb: *const f32,
    i0: &mut std::arch::x86_64::__m512,
    i1: &mut std::arch::x86_64::__m512,
    i2: &mut std::arch::x86_64::__m512,
    i3: &mut std::arch::x86_64::__m512,
    u0: &mut std::arch::x86_64::__m512,
    u1: &mut std::arch::x86_64::__m512,
    u2: &mut std::arch::x86_64::__m512,
    u3: &mut std::arch::x86_64::__m512,
) {
    use std::arch::x86_64::*;

    // SAFETY: Caller guarantees 128 readable elements from pa/pb
    let va0 = _mm512_loadu_ps(pa);
    let vb0 = _mm512_loadu_ps(pb);
    *i0 = _mm512_add_ps(*i0, _mm512_min_ps(va0, vb0));
    *u0 = _mm512_add_ps(*u0, _mm512_max_ps(va0, vb0));

    let va1 = _mm512_loadu_ps(pa.add(16));
    let vb1 = _mm512_loadu_ps(pb.add(16));
    *i1 = _mm512_add_ps(*i1, _mm512_min_ps(va1, vb1));
    *u1 = _mm512_add_ps(*u1, _mm512_max_ps(va1, vb1));

    let va2 = _mm512_loadu_ps(pa.add(32));
    let vb2 = _mm512_loadu_ps(pb.add(32));
    *i2 = _mm512_add_ps(*i2, _mm512_min_ps(va2, vb2));
    *u2 = _mm512_add_ps(*u2, _mm512_max_ps(va2, vb2));

    let va3 = _mm512_loadu_ps(pa.add(48));
    let vb3 = _mm512_loadu_ps(pb.add(48));
    *i3 = _mm512_add_ps(*i3, _mm512_min_ps(va3, vb3));
    *u3 = _mm512_add_ps(*u3, _mm512_max_ps(va3, vb3));
}

/// Upper half (offsets 64..128) of the 8-acc Jaccard loop body.
///
/// # Safety
///
/// Caller must ensure 128 elements are readable from `pa`/`pb`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(clippy::too_many_arguments)] // Reason: SIMD kernel helper passes 8 accumulator refs + 2 pointers; no cleaner decomposition exists
unsafe fn jaccard_8acc_body_hi(
    pa: *const f32,
    pb: *const f32,
    i4: &mut std::arch::x86_64::__m512,
    i5: &mut std::arch::x86_64::__m512,
    i6: &mut std::arch::x86_64::__m512,
    i7: &mut std::arch::x86_64::__m512,
    u4: &mut std::arch::x86_64::__m512,
    u5: &mut std::arch::x86_64::__m512,
    u6: &mut std::arch::x86_64::__m512,
    u7: &mut std::arch::x86_64::__m512,
) {
    use std::arch::x86_64::*;

    // SAFETY: Caller guarantees 128 readable elements from pa/pb
    let va4 = _mm512_loadu_ps(pa.add(64));
    let vb4 = _mm512_loadu_ps(pb.add(64));
    *i4 = _mm512_add_ps(*i4, _mm512_min_ps(va4, vb4));
    *u4 = _mm512_add_ps(*u4, _mm512_max_ps(va4, vb4));

    let va5 = _mm512_loadu_ps(pa.add(80));
    let vb5 = _mm512_loadu_ps(pb.add(80));
    *i5 = _mm512_add_ps(*i5, _mm512_min_ps(va5, vb5));
    *u5 = _mm512_add_ps(*u5, _mm512_max_ps(va5, vb5));

    let va6 = _mm512_loadu_ps(pa.add(96));
    let vb6 = _mm512_loadu_ps(pb.add(96));
    *i6 = _mm512_add_ps(*i6, _mm512_min_ps(va6, vb6));
    *u6 = _mm512_add_ps(*u6, _mm512_max_ps(va6, vb6));

    let va7 = _mm512_loadu_ps(pa.add(112));
    let vb7 = _mm512_loadu_ps(pb.add(112));
    *i7 = _mm512_add_ps(*i7, _mm512_min_ps(va7, vb7));
    *u7 = _mm512_add_ps(*u7, _mm512_max_ps(va7, vb7));
}

/// Handles Jaccard 8-acc remainder: 16-element full chunks then a masked tail.
///
/// # Safety
///
/// Caller must ensure `cur_a`/`cur_b` point within valid slices and
/// `end_ptr` marks the end of the slice. Both accumulators must be initialized.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
unsafe fn jaccard_8acc_remainder(
    mut cur_a: *const f32,
    mut cur_b: *const f32,
    end_ptr: *const f32,
    inter_acc: &mut std::arch::x86_64::__m512,
    union_acc: &mut std::arch::x86_64::__m512,
) {
    use std::arch::x86_64::*;

    // Process remaining 16-element full chunks
    while cur_a.add(16) <= end_ptr {
        // SAFETY: Loop guard ensures 16 elements remain
        let va = _mm512_loadu_ps(cur_a);
        let vb = _mm512_loadu_ps(cur_b);
        *inter_acc = _mm512_add_ps(*inter_acc, _mm512_min_ps(va, vb));
        *union_acc = _mm512_add_ps(*union_acc, _mm512_max_ps(va, vb));
        cur_a = cur_a.add(16);
        cur_b = cur_b.add(16);
    }

    // Final masked chunk for 1..15 remaining elements
    let remaining = end_ptr.offset_from(cur_a) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=15, mask computed without overflow
        let mask: __mmask16 = ((1u32 << remaining) - 1) as u16;
        let va = _mm512_maskz_loadu_ps(mask, cur_a);
        let vb = _mm512_maskz_loadu_ps(mask, cur_b);
        *inter_acc = _mm512_add_ps(*inter_acc, _mm512_min_ps(va, vb));
        *union_acc = _mm512_add_ps(*union_acc, _mm512_max_ps(va, vb));
    }
}

/// Handles Jaccard remainder: 16-element full chunks then a masked tail.
///
/// # Safety
///
/// Caller must ensure `cur_a`/`cur_b` point within valid slices and
/// `end_ptr` marks the end of the slice. Both accumulators must be initialized.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
unsafe fn jaccard_4acc_remainder(
    mut cur_a: *const f32,
    mut cur_b: *const f32,
    end_ptr: *const f32,
    inter_acc: &mut std::arch::x86_64::__m512,
    union_acc: &mut std::arch::x86_64::__m512,
) {
    use std::arch::x86_64::*;

    // Process remaining 16-element full chunks (up to 3 iterations)
    while cur_a.add(16) <= end_ptr {
        // SAFETY: Loop guard ensures 16 elements remain
        let va = _mm512_loadu_ps(cur_a);
        let vb = _mm512_loadu_ps(cur_b);
        *inter_acc = _mm512_add_ps(*inter_acc, _mm512_min_ps(va, vb));
        *union_acc = _mm512_add_ps(*union_acc, _mm512_max_ps(va, vb));
        cur_a = cur_a.add(16);
        cur_b = cur_b.add(16);
    }

    // Final masked chunk for 1..15 remaining elements
    let remaining = end_ptr.offset_from(cur_a) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=15, mask computed without overflow
        let mask: __mmask16 = ((1u32 << remaining) - 1) as u16;
        let va = _mm512_maskz_loadu_ps(mask, cur_a);
        let vb = _mm512_maskz_loadu_ps(mask, cur_b);
        *inter_acc = _mm512_add_ps(*inter_acc, _mm512_min_ps(va, vb));
        *union_acc = _mm512_add_ps(*union_acc, _mm512_max_ps(va, vb));
    }
}

// =============================================================================
// Binary Hamming Distance (packed u64)
// =============================================================================

/// AVX-512 binary Hamming distance on packed u64 vectors.
///
/// Uses 512-bit XOR to compute differing bits, then extracts u64 lanes
/// and counts set bits via scalar `count_ones()`. This approach avoids
/// relying on `_mm512_popcnt_epi64` (AVX512-VPOPCNTDQ), which requires
/// Ice Lake+ / Zen4+ and is not guaranteed by AVX-512F alone.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
pub(crate) unsafe fn hamming_binary_avx512(a: &[u64], b: &[u64]) -> u32 {
    // SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
    // - `_mm512_loadu_si512` handles unaligned loads safely
    // - Pointer arithmetic stays within bounds: loop guard `i + 8 <= len`
    // - `_mm512_storeu_si512` writes to a stack-allocated array of known size
    // - Both slices have equal length (caller's responsibility via public API assert)
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr().cast::<i64>();
    let b_ptr = b.as_ptr().cast::<i64>();
    let mut total: u64 = 0;
    let mut i = 0;

    // Process 8 u64 per iteration (512 bits / 64 bits = 8 elements)
    while i + 8 <= len {
        // SAFETY: i + 8 <= len guarantees 8 i64 elements are readable from both pointers
        let va = _mm512_loadu_si512(a_ptr.add(i).cast());
        let vb = _mm512_loadu_si512(b_ptr.add(i).cast());
        let xor = _mm512_xor_si512(va, vb);

        // Extract 8 u64 values and count_ones individually.
        // _mm512_popcnt_epi64 (AVX512-VPOPCNTDQ) is not available at MSRV 1.83
        // and requires a separate CPU feature beyond AVX-512F.
        let mut xor_arr = [0u64; 8];
        // SAFETY: xor_arr is 8 u64 = 64 bytes = exactly __m512i width
        _mm512_storeu_si512(xor_arr.as_mut_ptr().cast(), xor);
        for val in &xor_arr {
            total += u64::from(val.count_ones());
        }

        i += 8;
    }

    // Scalar remainder for < 8 trailing u64 elements
    for j in i..len {
        total += u64::from((a[j] ^ b[j]).count_ones());
    }

    // Reason: max total = len * 64; for any practical binary vector dimension
    // (len <= 67_108_864 words = 4 billion bits) this fits in u32.
    total as u32
}

/// AVX-512 VPOPCNTDQ binary Hamming distance for packed u64 vectors.
///
/// Uses native `_mm512_popcnt_epi64` for hardware-accelerated 64-bit popcount,
/// eliminating the extract+scalar loop used by [`hamming_binary_avx512`].
/// Available on Ice Lake (client), Cascade Lake (server), and Zen4+ CPUs.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F **and** AVX-512 VPOPCNTDQ (enforced by `#[target_feature]`
///   and runtime detection via `has_avx512vpopcntdq()`)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512vpopcntdq")]
#[inline]
pub(crate) unsafe fn hamming_binary_avx512_vpopcntdq(a: &[u64], b: &[u64]) -> u32 {
    // SAFETY: This function is only called after runtime detection confirms
    // both AVX-512F and AVX-512 VPOPCNTDQ.
    // - `_mm512_loadu_si512` handles unaligned loads safely.
    // - Loop guard `i + 8 <= len` ensures pointer arithmetic stays in bounds.
    // - `_mm512_popcnt_epi64` operates entirely within registers.
    // - `_mm512_reduce_add_epi64` horizontal sum within register.
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr().cast::<i64>();
    let b_ptr = b.as_ptr().cast::<i64>();
    let mut acc = _mm512_setzero_si512();
    let mut i = 0;

    // Process 8 u64 per iteration (512 bits / 64 bits = 8 elements)
    while i + 8 <= len {
        // SAFETY: i + 8 <= len guarantees 8 i64 elements are readable from both pointers
        let va = _mm512_loadu_si512(a_ptr.add(i).cast());
        let vb = _mm512_loadu_si512(b_ptr.add(i).cast());
        let xor = _mm512_xor_si512(va, vb);
        // Native 64-bit popcount per lane — no extract+scalar loop needed
        let popcnt = _mm512_popcnt_epi64(xor);
        acc = _mm512_add_epi64(acc, popcnt);
        i += 8;
    }

    // Horizontal sum of 8 i64 lanes
    let mut total = _mm512_reduce_add_epi64(acc) as u64;

    // Scalar remainder for < 8 trailing u64 elements
    for j in i..len {
        total += u64::from((a[j] ^ b[j]).count_ones());
    }

    // Reason: max total = len * 64; for any practical binary vector dimension
    // (len <= 67_108_864 words = 4 billion bits) this fits in u32.
    total as u32
}
