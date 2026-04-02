//! ARM NEON kernel implementations for aarch64.
//!
//! Contains hand-tuned NEON SIMD kernels for dot product, cosine similarity,
//! squared L2 distance, Hamming distance, and Jaccard similarity with 1-acc
//! and 4-acc variants for different vector sizes.
//!
//! NEON is always available on aarch64, so no runtime detection is needed.

// SAFETY: Numeric casts in this file are intentional and safe:
// - All casts are from well-bounded values (vector dimensions, loop indices)
// - All casts are validated by extensive SIMD tests (simd_native_tests.rs)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::similar_names)]

// =============================================================================
// Dot Product
// =============================================================================

/// ARM NEON dot product with 4 accumulators for ILP optimization (EPIC-052/US-009).
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn dot_product_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();

    if len >= 64 {
        return dot_product_neon_4acc(a, b);
    }

    let simd_len = len / 4;
    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // Reason: Initialise the SIMD accumulator register to zero before the reduction loop.
    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32` loads 4 f32 values from an unaligned address on aarch64.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, so both pointers stay within slice bounds.
        // - Condition 2: `vld1q_f32` is documented to support unaligned loads on ARM64.
        // Reason: Core NEON computation for dot product accumulation per 4-element block.
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            sum = vfmaq_f32(sum, va, vb);
        }
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: `sum` is a valid float32x4_t value set by `vdupq_n_f32`/`vfmaq_f32`.
    // Reason: Horizontal reduction of the SIMD accumulator to a scalar dot-product result.
    let mut result = unsafe { vaddvq_f32(sum) };

    let base = simd_len * 4;
    for i in base..len {
        result += a[i] * b[i];
    }

    result
}

/// NEON FMA wrapper with x86-compatible argument order.
///
/// NEON `vfmaq_f32(acc, a, b)` = acc + a*b, but [`simd_4acc_dot_loop!`] expects
/// `fmadd(a, b, acc)` = a*b + acc. This wrapper reorders the arguments.
///
/// SAFETY: `vfmaq_f32` is a non-faulting register operation on aarch64.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn neon_fma_compat(
    a: std::arch::aarch64::float32x4_t,
    b: std::arch::aarch64::float32x4_t,
    acc: std::arch::aarch64::float32x4_t,
) -> std::arch::aarch64::float32x4_t {
    std::arch::aarch64::vfmaq_f32(acc, a, b)
}

/// ARM NEON dot product with 4 accumulators for large vectors.
#[cfg(target_arch = "aarch64")]
#[inline]
fn dot_product_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    // SAFETY: `add` on a raw pointer derived from a valid slice.
    // - Condition 1: `len / 16 * 16 <= len`, so `end_main` is within or at the end of the slice.
    // - Condition 2: `add(len)` yields the one-past-the-end pointer, which is valid for comparison.
    // Reason: Establish loop bounds for the 16-element-wide main body and scalar tail.
    let end_main = unsafe { a.as_ptr().add(len / 16 * 16) };
    let end_ptr = unsafe { a.as_ptr().add(len) };

    // SAFETY: 4-accumulator ILP loop using NEON intrinsics. All pointer bounds
    // guaranteed by `end_main`. `neon_fma_compat` reorders args to match macro convention.
    let (combined, mut a_ptr, mut b_ptr) = unsafe {
        crate::simd_4acc_dot_loop!(
            a.as_ptr(),
            b.as_ptr(),
            end_main,
            vdupq_n_f32(0.0),
            vld1q_f32,
            neon_fma_compat,
            vaddq_f32,
            4
        )
    };

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // Reason: Horizontal reduction of the combined SIMD accumulator to a scalar result.
    let mut result = unsafe { vaddvq_f32(combined) };

    while a_ptr < end_ptr {
        // SAFETY: Raw pointer dereference for scalar tail processing.
        // - Condition 1: Loop condition `a_ptr < end_ptr` guarantees both pointers are within slice bounds.
        // - Condition 2: `b_ptr` advances in step with `a_ptr` so it remains within the `b` slice.
        // Reason: Handle the remaining 0-15 elements that the 16-wide SIMD loop did not cover.
        unsafe {
            result += *a_ptr * *b_ptr;
            a_ptr = a_ptr.add(1);
            b_ptr = b_ptr.add(1);
        }
    }

    result
}

// =============================================================================
// Cosine Similarity
// =============================================================================

/// ARM NEON cosine similarity — fused single-pass kernel.
///
/// Computes `dot(a,b)`, `norm(a)^2`, and `norm(b)^2` simultaneously in one
/// pass over the data, using 3 independent NEON accumulators. For vectors
/// with >= 64 elements, delegates to [`cosine_fused_neon_4acc`] which uses
/// 12 accumulators (3 products x 4-way ILP).
///
/// This replaces the prior 3-pass approach (`dot_product_neon` called 3x).
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn cosine_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        // SAFETY: `cosine_fused_neon_4acc` requires NEON (guaranteed on aarch64)
        // and len >= 64 (checked above).
        // Reason: Delegate to the 4-accumulator ILP variant for large vectors.
        return unsafe { cosine_fused_neon_4acc(a, b) };
    }
    // SAFETY: `cosine_fused_neon_1acc` requires NEON (guaranteed on aarch64).
    // Reason: Single-accumulator variant for small/medium vectors.
    unsafe { cosine_fused_neon_1acc(a, b) }
}

/// Single-accumulator fused cosine for vectors with < 64 elements.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn cosine_fused_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate value 0.0 is valid for the instruction.
    // Reason: Initialise three SIMD accumulators (dot, norm_a, norm_b).
    let mut dot_acc = vdupq_n_f32(0.0);
    let mut na_acc = vdupq_n_f32(0.0);
    let mut nb_acc = vdupq_n_f32(0.0);

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32`/`vfmaq_f32` are non-faulting NEON operations.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, pointers within bounds.
        // - Condition 2: `vld1q_f32` supports unaligned loads on ARM64.
        // Reason: Single-pass accumulation of dot, norm_a_sq, norm_b_sq.
        let va = vld1q_f32(a_ptr.add(offset));
        let vb = vld1q_f32(b_ptr.add(offset));
        dot_acc = vfmaq_f32(dot_acc, va, vb);
        na_acc = vfmaq_f32(na_acc, va, va);
        nb_acc = vfmaq_f32(nb_acc, vb, vb);
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to scalar on aarch64.
    // - Condition 1: All accumulators are valid float32x4_t values.
    // Reason: Horizontal reduction of the three accumulators.
    let mut dot = vaddvq_f32(dot_acc);
    let mut norm_a_sq = vaddvq_f32(na_acc);
    let mut norm_b_sq = vaddvq_f32(nb_acc);

    let base = simd_len * 4;
    for i in base..len {
        let x = a[i];
        let y = b[i];
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
    }

    finalize_cosine(dot, norm_a_sq, norm_b_sq)
}

/// Four-accumulator fused cosine for vectors with >= 64 elements.
///
/// Uses 12 NEON registers (3 products x 4-way ILP) and processes 16
/// elements per iteration, following the pattern from `cosine_fused_avx2_2acc`.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn cosine_fused_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len();
    let main_end = len / 16 * 16;
    // SAFETY: `add` on a raw pointer derived from a valid slice.
    // - Condition 1: `main_end <= len`, so pointer stays within the allocation.
    // - Condition 2: `add(len)` yields one-past-end, valid for comparison.
    // Reason: Establish loop bounds for 16-wide main body and scalar tail.
    let end_main = a.as_ptr().add(main_end);
    let end_ptr = a.as_ptr().add(len);

    let (dot, norm_a_sq, norm_b_sq) = cosine_fused_neon_main_loop(a.as_ptr(), b.as_ptr(), end_main);

    let (dot, norm_a_sq, norm_b_sq) = cosine_fused_neon_scalar_tail(
        end_main,
        b.as_ptr().add(main_end),
        end_ptr,
        dot,
        norm_a_sq,
        norm_b_sq,
    );

    finalize_cosine(dot, norm_a_sq, norm_b_sq)
}

/// Reduces 4 NEON f32x4 accumulators to a single scalar sum.
///
/// SAFETY: All inputs must be valid `float32x4_t` values.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn reduce_4acc_neon(
    a0: std::arch::aarch64::float32x4_t,
    a1: std::arch::aarch64::float32x4_t,
    a2: std::arch::aarch64::float32x4_t,
    a3: std::arch::aarch64::float32x4_t,
) -> f32 {
    use std::arch::aarch64::*;
    // SAFETY: `vaddq_f32`/`vaddvq_f32` are non-faulting register operations.
    // - Condition 1: All accumulators hold valid float32x4_t values.
    // Reason: Reduce 4 accumulators to scalar result.
    let ab01 = vaddq_f32(a0, a1);
    let ab23 = vaddq_f32(a2, a3);
    vaddvq_f32(vaddq_f32(ab01, ab23))
}

/// Main 16-wide SIMD loop for fused cosine (4-acc ILP).
///
/// Returns `(dot, norm_a_sq, norm_b_sq)` accumulated over full 16-element blocks.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn cosine_fused_neon_main_loop(
    mut a_ptr: *const f32,
    mut b_ptr: *const f32,
    end_main: *const f32,
) -> (f32, f32, f32) {
    use std::arch::aarch64::*;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate 0.0 is valid.
    // Reason: Initialise 12 accumulators (3 products x 4-way ILP).
    let (mut d0, mut d1, mut d2, mut d3) = (
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
    );
    let (mut na0, mut na1, mut na2, mut na3) = (
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
    );
    let (mut nb0, mut nb1, mut nb2, mut nb3) = (
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
        vdupq_n_f32(0.0),
    );

    while a_ptr < end_main {
        // SAFETY: Loop condition guarantees 16 elements remain before `end_main`.
        // - Condition 1: `vld1q_f32` supports unaligned loads on ARM64.
        // Reason: 16-wide single-pass accumulation with 4-way ILP.
        let va0 = vld1q_f32(a_ptr);
        let vb0 = vld1q_f32(b_ptr);
        d0 = vfmaq_f32(d0, va0, vb0);
        na0 = vfmaq_f32(na0, va0, va0);
        nb0 = vfmaq_f32(nb0, vb0, vb0);

        let va1 = vld1q_f32(a_ptr.add(4));
        let vb1 = vld1q_f32(b_ptr.add(4));
        d1 = vfmaq_f32(d1, va1, vb1);
        na1 = vfmaq_f32(na1, va1, va1);
        nb1 = vfmaq_f32(nb1, vb1, vb1);

        let va2 = vld1q_f32(a_ptr.add(8));
        let vb2 = vld1q_f32(b_ptr.add(8));
        d2 = vfmaq_f32(d2, va2, vb2);
        na2 = vfmaq_f32(na2, va2, va2);
        nb2 = vfmaq_f32(nb2, vb2, vb2);

        let va3 = vld1q_f32(a_ptr.add(12));
        let vb3 = vld1q_f32(b_ptr.add(12));
        d3 = vfmaq_f32(d3, va3, vb3);
        na3 = vfmaq_f32(na3, va3, va3);
        nb3 = vfmaq_f32(nb3, vb3, vb3);

        a_ptr = a_ptr.add(16);
        b_ptr = b_ptr.add(16);
    }

    (
        reduce_4acc_neon(d0, d1, d2, d3),
        reduce_4acc_neon(na0, na1, na2, na3),
        reduce_4acc_neon(nb0, nb1, nb2, nb3),
    )
}

/// Scalar tail for fused cosine — handles the remaining 0..15 elements.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn cosine_fused_neon_scalar_tail(
    mut a_ptr: *const f32,
    mut b_ptr: *const f32,
    end_ptr: *const f32,
    mut dot: f32,
    mut norm_a_sq: f32,
    mut norm_b_sq: f32,
) -> (f32, f32, f32) {
    while a_ptr < end_ptr {
        // SAFETY: Loop condition guarantees both pointers are within slice bounds.
        // - Condition 1: `b_ptr` advances in step with `a_ptr`.
        // Reason: Handle remaining elements the 16-wide loop did not cover.
        let x = *a_ptr;
        let y = *b_ptr;
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
        a_ptr = a_ptr.add(1);
        b_ptr = b_ptr.add(1);
    }
    (dot, norm_a_sq, norm_b_sq)
}

/// Finalize cosine from dot product and squared norms.
#[cfg(target_arch = "aarch64")]
#[inline]
fn finalize_cosine(dot: f32, norm_a_sq: f32, norm_b_sq: f32) -> f32 {
    super::scalar::cosine_finish_fast(dot, norm_a_sq, norm_b_sq)
}

// =============================================================================
// Squared L2 Distance
// =============================================================================

/// ARM NEON squared L2 distance with adaptive accumulator selection.
///
/// For vectors with >= 64 elements, delegates to [`squared_l2_neon_4acc`]
/// which uses 4 independent accumulators to hide FMA latency through ILP.
/// Smaller vectors use a single-accumulator loop.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn squared_l2_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        return squared_l2_neon_4acc(a, b);
    }
    // SAFETY: `squared_l2_neon_1acc` requires NEON (guaranteed on aarch64).
    // Reason: Single-accumulator variant for small/medium vectors.
    unsafe { squared_l2_neon_1acc(a, b) }
}

/// Single-accumulator NEON squared L2 distance for vectors with < 64 elements.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn squared_l2_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // Reason: Initialise the SIMD accumulator register to zero before the squared-diff loop.
    let mut sum = vdupq_n_f32(0.0);

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32`/`vsubq_f32`/`vfmaq_f32` are non-faulting NEON operations.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, so both pointers stay within slice bounds.
        // - Condition 2: `vld1q_f32` is documented to support unaligned loads on ARM64.
        // Reason: Compute squared element-wise differences for the L2 distance accumulator.
        let va = vld1q_f32(a_ptr.add(offset));
        let vb = vld1q_f32(b_ptr.add(offset));
        let diff = vsubq_f32(va, vb);
        sum = vfmaq_f32(sum, diff, diff);
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: `sum` is a valid float32x4_t value set by `vdupq_n_f32`/`vfmaq_f32`.
    // Reason: Horizontal reduction of the squared-difference accumulator to a scalar result.
    let mut result = vaddvq_f32(sum);

    let base = simd_len * 4;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}

/// ARM NEON squared L2 distance with 4 accumulators for large vectors.
///
/// Uses the [`simd_4acc_l2_loop!`] macro with 4 independent `float32x4_t`
/// accumulators processing 16 elements per iteration (4 lanes x 4 accumulators).
/// This hides FMA latency through instruction-level parallelism, following the
/// same pattern as [`dot_product_neon_4acc`].
///
/// Apple M1-M4 use 128-byte cache lines; NEON processes 16 floats (64 bytes)
/// per iteration, so two iterations fully consume one cache line.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn squared_l2_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    // SAFETY: `add` on a raw pointer derived from a valid slice.
    // - Condition 1: `len / 16 * 16 <= len`, so `end_main` is within or at the end of the slice.
    // - Condition 2: `add(len)` yields the one-past-the-end pointer, which is valid for comparison.
    // Reason: Establish loop bounds for the 16-element-wide main body and scalar tail.
    let end_main = unsafe { a.as_ptr().add(len / 16 * 16) };
    let end_ptr = unsafe { a.as_ptr().add(len) };

    // SAFETY: 4-accumulator ILP loop using NEON intrinsics. All pointer bounds
    // guaranteed by `end_main`. `neon_fma_compat` reorders args to match macro convention.
    // `vsubq_f32` computes element-wise difference before FMA accumulates diff².
    let (combined, mut a_ptr, mut b_ptr) = unsafe {
        crate::simd_4acc_l2_loop!(
            a.as_ptr(),
            b.as_ptr(),
            end_main,
            vdupq_n_f32(0.0),
            vld1q_f32,
            vsubq_f32,
            neon_fma_compat,
            vaddq_f32,
            4
        )
    };

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // Reason: Horizontal reduction of the combined SIMD accumulator to a scalar result.
    let mut result = unsafe { vaddvq_f32(combined) };

    while a_ptr < end_ptr {
        // SAFETY: Raw pointer dereference for scalar tail processing.
        // - Condition 1: Loop condition `a_ptr < end_ptr` guarantees both pointers are within slice bounds.
        // - Condition 2: `b_ptr` advances in step with `a_ptr` so it remains within the `b` slice.
        // Reason: Handle the remaining 0-15 elements that the 16-wide SIMD loop did not cover.
        unsafe {
            let d = *a_ptr - *b_ptr;
            result += d * d;
            a_ptr = a_ptr.add(1);
            b_ptr = b_ptr.add(1);
        }
    }

    result
}

// =============================================================================
// Hamming Distance
// =============================================================================

/// ARM NEON Hamming distance with adaptive accumulator selection.
///
/// Computes the number of positions where binary-thresholded values differ
/// (threshold at 0.5), consistent with AVX2/AVX-512 Hamming kernels.
/// For vectors with >= 64 elements, delegates to [`hamming_neon_4acc`] which
/// uses 4-way ILP for higher throughput.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn hamming_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        // SAFETY: `hamming_neon_4acc` requires NEON (guaranteed on aarch64)
        // and len >= 64 (checked above).
        // Reason: Delegate to the 4-accumulator ILP variant for large vectors.
        return unsafe { hamming_neon_4acc(a, b) };
    }
    // SAFETY: `hamming_neon_1acc` requires NEON (guaranteed on aarch64).
    // Reason: Single-accumulator variant for small/medium vectors.
    unsafe { hamming_neon_1acc(a, b) }
}

/// Single-accumulator NEON Hamming distance for vectors with < 64 elements.
///
/// Binary-thresholds each lane at 0.5, XORs the masks, and counts differing
/// positions. Accumulates in `uint32x4_t` for exact integer precision.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn hamming_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_u32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0 is a valid u32 constant accepted by the instruction.
    // Reason: Initialise the SIMD diff-count accumulator to zero before the reduction loop.
    let mut diff_count = vdupq_n_u32(0);

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate value 0.5 is a valid f32 constant.
    // Reason: Create threshold vector for binary comparison.
    let threshold = vdupq_n_f32(0.5);

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32`/`vcgtq_f32`/`veorq_u32`/`vshrq_n_u32`/`vaddq_u32` are
        // non-faulting NEON operations.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, so both pointers stay within
        //   slice bounds.
        // - Condition 2: `vld1q_f32` supports unaligned loads on ARM64.
        // Reason: Binary-threshold each lane, XOR masks, shift to 0/1, accumulate count.
        let va = vld1q_f32(a_ptr.add(offset));
        let vb = vld1q_f32(b_ptr.add(offset));

        // Compare > 0.5 yields all-1s (0xFFFF_FFFF) or all-0s per lane
        let mask_a = vcgtq_f32(va, threshold);
        let mask_b = vcgtq_f32(vb, threshold);

        // XOR finds lanes where binary values differ
        let diff = veorq_u32(mask_a, mask_b);

        // Shift right by 31 to convert 0xFFFF_FFFF -> 1, 0x0000_0000 -> 0
        let ones = vshrq_n_u32::<31>(diff);
        diff_count = vaddq_u32(diff_count, ones);
    }

    // SAFETY: `vaddvq_u32` reduces a 128-bit u32 register to a scalar u32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: `diff_count` is a valid uint32x4_t value set by `vdupq_n_u32`/`vaddq_u32`.
    // Reason: Horizontal reduction of the diff-count accumulator to a scalar result.
    let mut result = vaddvq_u32(diff_count);

    // Scalar tail for remainder 0-3 elements
    let base = simd_len * 4;
    for i in base..len {
        let x = a[i] > 0.5;
        let y = b[i] > 0.5;
        if x != y {
            result += 1;
        }
    }

    result as f32
}

/// Four-accumulator NEON Hamming distance for vectors with >= 64 elements.
///
/// Processes 16 elements per iteration with 4 independent `uint32x4_t` diff-count
/// accumulators for instruction-level parallelism. Uses binary tree reduction
/// at the end for the horizontal sum.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn hamming_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let main_end = len / 16 * 16;

    // SAFETY: `vdupq_n_u32` / `vdupq_n_f32` are non-faulting register initialisations.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate values 0 / 0.5 are valid constants.
    // Reason: Initialise 4 diff-count accumulators and threshold vector.
    let mut dc0 = vdupq_n_u32(0);
    let mut dc1 = vdupq_n_u32(0);
    let mut dc2 = vdupq_n_u32(0);
    let mut dc3 = vdupq_n_u32(0);
    let threshold = vdupq_n_f32(0.5);

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    let mut offset = 0;
    while offset < main_end {
        // SAFETY: `vld1q_f32` loads 4 f32 values from an unaligned address on aarch64.
        // - Condition 1: `offset + 16 <= main_end <= len`, so all 16 pointers stay within
        //   slice bounds across the 4 blocks.
        // - Condition 2: `vld1q_f32` is documented to support unaligned loads on ARM64.
        // Reason: 16-wide single-pass binary comparison with 4-way ILP.

        // Block 0
        let va0 = vld1q_f32(a_ptr.add(offset));
        let vb0 = vld1q_f32(b_ptr.add(offset));
        let diff0 = veorq_u32(vcgtq_f32(va0, threshold), vcgtq_f32(vb0, threshold));
        dc0 = vaddq_u32(dc0, vshrq_n_u32::<31>(diff0));

        // Block 1
        let va1 = vld1q_f32(a_ptr.add(offset + 4));
        let vb1 = vld1q_f32(b_ptr.add(offset + 4));
        let diff1 = veorq_u32(vcgtq_f32(va1, threshold), vcgtq_f32(vb1, threshold));
        dc1 = vaddq_u32(dc1, vshrq_n_u32::<31>(diff1));

        // Block 2
        let va2 = vld1q_f32(a_ptr.add(offset + 8));
        let vb2 = vld1q_f32(b_ptr.add(offset + 8));
        let diff2 = veorq_u32(vcgtq_f32(va2, threshold), vcgtq_f32(vb2, threshold));
        dc2 = vaddq_u32(dc2, vshrq_n_u32::<31>(diff2));

        // Block 3
        let va3 = vld1q_f32(a_ptr.add(offset + 12));
        let vb3 = vld1q_f32(b_ptr.add(offset + 12));
        let diff3 = veorq_u32(vcgtq_f32(va3, threshold), vcgtq_f32(vb3, threshold));
        dc3 = vaddq_u32(dc3, vshrq_n_u32::<31>(diff3));

        offset += 16;
    }

    // Binary tree reduction: (dc0+dc1) + (dc2+dc3) then horizontal sum
    // SAFETY: `vaddq_u32`/`vaddvq_u32` are non-faulting register operations.
    // - Condition 1: All accumulators hold valid uint32x4_t values.
    // Reason: Reduce 4 accumulators to scalar diff count.
    let ab01 = vaddq_u32(dc0, dc1);
    let ab23 = vaddq_u32(dc2, dc3);
    let mut result = vaddvq_u32(vaddq_u32(ab01, ab23));

    // Scalar tail for remainder 0-15 elements
    for i in main_end..len {
        let x = a[i] > 0.5;
        let y = b[i] > 0.5;
        if x != y {
            result += 1;
        }
    }

    result as f32
}

// =============================================================================
// Binary Hamming (packed u64)
// =============================================================================

/// ARM NEON binary Hamming distance for packed u64 vectors.
///
/// Processes 2 u64 (128 bits) per iteration using `vcntq_u8` (byte-level
/// popcount) followed by `vaddlvq_u8` (horizontal sum across 16 bytes).
/// NEON `cnt` is a single-cycle instruction on most ARM cores, making this
/// significantly faster than scalar `count_ones()` loops.
///
/// # Safety
///
/// Uses NEON intrinsics that are always available on aarch64. Pointer
/// arithmetic is bounded by `i + 2 <= len` loop guard.
#[cfg(target_arch = "aarch64")]
pub(crate) fn hamming_binary_neon(a: &[u64], b: &[u64]) -> u32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let mut total: u32 = 0;
    let mut i = 0;

    // Process 2 u64 (128 bits) per iteration using vcntq_u8 + horizontal sum
    while i + 2 <= len {
        // SAFETY: `vld1q_u64` loads 2 u64 from an unaligned address on aarch64.
        // - Condition 1: `i + 2 <= len`, so both pointers stay within slice bounds.
        // - Condition 2: `vld1q_u64` supports unaligned loads on ARM64.
        // Reason: NEON XOR + byte-popcount for binary Hamming distance.
        unsafe {
            let va = vld1q_u64(a.as_ptr().add(i));
            let vb = vld1q_u64(b.as_ptr().add(i));
            let xor = veorq_u64(va, vb);
            // Count set bits per byte, then sum all 16 bytes
            let cnt = vcntq_u8(vreinterpretq_u8_u64(xor));
            total += u32::from(vaddlvq_u8(cnt));
        }
        i += 2;
    }

    // Scalar tail for an odd trailing u64 element
    if i < len {
        total += (a[i] ^ b[i]).count_ones();
    }

    total
}

// =============================================================================
// Jaccard Similarity
// =============================================================================

/// ARM NEON Jaccard similarity with adaptive accumulator selection.
///
/// Computes generalized Jaccard similarity using `min` for intersection and
/// `max` for union, consistent with AVX2/AVX-512 Jaccard kernels. For vectors
/// with >= 64 elements, delegates to [`jaccard_neon_4acc`] which uses 8
/// accumulators (4 intersection + 4 union) for ILP.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn jaccard_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        // SAFETY: `jaccard_neon_4acc` requires NEON (guaranteed on aarch64)
        // and len >= 64 (checked above).
        // Reason: Delegate to the 4-accumulator ILP variant for large vectors.
        return unsafe { jaccard_neon_4acc(a, b) };
    }
    // SAFETY: `jaccard_neon_1acc` requires NEON (guaranteed on aarch64).
    // Reason: Single-accumulator variant for small/medium vectors.
    unsafe { jaccard_neon_1acc(a, b) }
}

/// Single-accumulator NEON Jaccard similarity for vectors with < 64 elements.
///
/// Accumulates `min(a, b)` for intersection and `max(a, b)` for union in
/// `float32x4_t` registers, then horizontally reduces.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn jaccard_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // Reason: Initialise intersection and union SIMD accumulators to zero.
    let mut inter_acc = vdupq_n_f32(0.0);
    let mut union_acc = vdupq_n_f32(0.0);

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32`/`vminq_f32`/`vmaxq_f32`/`vaddq_f32` are non-faulting
        // NEON operations.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, so both pointers stay
        //   within slice bounds.
        // - Condition 2: `vld1q_f32` supports unaligned loads on ARM64.
        // Reason: Accumulate min (intersection) and max (union) per 4-element block.
        let va = vld1q_f32(a_ptr.add(offset));
        let vb = vld1q_f32(b_ptr.add(offset));
        inter_acc = vaddq_f32(inter_acc, vminq_f32(va, vb));
        union_acc = vaddq_f32(union_acc, vmaxq_f32(va, vb));
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: Both accumulators are valid float32x4_t values.
    // Reason: Horizontal reduction of intersection and union accumulators.
    let mut inter = vaddvq_f32(inter_acc);
    let mut union_sum = vaddvq_f32(union_acc);

    // Scalar tail for remainder 0-3 elements
    let base = simd_len * 4;
    for i in base..len {
        let x = a[i];
        let y = b[i];
        inter += x.min(y);
        union_sum += x.max(y);
    }

    if union_sum == 0.0 {
        1.0
    } else {
        inter / union_sum
    }
}

/// Four-accumulator NEON Jaccard similarity for vectors with >= 64 elements.
///
/// Uses 8 NEON registers (4 intersection + 4 union) and processes 16 elements
/// per iteration for instruction-level parallelism. Binary tree reduction
/// merges accumulators at the end.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn jaccard_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let main_end = len / 16 * 16;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate value 0.0 is valid for the instruction.
    // Reason: Initialise 8 accumulators (4 intersection + 4 union) for ILP.
    let mut i0 = vdupq_n_f32(0.0);
    let mut i1 = vdupq_n_f32(0.0);
    let mut i2 = vdupq_n_f32(0.0);
    let mut i3 = vdupq_n_f32(0.0);
    let mut u0 = vdupq_n_f32(0.0);
    let mut u1 = vdupq_n_f32(0.0);
    let mut u2 = vdupq_n_f32(0.0);
    let mut u3 = vdupq_n_f32(0.0);

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    let mut offset = 0;
    while offset < main_end {
        // SAFETY: `vld1q_f32`/`vminq_f32`/`vmaxq_f32`/`vaddq_f32` are non-faulting
        // NEON operations.
        // - Condition 1: `offset + 16 <= main_end <= len`, so all 16 pointers stay
        //   within slice bounds across the 4 blocks.
        // - Condition 2: `vld1q_f32` supports unaligned loads on ARM64.
        // Reason: 16-wide single-pass min/max accumulation with 4-way ILP.

        // Block 0
        let va0 = vld1q_f32(a_ptr.add(offset));
        let vb0 = vld1q_f32(b_ptr.add(offset));
        i0 = vaddq_f32(i0, vminq_f32(va0, vb0));
        u0 = vaddq_f32(u0, vmaxq_f32(va0, vb0));

        // Block 1
        let va1 = vld1q_f32(a_ptr.add(offset + 4));
        let vb1 = vld1q_f32(b_ptr.add(offset + 4));
        i1 = vaddq_f32(i1, vminq_f32(va1, vb1));
        u1 = vaddq_f32(u1, vmaxq_f32(va1, vb1));

        // Block 2
        let va2 = vld1q_f32(a_ptr.add(offset + 8));
        let vb2 = vld1q_f32(b_ptr.add(offset + 8));
        i2 = vaddq_f32(i2, vminq_f32(va2, vb2));
        u2 = vaddq_f32(u2, vmaxq_f32(va2, vb2));

        // Block 3
        let va3 = vld1q_f32(a_ptr.add(offset + 12));
        let vb3 = vld1q_f32(b_ptr.add(offset + 12));
        i3 = vaddq_f32(i3, vminq_f32(va3, vb3));
        u3 = vaddq_f32(u3, vmaxq_f32(va3, vb3));

        offset += 16;
    }

    // Binary tree reduction for intersection: (i0+i1) + (i2+i3)
    // SAFETY: `vaddq_f32`/`vaddvq_f32` are non-faulting register operations.
    // - Condition 1: All accumulators hold valid float32x4_t values.
    // Reason: Reduce 4 intersection and 4 union accumulators to scalar results.
    let inter_01 = vaddq_f32(i0, i1);
    let inter_23 = vaddq_f32(i2, i3);
    let mut inter = vaddvq_f32(vaddq_f32(inter_01, inter_23));

    // Binary tree reduction for union: (u0+u1) + (u2+u3)
    let union_01 = vaddq_f32(u0, u1);
    let union_23 = vaddq_f32(u2, u3);
    let mut union_sum = vaddvq_f32(vaddq_f32(union_01, union_23));

    // Scalar tail for remainder 0-15 elements
    for idx in main_end..len {
        let x = a[idx];
        let y = b[idx];
        inter += x.min(y);
        union_sum += x.max(y);
    }

    if union_sum == 0.0 {
        1.0
    } else {
        inter / union_sum
    }
}
