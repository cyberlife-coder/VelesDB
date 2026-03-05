//! ARM NEON kernel implementations for aarch64.
//!
//! Contains hand-tuned NEON SIMD kernels for dot product and squared L2 distance
//! with 1-acc and 4-acc variants for different vector sizes.
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

/// ARM NEON dot product with 4 accumulators for large vectors.
#[cfg(target_arch = "aarch64")]
#[inline]
fn dot_product_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let mut a_ptr = a.as_ptr();
    let mut b_ptr = b.as_ptr();
    // SAFETY: `add` on a raw pointer derived from a valid slice.
    // - Condition 1: `len / 16 * 16 <= len`, so `end_main` is within or at the end of the slice.
    // - Condition 2: `add(len)` yields the one-past-the-end pointer, which is valid for comparison.
    // Reason: Establish loop bounds for the 16-element-wide main body and scalar tail.
    let end_main = unsafe { a.as_ptr().add(len / 16 * 16) };
    let end_ptr = unsafe { a.as_ptr().add(len) };

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // Reason: Initialise four independent SIMD accumulators for 4-way instruction-level parallelism.
    let mut acc0 = unsafe { vdupq_n_f32(0.0) };
    let mut acc1 = unsafe { vdupq_n_f32(0.0) };
    let mut acc2 = unsafe { vdupq_n_f32(0.0) };
    let mut acc3 = unsafe { vdupq_n_f32(0.0) };

    while a_ptr < end_main {
        // SAFETY: Four consecutive unaligned 4-element NEON loads per iteration.
        // - Condition 1: Loop condition `a_ptr < end_main` guarantees 16 elements remain before `end_main`.
        // - Condition 2: `vld1q_f32` is documented to support unaligned loads on ARM64.
        // Reason: Process 16 elements per iteration with 4 independent accumulators for ILP.
        unsafe {
            let va0 = vld1q_f32(a_ptr);
            let vb0 = vld1q_f32(b_ptr);
            acc0 = vfmaq_f32(acc0, va0, vb0);

            let va1 = vld1q_f32(a_ptr.add(4));
            let vb1 = vld1q_f32(b_ptr.add(4));
            acc1 = vfmaq_f32(acc1, va1, vb1);

            let va2 = vld1q_f32(a_ptr.add(8));
            let vb2 = vld1q_f32(b_ptr.add(8));
            acc2 = vfmaq_f32(acc2, va2, vb2);

            let va3 = vld1q_f32(a_ptr.add(12));
            let vb3 = vld1q_f32(b_ptr.add(12));
            acc3 = vfmaq_f32(acc3, va3, vb3);

            a_ptr = a_ptr.add(16);
            b_ptr = b_ptr.add(16);
        }
    }

    // SAFETY: `vaddq_f32`/`vaddvq_f32` are non-faulting register operations on aarch64.
    // - Condition 1: All four accumulator registers hold valid float32x4_t values from the loop.
    // - Condition 2: NEON is always present on aarch64; intrinsics are always available.
    // Reason: Reduce the four SIMD accumulators to a single scalar result in two steps.
    let sum01 = unsafe { vaddq_f32(acc0, acc1) };
    let sum23 = unsafe { vaddq_f32(acc2, acc3) };
    let sum = unsafe { vaddq_f32(sum01, sum23) };
    let mut result = unsafe { vaddvq_f32(sum) };

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

/// ARM NEON cosine similarity — fused: dot(a,b) / (‖a‖ × ‖b‖).
///
/// Uses the 4-accumulator dot-product kernel for vectors ≥ 64 elements.
/// Falls back to the 1-accumulator kernel for smaller vectors.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn cosine_neon(a: &[f32], b: &[f32]) -> f32 {
    let dot = dot_product_neon(a, b);
    let norm_a_sq = dot_product_neon(a, a);
    let norm_b_sq = dot_product_neon(b, b);

    if norm_a_sq == 0.0 || norm_b_sq == 0.0 {
        return 0.0;
    }

    // SAFETY: `vsqrts_f32` is a non-faulting scalar square-root instruction on aarch64.
    // - Condition 1: NEON/ASIMD is always present on aarch64; `vsqrts_f32` is always available.
    // - Condition 2: Both inputs are sums of squared floats (non-negative), so the result is well-defined.
    // Reason: Compute norms using the ARM64 hardware square-root to avoid a slow software path.
    let norm_a = unsafe { std::arch::aarch64::vsqrts_f32(norm_a_sq) };
    let norm_b = unsafe { std::arch::aarch64::vsqrts_f32(norm_b_sq) };

    dot / (norm_a * norm_b)
}

// =============================================================================
// Squared L2 Distance
// =============================================================================

/// ARM NEON squared L2 distance.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn squared_l2_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // Reason: Initialise the SIMD accumulator register to zero before the squared-diff loop.
    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32`/`vsubq_f32`/`vfmaq_f32` are non-faulting NEON operations.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, so both pointers stay within slice bounds.
        // - Condition 2: `vld1q_f32` is documented to support unaligned loads on ARM64.
        // Reason: Compute squared element-wise differences for the L2 distance accumulator.
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            let diff = vsubq_f32(va, vb);
            sum = vfmaq_f32(sum, diff, diff);
        }
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: `sum` is a valid float32x4_t value set by `vdupq_n_f32`/`vfmaq_f32`.
    // Reason: Horizontal reduction of the squared-difference accumulator to a scalar result.
    let mut result = unsafe { vaddvq_f32(sum) };

    let base = simd_len * 4;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}
