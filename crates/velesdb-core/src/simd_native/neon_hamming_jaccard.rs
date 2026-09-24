//! ARM NEON Hamming and Jaccard kernel implementations for aarch64.
//!
//! Split from `neon.rs` the way `x86_avx2_similarity.rs` is split from
//! `x86_avx2/`: these kernels share no helper with the dot, cosine and L2
//! ones, so they live apart. 1-acc and 4-acc variants for Hamming (thresholded
//! `f32` and packed `u64`) and Jaccard.
//!
//! NEON is always available on aarch64, so no runtime detection is needed.

// Reason: Numeric casts in this file are intentional and safe:
// - All casts are from well-bounded values (vector dimensions, loop indices)
// - All casts are validated by extensive SIMD tests (simd_native_tests.rs)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::missing_panics_doc)]
// Wildcard import of NEON intrinsics is idiomatic for SIMD kernels.
#![allow(clippy::wildcard_imports)]
#![allow(clippy::similar_names)]

// =============================================================================

// =============================================================================
// Hamming Distance
// =============================================================================

/// ARM NEON Hamming distance with adaptive accumulator selection.
///
/// Computes the number of positions where binary-thresholded values differ
/// (threshold at 0.5), consistent with AVX2/AVX-512 Hamming kernels.
/// For vectors with >= 64 elements, delegates to [`hamming_neon_4acc`] which
/// uses 4-way ILP for higher throughput.
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
/// Every public `simd_native` entry point asserts it, in release too, before
/// dispatching here.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) unsafe fn hamming_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        // SAFETY: `hamming_neon_4acc` reads `b` at every index of `a`.
        // - Condition 1: this function's own `# Safety` precondition, `a.len() == b.len()`.
        // - Condition 2: NEON is always present on aarch64.
        // Reason: from 64 elements the 4-accumulator loop is faster; it has no length floor.
        return unsafe { hamming_neon_4acc(a, b) };
    }
    // SAFETY: `hamming_neon_1acc` requires NEON (guaranteed on aarch64).
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: this function's own `# Safety` precondition, `a.len() == b.len()`.
    // SAFETY: Single-accumulator variant for small/medium vectors.
    unsafe { hamming_neon_1acc(a, b) }
}

/// Single-accumulator NEON Hamming distance for vectors with < 64 elements.
///
/// Binary-thresholds each lane at 0.5, XORs the masks, and counts differing
/// positions. Accumulates in `uint32x4_t` for exact integer precision.
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn hamming_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_u32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0 is a valid u32 constant accepted by the instruction.
    // SAFETY: Initialise the SIMD diff-count accumulator to zero before the reduction loop.
    let mut diff_count = vdupq_n_u32(0);

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate value 0.5 is a valid f32 constant.
    // SAFETY: Create threshold vector for binary comparison.
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
        // SAFETY: Binary-threshold each lane, XOR masks, shift to 0/1, accumulate count.
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
    // SAFETY: Horizontal reduction of the diff-count accumulator to a scalar result.
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
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn hamming_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let main_end = len / 16 * 16;

    // SAFETY: `vdupq_n_u32` / `vdupq_n_f32` are non-faulting register initialisations.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate values 0 / 0.5 are valid constants.
    // SAFETY: Initialise 4 diff-count accumulators and threshold vector.
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
        // SAFETY: 16-wide single-pass binary comparison with 4-way ILP.

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
    // SAFETY: Reduce 4 accumulators to scalar diff count.
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
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked
/// (the `i + 2 <= len` guard bounds `a` only). `hamming_binary_native`
/// asserts it, in release too, before dispatching here. NEON itself is always
/// available on aarch64.
#[cfg(target_arch = "aarch64")]
pub(crate) unsafe fn hamming_binary_neon(a: &[u64], b: &[u64]) -> u32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let mut total: u32 = 0;
    let mut i = 0;

    // Process 2 u64 (128 bits) per iteration using vcntq_u8 + horizontal sum
    while i + 2 <= len {
        // SAFETY: `vld1q_u64` loads 2 u64 from an unaligned address on aarch64.
        // - Condition 1: `i + 2 <= len`, so both pointers stay within slice bounds.
        // - Condition 2: `vld1q_u64` supports unaligned loads on ARM64.
        // SAFETY: NEON XOR + byte-popcount for binary Hamming distance.
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
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
/// Every public `simd_native` entry point asserts it, in release too, before
/// dispatching here.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) unsafe fn jaccard_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        // SAFETY: `jaccard_neon_4acc` reads `b` at every index of `a`.
        // - Condition 1: this function's own `# Safety` precondition, `a.len() == b.len()`.
        // - Condition 2: NEON is always present on aarch64.
        // Reason: from 64 elements the 4-accumulator loop is faster; it has no length floor.
        return unsafe { jaccard_neon_4acc(a, b) };
    }
    // SAFETY: `jaccard_neon_1acc` requires NEON (guaranteed on aarch64).
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: this function's own `# Safety` precondition, `a.len() == b.len()`.
    // SAFETY: Single-accumulator variant for small/medium vectors.
    unsafe { jaccard_neon_1acc(a, b) }
}

/// Single-accumulator NEON Jaccard similarity for vectors with < 64 elements.
///
/// Accumulates `min(a, b)` for intersection and `max(a, b)` for union in
/// `float32x4_t` registers, then horizontally reduces.
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn jaccard_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // SAFETY: Initialise intersection and union SIMD accumulators to zero.
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
        // SAFETY: Accumulate min (intersection) and max (union) per 4-element block.
        let va = vld1q_f32(a_ptr.add(offset));
        let vb = vld1q_f32(b_ptr.add(offset));
        inter_acc = vaddq_f32(inter_acc, vminq_f32(va, vb));
        union_acc = vaddq_f32(union_acc, vmaxq_f32(va, vb));
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: Both accumulators are valid float32x4_t values.
    // SAFETY: Horizontal reduction of intersection and union accumulators.
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
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn jaccard_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let main_end = len / 16 * 16;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate value 0.0 is valid for the instruction.
    // SAFETY: Initialise 8 accumulators (4 intersection + 4 union) for ILP.
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
        // SAFETY: 16-wide single-pass min/max accumulation with 4-way ILP.

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
    // SAFETY: Reduce 4 intersection and 4 union accumulators to scalar results.
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
