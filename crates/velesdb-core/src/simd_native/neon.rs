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
    // SAFETY: NEON intrinsics are always safe on aarch64.
    // Reason: NEON SIMD operations are the primary compute mechanism on ARM64.
    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: offset + 4 <= len, vld1q_f32 handles unaligned loads safely on ARM64
        // Reason: Core NEON computation for dot product accumulation.
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            sum = vfmaq_f32(sum, va, vb);
        }
    }

    // SAFETY: vaddvq_f32 is always safe on aarch64.
    // Reason: Horizontal reduction to scalar result.
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
    // SAFETY: Pointer arithmetic stays within slice bounds.
    let end_main = unsafe { a.as_ptr().add(len / 16 * 16) };
    let end_ptr = unsafe { a.as_ptr().add(len) };

    // SAFETY: vdupq_n_f32 is always safe on aarch64.
    let mut acc0 = unsafe { vdupq_n_f32(0.0) };
    let mut acc1 = unsafe { vdupq_n_f32(0.0) };
    let mut acc2 = unsafe { vdupq_n_f32(0.0) };
    let mut acc3 = unsafe { vdupq_n_f32(0.0) };

    while a_ptr < end_main {
        // SAFETY: Loop condition ensures 16 elements available; vld1q_f32 is unaligned-safe.
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

    // SAFETY: vaddq_f32/vaddvq_f32 always safe on aarch64.
    let sum01 = unsafe { vaddq_f32(acc0, acc1) };
    let sum23 = unsafe { vaddq_f32(acc2, acc3) };
    let sum = unsafe { vaddq_f32(sum01, sum23) };
    let mut result = unsafe { vaddvq_f32(sum) };

    while a_ptr < end_ptr {
        // SAFETY: Loop condition ensures pointer is within slice bounds.
        unsafe {
            result += *a_ptr * *b_ptr;
            a_ptr = a_ptr.add(1);
            b_ptr = b_ptr.add(1);
        }
    }

    result
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

    // SAFETY: NEON intrinsics are always safe on aarch64.
    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: offset + 4 <= len, vld1q_f32 handles unaligned loads safely.
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            let diff = vsubq_f32(va, vb);
            sum = vfmaq_f32(sum, diff, diff);
        }
    }

    // SAFETY: vaddvq_f32 is always safe on aarch64.
    let mut result = unsafe { vaddvq_f32(sum) };

    let base = simd_len * 4;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}
