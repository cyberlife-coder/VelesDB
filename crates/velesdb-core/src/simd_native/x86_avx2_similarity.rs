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

    let hs = |v: __m256| {
        let hi = _mm256_extractf128_ps(v, 1);
        let lo = _mm256_castps256_ps128(v);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        _mm_cvtss_f32(_mm_add_ss(sums, shuf2))
    };

    let mut dot = hs(dot_acc);
    let mut norm_a_sq = hs(na_acc);
    let mut norm_b_sq = hs(nb_acc);

    while a_ptr < end_ptr {
        let x = *a_ptr;
        let y = *b_ptr;
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }

    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();
    if norm_a < f32::EPSILON || norm_b < f32::EPSILON {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

/// AVX2 fused cosine similarity - computes dot product and norms in single SIMD pass.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
pub(crate) unsafe fn cosine_fused_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: runtime feature detection ensures AVX2+FMA; loads are unaligned-safe.
    use std::arch::x86_64::*;

    let len = a.len();
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 32 * 32);
    let end_ptr = a.as_ptr().add(len);

    let mut dot0 = _mm256_setzero_ps();
    let mut dot1 = _mm256_setzero_ps();
    let mut dot2 = _mm256_setzero_ps();
    let mut dot3 = _mm256_setzero_ps();
    let mut na0 = _mm256_setzero_ps();
    let mut na1 = _mm256_setzero_ps();
    let mut na2 = _mm256_setzero_ps();
    let mut na3 = _mm256_setzero_ps();
    let mut nb0 = _mm256_setzero_ps();
    let mut nb1 = _mm256_setzero_ps();
    let mut nb2 = _mm256_setzero_ps();
    let mut nb3 = _mm256_setzero_ps();

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

        let va2 = _mm256_loadu_ps(a_ptr.add(16));
        let vb2 = _mm256_loadu_ps(b_ptr.add(16));
        dot2 = _mm256_fmadd_ps(va2, vb2, dot2);
        na2 = _mm256_fmadd_ps(va2, va2, na2);
        nb2 = _mm256_fmadd_ps(vb2, vb2, nb2);

        let va3 = _mm256_loadu_ps(a_ptr.add(24));
        let vb3 = _mm256_loadu_ps(b_ptr.add(24));
        dot3 = _mm256_fmadd_ps(va3, vb3, dot3);
        na3 = _mm256_fmadd_ps(va3, va3, na3);
        nb3 = _mm256_fmadd_ps(vb3, vb3, nb3);

        a_ptr = a_ptr.add(32);
        b_ptr = b_ptr.add(32);
    }

    let dot01 = _mm256_add_ps(dot0, dot1);
    let dot23 = _mm256_add_ps(dot2, dot3);
    let dot_acc = _mm256_add_ps(dot01, dot23);

    let na01 = _mm256_add_ps(na0, na1);
    let na23 = _mm256_add_ps(na2, na3);
    let na_acc = _mm256_add_ps(na01, na23);

    let nb01 = _mm256_add_ps(nb0, nb1);
    let nb23 = _mm256_add_ps(nb2, nb3);
    let nb_acc = _mm256_add_ps(nb01, nb23);

    let hs = |v: __m256| {
        let hi = _mm256_extractf128_ps(v, 1);
        let lo = _mm256_castps256_ps128(v);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        _mm_cvtss_f32(_mm_add_ss(sums, shuf2))
    };

    let mut dot = hs(dot_acc);
    let mut norm_a_sq = hs(na_acc);
    let mut norm_b_sq = hs(nb_acc);

    while a_ptr < end_ptr {
        let x = *a_ptr;
        let y = *b_ptr;
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }

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

/// AVX2 Hamming with 4x unrolling for ILP optimization (EPIC-052/US-008).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn hamming_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    let end_main = a.as_ptr().add(len / 32 * 32);
    let end_ptr = a.as_ptr().add(len);

    let threshold = _mm256_set1_ps(0.5);
    let mut diff_count: u64 = 0;

    while a_ptr < end_main {
        let va0 = _mm256_loadu_ps(a_ptr);
        let vb0 = _mm256_loadu_ps(b_ptr);
        let cmp_a0 = _mm256_cmp_ps(va0, threshold, _CMP_GT_OQ);
        let cmp_b0 = _mm256_cmp_ps(vb0, threshold, _CMP_GT_OQ);
        let diff0 = _mm256_xor_ps(cmp_a0, cmp_b0);
        diff_count += _mm256_movemask_ps(diff0).count_ones() as u64;

        let va1 = _mm256_loadu_ps(a_ptr.add(8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(8));
        let cmp_a1 = _mm256_cmp_ps(va1, threshold, _CMP_GT_OQ);
        let cmp_b1 = _mm256_cmp_ps(vb1, threshold, _CMP_GT_OQ);
        let diff1 = _mm256_xor_ps(cmp_a1, cmp_b1);
        diff_count += _mm256_movemask_ps(diff1).count_ones() as u64;

        let va2 = _mm256_loadu_ps(a_ptr.add(16));
        let vb2 = _mm256_loadu_ps(b_ptr.add(16));
        let cmp_a2 = _mm256_cmp_ps(va2, threshold, _CMP_GT_OQ);
        let cmp_b2 = _mm256_cmp_ps(vb2, threshold, _CMP_GT_OQ);
        let diff2 = _mm256_xor_ps(cmp_a2, cmp_b2);
        diff_count += _mm256_movemask_ps(diff2).count_ones() as u64;

        let va3 = _mm256_loadu_ps(a_ptr.add(24));
        let vb3 = _mm256_loadu_ps(b_ptr.add(24));
        let cmp_a3 = _mm256_cmp_ps(va3, threshold, _CMP_GT_OQ);
        let cmp_b3 = _mm256_cmp_ps(vb3, threshold, _CMP_GT_OQ);
        let diff3 = _mm256_xor_ps(cmp_a3, cmp_b3);
        diff_count += _mm256_movemask_ps(diff3).count_ones() as u64;

        a_ptr = a_ptr.add(32);
        b_ptr = b_ptr.add(32);
    }

    // Handle remainder with scalar
    while a_ptr < end_ptr {
        let x = *a_ptr > 0.5;
        let y = *b_ptr > 0.5;
        if x != y {
            diff_count += 1;
        }
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }

    diff_count as f32
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

    let mut inter_sum = hsum256_ps(acc_inter);
    let mut union_sum = hsum256_ps(acc_union);

    while a_ptr < end_ptr {
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

/// Horizontal sum helper for AVX2 256-bit vector.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn hsum256_ps(v: std::arch::x86_64::__m256) -> f32 {
    use std::arch::x86_64::*;
    let low = _mm256_castps256_ps128(v);
    let high = _mm256_extractf128_ps(v, 1);
    let sum128 = _mm_add_ps(low, high);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let result = _mm_add_ss(sums, shuf2);
    _mm_cvtss_f32(result)
}
