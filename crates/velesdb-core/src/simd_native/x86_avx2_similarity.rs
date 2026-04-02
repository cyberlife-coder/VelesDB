//! AVX2+FMA similarity kernel implementations for x86_64.
//!
//! Contains hand-tuned AVX2 SIMD kernels for cosine similarity (fused),
//! Hamming distance, and Jaccard similarity.
//!
//! All functions require runtime AVX2+FMA detection before calling.
//! Dispatch is handled by `dispatch.rs` after `simd_level()` confirms support.

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

use super::reduction::hsum_avx256;
use super::scalar::cosine_finish_fast;

// =============================================================================
// Cosine Similarity (Fused) — shared remainder helper
// =============================================================================

/// Processes the remainder elements after a cosine main loop: 8-wide vectorized
/// chunks then a scalar tail, followed by the final cosine computation.
///
/// Shared by both 2-acc and 4-acc cosine variants to eliminate duplication.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2+FMA (enforced by `#[target_feature]`)
/// - `a_ptr..end_ptr` and matching `b_ptr` range are valid readable memory
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
unsafe fn cosine_avx2_remainder(
    mut a_ptr: *const f32,
    mut b_ptr: *const f32,
    end_ptr: *const f32,
    mut dot_acc: std::arch::x86_64::__m256,
    mut na_acc: std::arch::x86_64::__m256,
    mut nb_acc: std::arch::x86_64::__m256,
) -> f32 {
    // SAFETY: AVX2+FMA guaranteed by #[target_feature]; unaligned loads are safe.
    // Loop guards ensure pointer arithmetic stays within the original slice bounds.
    use std::arch::x86_64::*;

    // Vectorized 8-wide remainder to reduce scalar tail to at most 7 elements
    while a_ptr.add(8) <= end_ptr {
        let va = _mm256_loadu_ps(a_ptr);
        let vb = _mm256_loadu_ps(b_ptr);
        dot_acc = _mm256_fmadd_ps(va, vb, dot_acc);
        na_acc = _mm256_fmadd_ps(va, va, na_acc);
        nb_acc = _mm256_fmadd_ps(vb, vb, nb_acc);
        a_ptr = a_ptr.add(8);
        b_ptr = b_ptr.add(8);
    }

    let mut dot = hsum_avx256(dot_acc);
    let mut norm_a_sq = hsum_avx256(na_acc);
    let mut norm_b_sq = hsum_avx256(nb_acc);

    // Scalar tail for 0-7 remaining elements
    while a_ptr < end_ptr {
        // SAFETY: Loop guard ensures pointers are within slice bounds.
        let x = *a_ptr;
        let y = *b_ptr;
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }

    cosine_finish_fast(dot, norm_a_sq, norm_b_sq)
}

// =============================================================================
// Cosine Similarity (Fused)
// =============================================================================

/// AVX2 fused cosine similarity with 2 accumulators for medium-sized vectors.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn cosine_fused_avx2_2acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 16 * 16);
    let end_ptr = a.as_ptr().add(len);

    let mut dot0 = _mm256_setzero_ps();
    let mut dot1 = _mm256_setzero_ps();
    let mut na0 = _mm256_setzero_ps();
    let mut na1 = _mm256_setzero_ps();
    let mut nb0 = _mm256_setzero_ps();
    let mut nb1 = _mm256_setzero_ps();

    while a_ptr < end_main {
        let va0 = _mm256_loadu_ps(a_ptr);
        let vb0 = _mm256_loadu_ps(b_ptr);
        dot0 = _mm256_fmadd_ps(va0, vb0, dot0);
        na0 = _mm256_fmadd_ps(va0, va0, na0);
        nb0 = _mm256_fmadd_ps(vb0, vb0, nb0);

        let va1 = _mm256_loadu_ps(a_ptr.add(8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(8));
        dot1 = _mm256_fmadd_ps(va1, vb1, dot1);
        na1 = _mm256_fmadd_ps(va1, va1, na1);
        nb1 = _mm256_fmadd_ps(vb1, vb1, nb1);

        a_ptr = a_ptr.add(16);
        b_ptr = b_ptr.add(16);
    }

    let dot_acc = _mm256_add_ps(dot0, dot1);
    let na_acc = _mm256_add_ps(na0, na1);
    let nb_acc = _mm256_add_ps(nb0, nb1);

    cosine_avx2_remainder(a_ptr, b_ptr, end_ptr, dot_acc, na_acc, nb_acc)
}

/// 4-accumulator main loop for cosine: computes dot(a,b), norm(a), norm(b) in
/// a single pass with 4-way ILP unrolling (32 floats per iteration).
///
/// Returns `(dot_acc, na_acc, nb_acc, updated_a_ptr, updated_b_ptr)` with the
/// 4 accumulators already reduced to 1 each via binary-tree addition.
///
/// # Safety
///
/// Caller must ensure AVX2+FMA and that `a_ptr..end_main` (and matching `b_ptr`
/// range) are readable with length aligned to 32 floats.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
unsafe fn cosine_4acc_main_loop(
    mut a_ptr: *const f32,
    mut b_ptr: *const f32,
    end_main: *const f32,
) -> (
    std::arch::x86_64::__m256,
    std::arch::x86_64::__m256,
    std::arch::x86_64::__m256,
    *const f32,
    *const f32,
) {
    // SAFETY: AVX2+FMA guaranteed by #[target_feature]; unaligned loads are safe.
    // Loop guard `a_ptr < end_main` ensures 32 floats remain at each iteration.
    use std::arch::x86_64::*;
    let z = _mm256_setzero_ps();
    let (mut dot0, mut dot1, mut dot2, mut dot3) = (z, z, z, z);
    let (mut na0, mut na1, mut na2, mut na3) = (z, z, z, z);
    let (mut nb0, mut nb1, mut nb2, mut nb3) = (z, z, z, z);

    while a_ptr < end_main {
        let (va0, vb0) = (_mm256_loadu_ps(a_ptr), _mm256_loadu_ps(b_ptr));
        dot0 = _mm256_fmadd_ps(va0, vb0, dot0);
        na0 = _mm256_fmadd_ps(va0, va0, na0);
        nb0 = _mm256_fmadd_ps(vb0, vb0, nb0);

        let (va1, vb1) = (_mm256_loadu_ps(a_ptr.add(8)), _mm256_loadu_ps(b_ptr.add(8)));
        dot1 = _mm256_fmadd_ps(va1, vb1, dot1);
        na1 = _mm256_fmadd_ps(va1, va1, na1);
        nb1 = _mm256_fmadd_ps(vb1, vb1, nb1);

        let (va2, vb2) = (
            _mm256_loadu_ps(a_ptr.add(16)),
            _mm256_loadu_ps(b_ptr.add(16)),
        );
        dot2 = _mm256_fmadd_ps(va2, vb2, dot2);
        na2 = _mm256_fmadd_ps(va2, va2, na2);
        nb2 = _mm256_fmadd_ps(vb2, vb2, nb2);

        let (va3, vb3) = (
            _mm256_loadu_ps(a_ptr.add(24)),
            _mm256_loadu_ps(b_ptr.add(24)),
        );
        dot3 = _mm256_fmadd_ps(va3, vb3, dot3);
        na3 = _mm256_fmadd_ps(va3, va3, na3);
        nb3 = _mm256_fmadd_ps(vb3, vb3, nb3);

        a_ptr = a_ptr.add(32);
        b_ptr = b_ptr.add(32);
    }

    let dot_acc = _mm256_add_ps(_mm256_add_ps(dot0, dot1), _mm256_add_ps(dot2, dot3));
    let na_acc = _mm256_add_ps(_mm256_add_ps(na0, na1), _mm256_add_ps(na2, na3));
    let nb_acc = _mm256_add_ps(_mm256_add_ps(nb0, nb1), _mm256_add_ps(nb2, nb3));

    (dot_acc, na_acc, nb_acc, a_ptr, b_ptr)
}

/// AVX2 fused cosine similarity - computes dot product and norms in single SIMD pass.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn cosine_fused_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: runtime feature detection ensures AVX2+FMA; loads are unaligned-safe.
    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 32 * 32);
    let end_ptr = a.as_ptr().add(len);

    let (dot_acc, na_acc, nb_acc, a_p, b_p) = cosine_4acc_main_loop(a_ptr, b_ptr, end_main);

    cosine_avx2_remainder(a_p, b_p, end_ptr, dot_acc, na_acc, nb_acc)
}

// =============================================================================
// Hamming & Jaccard
// =============================================================================

/// AVX2 Hamming with 4 FP-domain accumulators (no cross-domain penalty).
///
/// Stays entirely in the SIMD floating-point pipeline to avoid the ~1 cycle
/// bypass penalty per FP→INT domain crossing that occurs on Intel cores.
/// Uses `cmp_ps → xor_ps → and_ps(1.0) → add_ps` instead of the previous
/// `cmp_ps → castps_si256 → xor_si256 → srli_epi32 → add_epi32` path.
///
/// The key trick: `and_ps(0xFFFFFFFF, 1.0_bits) = 1.0` and
/// `and_ps(0x00000000, 1.0_bits) = 0.0`, converting boolean masks to 1.0/0.0
/// without leaving the FP domain.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn hamming_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: Runtime feature detection ensures AVX2; all loads are unaligned-safe.
    // Pointer arithmetic stays in bounds: loop guards enforce `a_ptr + N <= end_ptr`.
    use std::arch::x86_64::*;

    let len = a.len();
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 32 * 32);
    let end_ptr = a.as_ptr().add(len);

    let threshold = _mm256_set1_ps(0.5);
    // Broadcast 1.0f32 for mask-to-float conversion via bitwise AND
    let one_vec = _mm256_set1_ps(1.0);
    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    let mut acc2 = _mm256_setzero_ps();
    let mut acc3 = _mm256_setzero_ps();

    while a_ptr < end_main {
        // SAFETY: Loop guard ensures 32 elements remain; unaligned loads are safe.
        acc0 = hamming_avx2_fp_acc(a_ptr, b_ptr, threshold, one_vec, acc0);
        acc1 = hamming_avx2_fp_acc(a_ptr.add(8), b_ptr.add(8), threshold, one_vec, acc1);
        acc2 = hamming_avx2_fp_acc(a_ptr.add(16), b_ptr.add(16), threshold, one_vec, acc2);
        acc3 = hamming_avx2_fp_acc(a_ptr.add(24), b_ptr.add(24), threshold, one_vec, acc3);
        a_ptr = a_ptr.add(32);
        b_ptr = b_ptr.add(32);
    }

    // Binary-tree reduce 4 accumulators, then 8-wide remainder
    let acc01 = _mm256_add_ps(acc0, acc1);
    let acc23 = _mm256_add_ps(acc2, acc3);
    let mut acc = _mm256_add_ps(acc01, acc23);

    while a_ptr.add(8) <= end_ptr {
        // SAFETY: Guard ensures 8 elements remain.
        acc = hamming_avx2_fp_acc(a_ptr, b_ptr, threshold, one_vec, acc);
        a_ptr = a_ptr.add(8);
        b_ptr = b_ptr.add(8);
    }

    let mut diff_count = hsum_avx256(acc);

    // Scalar tail for 0-7 remaining elements
    while a_ptr < end_ptr {
        // SAFETY: Loop guard ensures pointers are within slice bounds.
        if (*a_ptr > 0.5) != (*b_ptr > 0.5) {
            diff_count += 1.0;
        }
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }

    diff_count
}

/// Compares 8 f32 lanes against threshold, XORs results, and accumulates
/// the 0/1 diff count into an `__m256` FP accumulator (no domain crossing).
///
/// The AND with `1.0f32` converts the all-ones mask (`0xFFFFFFFF`) to `1.0`
/// and the all-zeros mask (`0x00000000`) to `0.0`, staying in the FP pipeline.
///
/// # Safety
///
/// Caller must ensure AVX2 is available and `a_ptr`/`b_ptr` point to at
/// least 8 readable f32 elements.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn hamming_avx2_fp_acc(
    a_ptr: *const f32,
    b_ptr: *const f32,
    threshold: std::arch::x86_64::__m256,
    one_vec: std::arch::x86_64::__m256,
    acc: std::arch::x86_64::__m256,
) -> std::arch::x86_64::__m256 {
    // SAFETY: AVX2 guaranteed by #[target_feature]; no memory beyond the 8-lane read.
    use std::arch::x86_64::*;
    let mask_a = _mm256_cmp_ps(_mm256_loadu_ps(a_ptr), threshold, _CMP_GT_OQ);
    let mask_b = _mm256_cmp_ps(_mm256_loadu_ps(b_ptr), threshold, _CMP_GT_OQ);
    // XOR in FP domain: 0xFFFFFFFF if different, 0x00000000 if same
    let diff = _mm256_xor_ps(mask_a, mask_b);
    // AND with 1.0f32 bits: 0xFFFFFFFF & 0x3F800000 = 0x3F800000 = 1.0
    //                        0x00000000 & 0x3F800000 = 0x00000000 = 0.0
    let ones = _mm256_and_ps(diff, one_vec);
    _mm256_add_ps(acc, ones)
}

// =============================================================================
// Binary Hamming (packed u64)
// =============================================================================

/// AVX2 binary Hamming distance for packed u64 vectors.
///
/// XORs 4 u64 words per iteration (256 bits), extracts individual u64 values,
/// and uses scalar `count_ones()` (compiles to `popcnt`). AVX2 lacks native
/// 64-bit popcount, so extract-and-scalar is the best portable approach.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2 (enforced by `#[target_feature]` and runtime detection)
/// - `a.len() == b.len()` (enforced by public API assert)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn hamming_binary_avx2(a: &[u64], b: &[u64]) -> u32 {
    // SAFETY: AVX2 confirmed by #[target_feature] + runtime detection.
    // - `_mm256_loadu_si256` handles unaligned loads safely.
    // - Loop guard `i + 4 <= len` ensures pointer arithmetic stays in bounds.
    // - `_mm256_extract_epi64` reads from a register, not memory.
    use std::arch::x86_64::*;

    let len = a.len();
    let mut total: u32 = 0;
    let mut i = 0;

    // Process 4 u64 (256 bits) per iteration
    while i + 4 <= len {
        // SAFETY: i + 4 <= len guarantees 4 u64 elements are readable from both slices
        let va = _mm256_loadu_si256(a.as_ptr().add(i).cast());
        let vb = _mm256_loadu_si256(b.as_ptr().add(i).cast());
        let xor = _mm256_xor_si256(va, vb);

        // Extract individual u64s and popcount (AVX2 has no native 64-bit popcount)
        let x0 = _mm256_extract_epi64(xor, 0) as u64;
        let x1 = _mm256_extract_epi64(xor, 1) as u64;
        let x2 = _mm256_extract_epi64(xor, 2) as u64;
        let x3 = _mm256_extract_epi64(xor, 3) as u64;
        total += x0.count_ones() + x1.count_ones() + x2.count_ones() + x3.count_ones();
        i += 4;
    }

    // Scalar tail for < 4 trailing u64 elements
    while i < len {
        total += (a[i] ^ b[i]).count_ones();
        i += 1;
    }

    total
}

// =============================================================================
// Jaccard Similarity
// =============================================================================

/// Processes Jaccard remainder elements: 8-wide vectorized chunks with min/max,
/// then a scalar tail, followed by the final intersection/union ratio.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX2 (enforced by `#[target_feature]`)
/// - `a_ptr..end_ptr` and matching `b_ptr` range are valid readable memory
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn jaccard_avx2_remainder(
    mut a_ptr: *const f32,
    mut b_ptr: *const f32,
    end_ptr: *const f32,
    mut acc_inter: std::arch::x86_64::__m256,
    mut acc_union: std::arch::x86_64::__m256,
) -> f32 {
    // SAFETY: AVX2 guaranteed by #[target_feature]; unaligned loads are safe.
    // Loop guards ensure pointer arithmetic stays within the original slice bounds.
    use std::arch::x86_64::*;

    // Vectorized 8-wide remainder to reduce scalar tail to at most 7 elements
    while a_ptr.add(8) <= end_ptr {
        let va = _mm256_loadu_ps(a_ptr);
        let vb = _mm256_loadu_ps(b_ptr);
        acc_inter = _mm256_add_ps(acc_inter, _mm256_min_ps(va, vb));
        acc_union = _mm256_add_ps(acc_union, _mm256_max_ps(va, vb));
        a_ptr = a_ptr.add(8);
        b_ptr = b_ptr.add(8);
    }

    let mut inter_sum = hsum_avx256(acc_inter);
    let mut union_sum = hsum_avx256(acc_union);

    // Scalar tail for 0-7 remaining elements
    while a_ptr < end_ptr {
        // SAFETY: Loop guard ensures pointers are within slice bounds.
        let x = *a_ptr;
        let y = *b_ptr;
        inter_sum += x.min(y);
        union_sum += x.max(y);
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }

    if union_sum == 0.0 {
        1.0
    } else {
        inter_sum / union_sum
    }
}

/// AVX2 Jaccard with 4 accumulators for ILP optimization (EPIC-052/US-008).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn jaccard_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 32 * 32);
    let end_ptr = a.as_ptr().add(len);

    let mut inter0 = _mm256_setzero_ps();
    let mut inter1 = _mm256_setzero_ps();
    let mut inter2 = _mm256_setzero_ps();
    let mut inter3 = _mm256_setzero_ps();
    let mut union0 = _mm256_setzero_ps();
    let mut union1 = _mm256_setzero_ps();
    let mut union2 = _mm256_setzero_ps();
    let mut union3 = _mm256_setzero_ps();

    while a_ptr < end_main {
        let va0 = _mm256_loadu_ps(a_ptr);
        let vb0 = _mm256_loadu_ps(b_ptr);
        inter0 = _mm256_add_ps(inter0, _mm256_min_ps(va0, vb0));
        union0 = _mm256_add_ps(union0, _mm256_max_ps(va0, vb0));

        let va1 = _mm256_loadu_ps(a_ptr.add(8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(8));
        inter1 = _mm256_add_ps(inter1, _mm256_min_ps(va1, vb1));
        union1 = _mm256_add_ps(union1, _mm256_max_ps(va1, vb1));

        let va2 = _mm256_loadu_ps(a_ptr.add(16));
        let vb2 = _mm256_loadu_ps(b_ptr.add(16));
        inter2 = _mm256_add_ps(inter2, _mm256_min_ps(va2, vb2));
        union2 = _mm256_add_ps(union2, _mm256_max_ps(va2, vb2));

        let va3 = _mm256_loadu_ps(a_ptr.add(24));
        let vb3 = _mm256_loadu_ps(b_ptr.add(24));
        inter3 = _mm256_add_ps(inter3, _mm256_min_ps(va3, vb3));
        union3 = _mm256_add_ps(union3, _mm256_max_ps(va3, vb3));

        a_ptr = a_ptr.add(32);
        b_ptr = b_ptr.add(32);
    }

    let inter01 = _mm256_add_ps(inter0, inter1);
    let inter23 = _mm256_add_ps(inter2, inter3);
    let acc_inter = _mm256_add_ps(inter01, inter23);

    let union01 = _mm256_add_ps(union0, union1);
    let union23 = _mm256_add_ps(union2, union3);
    let acc_union = _mm256_add_ps(union01, union23);

    jaccard_avx2_remainder(a_ptr, b_ptr, end_ptr, acc_inter, acc_union)
}
