//! Native SIMD intrinsics for maximum performance.
//!
//! This module provides hand-tuned SIMD implementations using `core::arch` intrinsics
//! for AVX-512, AVX2, and ARM NEON architectures.
//!
//! # Performance (based on arXiv research)
//!
//! - **AVX-512**: True 16-wide f32 operations
//! - **ARM NEON**: Native 128-bit SIMD for Apple Silicon/ARM64
//! - **Prefetch**: Software prefetching for cache optimization
//!
//! # References
//!
//! - arXiv:2505.07621 "Bang for the Buck: Vector Search on Cloud CPUs"
//! - arXiv:2502.18113 "Accelerating Graph Indexing for ANNS on Modern CPUs"

// Allow AVX-512 intrinsics even if MSRV is lower (runtime feature detection ensures safety)
#![allow(clippy::incompatible_msrv)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::similar_names)]

// =============================================================================
// Remainder Handling Macro (FLAG-005: Factorisation)
// =============================================================================

/// Macro for unrolled remainder sum computation (1-7 elements).
/// Generates optimal code for remainders 1-7 with 4→2→1 unrolling.
#[macro_export]
macro_rules! sum_remainder_unrolled_8 {
    ($a:expr, $b:expr, $base:expr, $remainder:expr, $result:expr) => {
        if $remainder >= 4 {
            $result += $a[$base] * $b[$base]
                + $a[$base + 1] * $b[$base + 1]
                + $a[$base + 2] * $b[$base + 2]
                + $a[$base + 3] * $b[$base + 3];
            if $remainder >= 5 {
                $result += $a[$base + 4] * $b[$base + 4];
            }
            if $remainder >= 6 {
                $result += $a[$base + 5] * $b[$base + 5];
            }
            if $remainder == 7 {
                $result += $a[$base + 6] * $b[$base + 6];
            }
        } else if $remainder >= 2 {
            $result += $a[$base] * $b[$base] + $a[$base + 1] * $b[$base + 1];
            if $remainder == 3 {
                $result += $a[$base + 2] * $b[$base + 2];
            }
        } else if $remainder == 1 {
            $result += $a[$base] * $b[$base];
        }
    };
}

/// Macro for unrolled squared L2 remainder (1-7 elements).
#[macro_export]
macro_rules! sum_squared_remainder_unrolled_8 {
    ($a:expr, $b:expr, $base:expr, $remainder:expr, $result:expr) => {
        if $remainder >= 4 {
            let d0 = $a[$base] - $b[$base];
            let d1 = $a[$base + 1] - $b[$base + 1];
            let d2 = $a[$base + 2] - $b[$base + 2];
            let d3 = $a[$base + 3] - $b[$base + 3];
            $result += d0 * d0 + d1 * d1 + d2 * d2 + d3 * d3;
            if $remainder >= 5 {
                let d4 = $a[$base + 4] - $b[$base + 4];
                $result += d4 * d4;
            }
            if $remainder >= 6 {
                let d5 = $a[$base + 5] - $b[$base + 5];
                $result += d5 * d5;
            }
            if $remainder == 7 {
                let d6 = $a[$base + 6] - $b[$base + 6];
                $result += d6 * d6;
            }
        } else if $remainder >= 2 {
            let d0 = $a[$base] - $b[$base];
            let d1 = $a[$base + 1] - $b[$base + 1];
            $result += d0 * d0 + d1 * d1;
            if $remainder == 3 {
                let d2 = $a[$base + 2] - $b[$base + 2];
                $result += d2 * d2;
            }
        } else if $remainder == 1 {
            let d = $a[$base] - $b[$base];
            $result += d * d;
        }
    };
}

// Re-export macros for internal use
#[allow(unused_imports)]
pub(crate) use sum_remainder_unrolled_8;
#[allow(unused_imports)]
pub(crate) use sum_squared_remainder_unrolled_8;

// =============================================================================
// AVX-512 Implementation (x86_64)
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
        // SAFETY: remainder is in 1..16, so mask is valid
        let mask: __mmask16 = (1_u16 << remainder) - 1;
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
        let mask: __mmask16 = (1_u16 << remaining) - 1;
        let va = _mm512_maskz_loadu_ps(mask, a_ptr);
        let vb = _mm512_maskz_loadu_ps(mask, b_ptr);
        acc0 = _mm512_fmadd_ps(va, vb, acc0);
    }

    _mm512_reduce_add_ps(acc0)
}

/// AVX-512 squared L2 distance.
///
/// # Safety
///
/// Same requirements as `dot_product_avx512`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
unsafe fn squared_l2_avx512(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: See dot_product_avx512 for detailed safety justification.
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

    // Handle remainder with masked load (EPIC-PERF-002)
    if remainder > 0 {
        let base = simd_len * 16;
        let mask: __mmask16 = (1_u16 << remainder) - 1;
        let va = _mm512_maskz_loadu_ps(mask, a_ptr.add(base));
        let vb = _mm512_maskz_loadu_ps(mask, b_ptr.add(base));
        let diff = _mm512_sub_ps(va, vb);
        sum = _mm512_fmadd_ps(diff, diff, sum);
    }

    _mm512_reduce_add_ps(sum)
}

/// AVX-512 cosine similarity for pre-normalized vectors.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(dead_code)] // Reserved for future direct use
unsafe fn cosine_normalized_avx512(a: &[f32], b: &[f32]) -> f32 {
    // For unit vectors: cos(θ) = a · b
    dot_product_avx512(a, b)
}

// =============================================================================
// AVX2 Implementation (x86_64 fallback)
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
                    if rr >= 4 {
                        result += a[rrbase] * b[rrbase]
                            + a[rrbase + 1] * b[rrbase + 1]
                            + a[rrbase + 2] * b[rrbase + 2]
                            + a[rrbase + 3] * b[rrbase + 3];
                        if rr >= 5 {
                            result += a[rrbase + 4] * b[rrbase + 4];
                        }
                        if rr >= 6 {
                            result += a[rrbase + 5] * b[rrbase + 5];
                        }
                        if rr == 7 {
                            result += a[rrbase + 6] * b[rrbase + 6];
                        }
                    } else if rr >= 2 {
                        result += a[rrbase] * b[rrbase] + a[rrbase + 1] * b[rrbase + 1];
                        if rr == 3 {
                            result += a[rrbase + 2] * b[rrbase + 2];
                        }
                    } else if rr == 1 {
                        result += a[rrbase] * b[rrbase];
                    }
                }
            } else if r >= 4 {
                result += a[rbase] * b[rbase]
                    + a[rbase + 1] * b[rbase + 1]
                    + a[rbase + 2] * b[rbase + 2]
                    + a[rbase + 3] * b[rbase + 3];
                if r >= 5 {
                    result += a[rbase + 4] * b[rbase + 4];
                }
                if r >= 6 {
                    result += a[rbase + 5] * b[rbase + 5];
                }
                if r >= 7 {
                    result += a[rbase + 6] * b[rbase + 6];
                }
            } else if r >= 2 {
                result += a[rbase] * b[rbase] + a[rbase + 1] * b[rbase + 1];
                if r == 3 {
                    result += a[rbase + 2] * b[rbase + 2];
                }
            } else if r == 1 {
                result += a[rbase] * b[rbase];
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
        if r >= 4 {
            result += a[base + 8] * b[base + 8]
                + a[base + 9] * b[base + 9]
                + a[base + 10] * b[base + 10]
                + a[base + 11] * b[base + 11];
            if r >= 5 {
                result += a[base + 12] * b[base + 12];
            }
            if r >= 6 {
                result += a[base + 13] * b[base + 13];
            }
            if r == 7 {
                result += a[base + 14] * b[base + 14];
            }
        } else if r >= 2 {
            result += a[base + 8] * b[base + 8] + a[base + 9] * b[base + 9];
            if r == 3 {
                result += a[base + 10] * b[base + 10];
            }
        } else if r == 1 {
            result += a[base + 8] * b[base + 8];
        }
    } else if remainder >= 4 {
        result += a[base] * b[base]
            + a[base + 1] * b[base + 1]
            + a[base + 2] * b[base + 2]
            + a[base + 3] * b[base + 3];
        if remainder >= 5 {
            result += a[base + 4] * b[base + 4];
        }
        if remainder >= 6 {
            result += a[base + 5] * b[base + 5];
        }
        if remainder == 7 {
            result += a[base + 6] * b[base + 6];
        }
    } else if remainder >= 2 {
        result += a[base] * b[base] + a[base + 1] * b[base + 1];
        if remainder == 3 {
            result += a[base + 2] * b[base + 2];
        }
    } else if remainder == 1 {
        result += a[base] * b[base];
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
    if remainder >= 4 {
        result += a[base] * b[base]
            + a[base + 1] * b[base + 1]
            + a[base + 2] * b[base + 2]
            + a[base + 3] * b[base + 3];
        if remainder >= 5 {
            result += a[base + 4] * b[base + 4];
        }
        if remainder >= 6 {
            result += a[base + 5] * b[base + 5];
        }
        if remainder == 7 {
            result += a[base + 6] * b[base + 6];
        }
    } else if remainder >= 2 {
        result += a[base] * b[base] + a[base + 1] * b[base + 1];
        if remainder == 3 {
            result += a[base + 2] * b[base + 2];
        }
    } else if remainder == 1 {
        result += a[base] * b[base];
    }

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
            if r >= 4 {
                result += a[rbase] * b[rbase]
                    + a[rbase + 1] * b[rbase + 1]
                    + a[rbase + 2] * b[rbase + 2]
                    + a[rbase + 3] * b[rbase + 3];
                if r >= 5 {
                    result += a[rbase + 4] * b[rbase + 4];
                }
                if r >= 6 {
                    result += a[rbase + 5] * b[rbase + 5];
                }
                if r == 7 {
                    result += a[rbase + 6] * b[rbase + 6];
                }
            } else if r >= 2 {
                result += a[rbase] * b[rbase] + a[rbase + 1] * b[rbase + 1];
                if r == 3 {
                    result += a[rbase + 2] * b[rbase + 2];
                }
            } else if r == 1 {
                result += a[rbase] * b[rbase];
            }
        }
    } else if remainder >= 4 {
        result += a[base] * b[base]
            + a[base + 1] * b[base + 1]
            + a[base + 2] * b[base + 2]
            + a[base + 3] * b[base + 3];
        if remainder >= 5 {
            result += a[base + 4] * b[base + 4];
        }
        if remainder >= 6 {
            result += a[base + 5] * b[base + 5];
        }
        if remainder >= 7 {
            result += a[base + 6] * b[base + 6];
        }
    } else if remainder >= 2 {
        result += a[base] * b[base] + a[base + 1] * b[base + 1];
        if remainder == 3 {
            result += a[base + 2] * b[base + 2];
        }
    } else if remainder == 1 {
        result += a[base] * b[base];
    }

    result
}

/// AVX2 squared L2 distance.
///
/// # Safety
///
/// Same requirements as `dot_product_avx2`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
#[allow(clippy::too_many_lines)] // Remainder unrolling adds lines for performance
unsafe fn squared_l2_avx2(a: &[f32], b: &[f32]) -> f32 {
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
            if r >= 4 {
                for i in 0..4 {
                    let d = a[rbase + i] - b[rbase + i];
                    result += d * d;
                }
                if r >= 5 {
                    let d = a[rbase + 4] - b[rbase + 4];
                    result += d * d;
                }
                if r >= 6 {
                    let d = a[rbase + 5] - b[rbase + 5];
                    result += d * d;
                }
                if r == 7 {
                    let d = a[rbase + 6] - b[rbase + 6];
                    result += d * d;
                }
            } else if r >= 2 {
                let d0 = a[rbase] - b[rbase];
                result += d0 * d0;
                let d1 = a[rbase + 1] - b[rbase + 1];
                result += d1 * d1;
                if r == 3 {
                    let d2 = a[rbase + 2] - b[rbase + 2];
                    result += d2 * d2;
                }
            } else if r == 1 {
                let d = a[rbase] - b[rbase];
                result += d * d;
            }
        }
    } else if remainder >= 4 {
        for i in 0..4 {
            let d = a[base + i] - b[base + i];
            result += d * d;
        }
        if remainder >= 5 {
            let d = a[base + 4] - b[base + 4];
            result += d * d;
        }
        if remainder >= 6 {
            let d = a[base + 5] - b[base + 5];
            result += d * d;
        }
        if remainder >= 7 {
            let d = a[base + 6] - b[base + 6];
            result += d * d;
        }
    } else if remainder >= 2 {
        let d0 = a[base] - b[base];
        result += d0 * d0;
        let d1 = a[base + 1] - b[base + 1];
        result += d1 * d1;
        if remainder == 3 {
            let d2 = a[base + 2] - b[base + 2];
            result += d2 * d2;
        }
    } else if remainder == 1 {
        let d = a[base] - b[base];
        result += d * d;
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
unsafe fn squared_l2_avx2_1acc(a: &[f32], b: &[f32]) -> f32 {
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

    if remainder >= 4 {
        for i in 0..4 {
            let d = a[base + i] - b[base + i];
            result += d * d;
        }
        if remainder >= 5 {
            let d = a[base + 4] - b[base + 4];
            result += d * d;
        }
        if remainder >= 6 {
            let d = a[base + 5] - b[base + 5];
            result += d * d;
        }
        if remainder == 7 {
            let d = a[base + 6] - b[base + 6];
            result += d * d;
        }
    } else if remainder >= 2 {
        let d0 = a[base] - b[base];
        result += d0 * d0;
        let d1 = a[base + 1] - b[base + 1];
        result += d1 * d1;
        if remainder == 3 {
            let d2 = a[base + 2] - b[base + 2];
            result += d2 * d2;
        }
    } else if remainder == 1 {
        let d = a[base] - b[base];
        result += d * d;
    }

    result
}

// =============================================================================
// ARM NEON Implementation (aarch64)
// =============================================================================

/// ARM NEON dot product using native intrinsics.
///
/// # Safety
///
/// The unsafe blocks within are safe because:
/// - NEON is always available on aarch64 targets
/// - `vld1q_f32` handles unaligned loads safely
/// - Pointer arithmetic stays within slice bounds
#[cfg(target_arch = "aarch64")]
#[inline]
fn dot_product_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: vdupq_n_f32 is always safe on aarch64
    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            sum = vfmaq_f32(sum, va, vb);
        }
    }

    // Horizontal sum
    let mut result = unsafe { vaddvq_f32(sum) };

    // Handle remainder
    let base = simd_len * 4;
    for i in base..len {
        result += a[i] * b[i];
    }

    result
}

/// ARM NEON squared L2 distance.
///
/// # Safety
///
/// Same requirements as `dot_product_neon`.
#[cfg(target_arch = "aarch64")]
#[inline]
fn squared_l2_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: vdupq_n_f32 is always safe on aarch64
    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            let diff = vsubq_f32(va, vb);
            sum = vfmaq_f32(sum, diff, diff);
        }
    }

    let mut result = unsafe { vaddvq_f32(sum) };

    let base = simd_len * 4;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}

// =============================================================================
// Cached SIMD Level Detection (EPIC-033 US-002)
// =============================================================================

/// SIMD capability level detected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    /// AVX-512F available (x86_64 only).
    Avx512,
    /// AVX2 + FMA available (x86_64 only).
    Avx2,
    /// NEON available (aarch64, always true).
    Neon,
    /// Scalar fallback.
    Scalar,
}

/// Cached SIMD level - detected once at first use.
static SIMD_LEVEL: std::sync::OnceLock<SimdLevel> = std::sync::OnceLock::new();

/// Detects the best available SIMD level for the current CPU.
fn detect_simd_level() -> SimdLevel {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return SimdLevel::Avx512;
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return SimdLevel::Avx2;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        return SimdLevel::Neon;
    }

    #[allow(unreachable_code)]
    SimdLevel::Scalar
}

/// Returns the cached SIMD capability level.
#[inline]
#[must_use]
pub fn simd_level() -> SimdLevel {
    *SIMD_LEVEL.get_or_init(detect_simd_level)
}

// =============================================================================
// Public API with cached dispatch
// =============================================================================

/// Dot product with automatic dispatch to best available SIMD.
///
/// Runtime detection is cached after first call for zero-overhead dispatch.
///
/// # Dispatch Strategy Adaptative (EPIC-PERF-003)
///
/// La stratégie s'adapte automatiquement au CPU détecté :
///
/// ## AVX-512 (Xeon, serveurs, anciens Core)
/// - 4-acc (len >= 512): 4 accumulateurs pour masquer latence FMA (4 cycles)
/// - 1-acc (len >= 16): Standard avec masked remainder
///
/// ## AVX2 (Core 12th/13th/14th gen, Ryzen)
/// - 4-acc (len >= 256): 4 accumulateurs AVX2 (masque latence 3-4 cycles)
/// - 2-acc (len >= 16): Standard optimisé pour petits vecteurs
///
/// ## NEON (Apple Silicon, ARM64)
/// - 1-acc (len >= 4): FMA natif ARM
///
/// ## Scalar (fallback)
/// - Loop simple pour tous les cas
///
/// Les seuils sont calibrés pour éviter les régressions sur chaque architecture.
#[inline]
#[must_use]
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    match simd_level() {
        // AVX-512: 4-acc pour très grands vecteurs, 1-acc pour le reste
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => unsafe { dot_product_avx512_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 16 => unsafe { dot_product_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => unsafe { dot_product_avx512(a, b) }, // < 16 elements, masked loads handle it
        // AVX2: seuils optimisés basés sur la recherche
        // - < 16: scalar (overhead SIMD trop élevé)
        // - 16-63: 1-acc (meilleur ratio overhead/perf)
        // - 64-255: 2-acc (ILP sans overhead excessif)
        // - 256+: 4-acc (maximise ILP pour grands vecteurs)
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 256 => unsafe { dot_product_avx2_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => unsafe { dot_product_avx2(a, b) }, // 2-acc
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 16 => unsafe { dot_product_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => unsafe { dot_product_avx2_1acc(a, b) }, // 8-15 elements
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 => a.iter().zip(b.iter()).map(|(x, y)| x * y).sum(), // < 8 elements
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => dot_product_neon(a, b),
        _ => a.iter().zip(b.iter()).map(|(x, y)| x * y).sum(),
    }
}

/// Squared L2 distance with automatic dispatch to best available SIMD.
///
/// Runtime detection is cached after first call for zero-overhead dispatch.
///
/// # Note
///
/// Unlike `dot_product_native`, this function does not use the 4-accumulator
/// optimization for large vectors. This is a potential future enhancement.
#[inline]
#[must_use]
pub fn squared_l2_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 16 => unsafe { squared_l2_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => unsafe { squared_l2_avx512(a, b) },
        // AVX2: seuils optimisés (mêmes que dot_product)
        // - < 16: scalar
        // - 16-63: 1-acc
        // - 64+: 2-acc (4-acc TODO pour 256+)
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => unsafe { squared_l2_avx2(a, b) }, // 2-acc
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 16 => unsafe { squared_l2_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => unsafe { squared_l2_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 => a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum(),
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => squared_l2_neon(a, b),
        _ => a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum(),
    }
}

/// Euclidean distance with automatic dispatch.
#[inline]
#[must_use]
pub fn euclidean_native(a: &[f32], b: &[f32]) -> f32 {
    squared_l2_native(a, b).sqrt()
}

/// L2 norm with automatic dispatch to best available SIMD.
///
/// Computes `sqrt(sum(v[i]²))` using native SIMD intrinsics.
#[inline]
#[must_use]
pub fn norm_native(v: &[f32]) -> f32 {
    // Norm is sqrt(dot(v, v))
    dot_product_native(v, v).sqrt()
}

/// Normalizes a vector in-place using native SIMD.
///
/// After normalization, the vector will have L2 norm of 1.0.
#[inline]
pub fn normalize_inplace_native(v: &mut [f32]) {
    let n = norm_native(v);
    if n > 0.0 {
        let inv_norm = 1.0 / n;
        for x in v.iter_mut() {
            *x *= inv_norm;
        }
    }
}

/// Cosine similarity for pre-normalized vectors with automatic dispatch.
#[inline]
#[must_use]
pub fn cosine_normalized_native(a: &[f32], b: &[f32]) -> f32 {
    // For unit vectors: cos(θ) = a · b
    dot_product_native(a, b)
}

/// Full cosine similarity (with normalization) using native SIMD.
#[inline]
#[must_use]
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Compute dot product and norms in single pass for better cache utilization
    let mut dot = 0.0_f32;
    let mut norm_a_sq = 0.0_f32;
    let mut norm_b_sq = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
    }

    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    // Clamp to [-1, 1] for consistency with scalar implementations
    // Floating point errors can produce values slightly outside this range
    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

// =============================================================================
// Newton-Raphson Fast Inverse Square Root (EPIC-PERF-001)
// =============================================================================

/// Fast approximate inverse square root using Newton-Raphson iteration.
///
/// Based on the famous Quake III algorithm, adapted for modern use.
/// Provides ~1-2% accuracy with significant speedup over `1.0 / x.sqrt()`.
///
/// # Performance
///
/// - Avoids expensive `sqrt()` call from libc
/// - Uses bit manipulation + one Newton-Raphson iteration
/// - ~2x faster than standard sqrt on most CPUs
///
/// # References
///
/// - SimSIMD v5.4.0: Newton-Raphson substitution
/// - arXiv: "Bang for the Buck: Vector Search on Cloud CPUs"
#[inline]
#[must_use]
pub fn fast_rsqrt(x: f32) -> f32 {
    // SAFETY: Bit manipulation is safe for f32
    // Magic constant from Quake III, refined for f32
    let i = x.to_bits();
    let i = 0x5f37_5a86_u32.wrapping_sub(i >> 1);
    let y = f32::from_bits(i);

    // One Newton-Raphson iteration: y = y * (1.5 - 0.5 * x * y * y)
    // This gives ~1% accuracy, sufficient for cosine similarity
    let half_x = 0.5 * x;
    y * (1.5 - half_x * y * y)
}

/// Fast cosine similarity using Newton-Raphson rsqrt.
///
/// Optimized version that avoids two `sqrt()` calls by using fast_rsqrt.
/// Accuracy is within 2% of exact computation, acceptable for similarity ranking.
///
/// # Performance
///
/// - ~20-50% faster than standard cosine_similarity_native
/// - Uses single-pass dot product + norms computation
/// - Avoids libc sqrt() overhead
#[inline]
#[must_use]
pub fn cosine_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Compute dot product and squared norms in single pass
    let mut dot = 0.0_f32;
    let mut norm_a_sq = 0.0_f32;
    let mut norm_b_sq = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
    }

    // Guard against zero vectors
    if norm_a_sq == 0.0 || norm_b_sq == 0.0 {
        return 0.0;
    }

    // Use fast_rsqrt: cos = dot * rsqrt(norm_a_sq) * rsqrt(norm_b_sq)
    dot * fast_rsqrt(norm_a_sq) * fast_rsqrt(norm_b_sq)
}

/// Batch dot products with prefetching.
///
/// Computes dot products between a query and multiple candidates,
/// using software prefetch hints for cache optimization.
#[must_use]
pub fn batch_dot_product_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        // Prefetch ahead for cache warming
        #[cfg(target_arch = "x86_64")]
        if i + 4 < candidates.len() {
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                _mm_prefetch(candidates[i + 4].as_ptr().cast::<i8>(), _MM_HINT_T0);
            }
        }

        // Note: aarch64 prefetch requires unstable feature, skipped for now
        // See: https://github.com/rust-lang/rust/issues/117217

        results.push(dot_product_native(candidate, query));
    }

    results
}

// =============================================================================
// Hamming & Jaccard (migrated from simd_explicit - EPIC-075)
// Optimized with AVX2/AVX-512 SIMD intrinsics
// =============================================================================

/// Hamming distance between two vectors using SIMD.
///
/// Uses AVX-512 VPTESTMD for parallel comparison on x86_64,
/// with fallback to AVX2 or scalar for smaller vectors.
///
/// # Panics
///
/// Panics if vectors have different lengths.
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

/// Jaccard similarity between two vectors using SIMD.
///
/// Uses AVX2/AVX-512 for parallel min/max operations.
/// Computes intersection / union for element-wise min/max interpretation.
///
/// # Panics
///
/// Panics if vectors have different lengths.
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

/// SIMD Hamming distance with runtime dispatch.
#[inline]
fn hamming_simd(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            return unsafe { hamming_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            return unsafe { hamming_avx2(a, b) };
        }
    }
    hamming_scalar(a, b)
}

/// SIMD Jaccard similarity with runtime dispatch.
#[inline]
fn jaccard_simd(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            return unsafe { jaccard_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            return unsafe { jaccard_avx2(a, b) };
        }
    }
    jaccard_scalar(a, b)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn hamming_avx512(a: &[f32], b: &[f32]) -> f32 {
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
    diff_count as f32 + hamming_scalar(&a[i..], &b[i..])
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn hamming_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    let mut diff_count: u64 = 0;
    let mut i = 0;

    // Threshold for binary comparison
    let threshold = _mm256_set1_ps(0.5);

    // Process 8 floats at a time using AVX2
    while i + 8 <= len {
        let va = _mm256_loadu_ps(a_ptr.add(i));
        let vb = _mm256_loadu_ps(b_ptr.add(i));

        // Binary threshold: compare each value > 0.5
        let cmp_a = _mm256_cmp_ps(va, threshold, _CMP_GT_OQ);
        let cmp_b = _mm256_cmp_ps(vb, threshold, _CMP_GT_OQ);

        // XOR to find positions where binary values differ
        let diff = _mm256_xor_ps(cmp_a, cmp_b);
        let mask = _mm256_movemask_ps(diff);
        diff_count += mask.count_ones() as u64;

        i += 8;
    }

    // Handle remaining elements
    diff_count as f32 + hamming_scalar(&a[i..], &b[i..])
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn jaccard_avx512(a: &[f32], b: &[f32]) -> f32 {
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
    let (scalar_inter, scalar_union) = jaccard_scalar_accum(&a[i..], &b[i..]);

    let total_inter = inter_sum + scalar_inter;
    let total_union = union_sum + scalar_union;

    if total_union == 0.0 {
        1.0
    } else {
        total_inter / total_union
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn jaccard_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    let mut acc_inter = _mm256_setzero_ps();
    let mut acc_union = _mm256_setzero_ps();

    let mut i = 0;
    // Process 8 floats at a time
    while i + 8 <= len {
        let va = _mm256_loadu_ps(a_ptr.add(i));
        let vb = _mm256_loadu_ps(b_ptr.add(i));

        // min for intersection, max for union
        acc_inter = _mm256_add_ps(acc_inter, _mm256_min_ps(va, vb));
        acc_union = _mm256_add_ps(acc_union, _mm256_max_ps(va, vb));

        i += 8;
    }

    // Horizontal sum using hadd
    let inter_sum = hsum256_ps(acc_inter);
    let union_sum = hsum256_ps(acc_union);

    // Handle remaining elements
    let (scalar_inter, scalar_union) = jaccard_scalar_accum(&a[i..], &b[i..]);

    let total_inter = inter_sum + scalar_inter;
    let total_union = union_sum + scalar_union;

    if total_union == 0.0 {
        1.0
    } else {
        total_inter / total_union
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn hsum256_ps(v: std::arch::x86_64::__m256) -> f32 {
    use std::arch::x86_64::*;
    // Extract high and low 128-bit halves
    let low = _mm256_castps256_ps128(v);
    let high = _mm256_extractf128_ps(v, 1);
    // Add them
    let sum128 = _mm_add_ps(low, high);
    // Horizontal sum of 128-bit
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let result = _mm_add_ss(sums, shuf2);
    _mm_cvtss_f32(result)
}

/// Scalar Hamming distance implementation.
///
/// Uses binary threshold at 0.5 for consistency with SIMD versions.
/// This is the standard interpretation for binary/categorical vectors.
#[inline]
fn hamming_scalar(a: &[f32], b: &[f32]) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    {
        a.iter()
            .zip(b.iter())
            .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
            .count() as f32
    }
}

/// Scalar Jaccard similarity implementation.
#[inline]
fn jaccard_scalar(a: &[f32], b: &[f32]) -> f32 {
    let (intersection, union) = jaccard_scalar_accum(a, b);
    if union == 0.0 {
        1.0
    } else {
        intersection / union
    }
}

/// Helper to compute Jaccard accumulator values.
#[inline]
fn jaccard_scalar_accum(a: &[f32], b: &[f32]) -> (f32, f32) {
    a.iter()
        .zip(b.iter())
        .fold((0.0_f32, 0.0_f32), |(inter, uni), (x, y)| {
            (inter + x.min(*y), uni + x.max(*y))
        })
}

// Tests moved to simd_native_tests.rs per project rules (tests in separate files)

#[cfg(test)]
mod simd_native_dispatch_tests;
