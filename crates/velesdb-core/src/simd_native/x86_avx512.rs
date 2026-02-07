//! AVX-512F kernel implementations for x86_64.
//!
//! Contains hand-tuned AVX-512 SIMD kernels for dot product, squared L2,
//! cosine similarity, Hamming distance, and Jaccard similarity.
//!
//! All functions require runtime AVX-512F detection before calling.

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

use super::scalar;

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
        // Create mask: first `remainder` bits set to 1
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

    _mm512_reduce_add_ps(sum)
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
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 64 * 64);
    let end_ptr = a.as_ptr().add(len);

    let mut acc0 = _mm512_setzero_ps();
    let mut acc1 = _mm512_setzero_ps();
    let mut acc2 = _mm512_setzero_ps();
    let mut acc3 = _mm512_setzero_ps();

    // Main loop: process 64 elements at a time using pointer arithmetic
    while a_ptr < end_main {
        let va0 = _mm512_loadu_ps(a_ptr);
        let vb0 = _mm512_loadu_ps(b_ptr);
        acc0 = _mm512_fmadd_ps(va0, vb0, acc0);

        let va1 = _mm512_loadu_ps(a_ptr.add(16));
        let vb1 = _mm512_loadu_ps(b_ptr.add(16));
        acc1 = _mm512_fmadd_ps(va1, vb1, acc1);

        let va2 = _mm512_loadu_ps(a_ptr.add(32));
        let vb2 = _mm512_loadu_ps(b_ptr.add(32));
        acc2 = _mm512_fmadd_ps(va2, vb2, acc2);

        let va3 = _mm512_loadu_ps(a_ptr.add(48));
        let vb3 = _mm512_loadu_ps(b_ptr.add(48));
        acc3 = _mm512_fmadd_ps(va3, vb3, acc3);

        a_ptr = a_ptr.add(64);
        b_ptr = b_ptr.add(64);
    }

    // Combine all 4 accumulators into one, then continue with single acc
    acc0 = _mm512_add_ps(acc0, acc1);
    acc2 = _mm512_add_ps(acc2, acc3);
    acc0 = _mm512_add_ps(acc0, acc2);

    // Process remaining 16-element chunks with same accumulator
    while a_ptr.add(16) <= end_ptr {
        let va = _mm512_loadu_ps(a_ptr);
        let vb = _mm512_loadu_ps(b_ptr);
        acc0 = _mm512_fmadd_ps(va, vb, acc0);
        a_ptr = a_ptr.add(16);
        b_ptr = b_ptr.add(16);
    }

    // Final masked chunk if any
    let remaining = end_ptr.offset_from(a_ptr) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=16, mask computed without overflow
        let mask: __mmask16 = if remaining == 16 {
            !0
        } else {
            ((1u32 << remaining) - 1) as u16
        };
        let va = _mm512_maskz_loadu_ps(mask, a_ptr);
        let vb = _mm512_maskz_loadu_ps(mask, b_ptr);
        acc0 = _mm512_fmadd_ps(va, vb, acc0);
    }

    _mm512_reduce_add_ps(acc0)
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
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 64 * 64);
    let end_ptr = a.as_ptr().add(len);

    let mut acc0 = _mm512_setzero_ps();
    let mut acc1 = _mm512_setzero_ps();
    let mut acc2 = _mm512_setzero_ps();
    let mut acc3 = _mm512_setzero_ps();

    // Main loop: process 64 elements at a time (4×16)
    while a_ptr < end_main {
        let va0 = _mm512_loadu_ps(a_ptr);
        let vb0 = _mm512_loadu_ps(b_ptr);
        let diff0 = _mm512_sub_ps(va0, vb0);
        acc0 = _mm512_fmadd_ps(diff0, diff0, acc0);

        let va1 = _mm512_loadu_ps(a_ptr.add(16));
        let vb1 = _mm512_loadu_ps(b_ptr.add(16));
        let diff1 = _mm512_sub_ps(va1, vb1);
        acc1 = _mm512_fmadd_ps(diff1, diff1, acc1);

        let va2 = _mm512_loadu_ps(a_ptr.add(32));
        let vb2 = _mm512_loadu_ps(b_ptr.add(32));
        let diff2 = _mm512_sub_ps(va2, vb2);
        acc2 = _mm512_fmadd_ps(diff2, diff2, acc2);

        let va3 = _mm512_loadu_ps(a_ptr.add(48));
        let vb3 = _mm512_loadu_ps(b_ptr.add(48));
        let diff3 = _mm512_sub_ps(va3, vb3);
        acc3 = _mm512_fmadd_ps(diff3, diff3, acc3);

        a_ptr = a_ptr.add(64);
        b_ptr = b_ptr.add(64);
    }

    // Combine all 4 accumulators
    acc0 = _mm512_add_ps(acc0, acc1);
    acc2 = _mm512_add_ps(acc2, acc3);
    acc0 = _mm512_add_ps(acc0, acc2);

    // Process remaining 16-element chunks
    while a_ptr.add(16) <= end_ptr {
        let va = _mm512_loadu_ps(a_ptr);
        let vb = _mm512_loadu_ps(b_ptr);
        let diff = _mm512_sub_ps(va, vb);
        acc0 = _mm512_fmadd_ps(diff, diff, acc0);
        a_ptr = a_ptr.add(16);
        b_ptr = b_ptr.add(16);
    }

    // Final masked chunk if any
    let remaining = end_ptr.offset_from(a_ptr) as usize;
    if remaining > 0 {
        // SAFETY: remaining is in 1..=16, mask computed without overflow
        let mask: __mmask16 = if remaining == 16 {
            !0
        } else {
            ((1u32 << remaining) - 1) as u16
        };
        let va = _mm512_maskz_loadu_ps(mask, a_ptr);
        let vb = _mm512_maskz_loadu_ps(mask, b_ptr);
        let diff = _mm512_sub_ps(va, vb);
        acc0 = _mm512_fmadd_ps(diff, diff, acc0);
    }

    _mm512_reduce_add_ps(acc0)
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

    _mm512_reduce_add_ps(sum)
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

    let dot = _mm512_reduce_add_ps(_mm512_add_ps(dot0, dot1));
    let norm_a_sq = _mm512_reduce_add_ps(_mm512_add_ps(na0, na1));
    let norm_b_sq = _mm512_reduce_add_ps(_mm512_add_ps(nb0, nb1));

    // Use precise sqrt for accuracy (fast_rsqrt has ~0.2% error)
    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();
    if norm_a < f32::EPSILON || norm_b < f32::EPSILON {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
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
    let inter_sum = _mm512_reduce_add_ps(acc_inter);
    let union_sum = _mm512_reduce_add_ps(acc_union);

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
