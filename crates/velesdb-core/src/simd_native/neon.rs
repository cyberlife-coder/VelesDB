//! ARM NEON kernel implementations for aarch64.
//!
//! Contains hand-tuned NEON SIMD kernels for dot product, cosine similarity and
//! squared L2 distance, with 1-acc and 4-acc variants for different vector
//! sizes. Hamming and Jaccard live in `neon_hamming_jaccard.rs`.
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
// Dot Product
// =============================================================================

/// ARM NEON dot product with 4 accumulators for ILP optimization (EPIC-052/US-009).
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
/// Every public `simd_native` entry point asserts it, in release too, before
/// dispatching here.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) unsafe fn dot_product_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();

    if len >= 64 {
        // SAFETY: `dot_product_neon_4acc` reads `b` at every index of `a`.
        // - Condition 1: this function's own `# Safety` precondition, `a.len() == b.len()`.
        // Reason: the 4-accumulator loop for vectors of 64 elements or more.
        return unsafe { dot_product_neon_4acc(a, b) };
    }

    let simd_len = len / 4;
    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // SAFETY: Initialise the SIMD accumulator register to zero before the reduction loop.
    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32` loads 4 f32 values from an unaligned address on aarch64.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, so both pointers stay within slice bounds.
        // - Condition 2: `vld1q_f32` is documented to support unaligned loads on ARM64.
        // SAFETY: Core NEON computation for dot product accumulation per 4-element block.
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            sum = vfmaq_f32(sum, va, vb);
        }
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: `sum` is a valid float32x4_t value set by `vdupq_n_f32`/`vfmaq_f32`.
    // SAFETY: Horizontal reduction of the SIMD accumulator to a scalar dot-product result.
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
/// [`simd_4acc_dot_loop!`]: crate::simd_4acc_dot_loop!
///
/// # Safety
///
/// None beyond NEON, which aarch64 always has: `vfmaq_f32` is a
/// non-faulting register operation, with no memory access.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn neon_fma_compat(
    a: std::arch::aarch64::float32x4_t,
    b: std::arch::aarch64::float32x4_t,
    acc: std::arch::aarch64::float32x4_t,
) -> std::arch::aarch64::float32x4_t {
    std::arch::aarch64::vfmaq_f32(acc, a, b)
}

/// Loop bounds of the 16-wide 4-accumulator kernels: the length of `a`'s main
/// body (`len / 16 * 16` elements), the pointer where it ends and the scalar
/// tail starts, and `a`'s one-past-the-end pointer, where the tail stops.
///
/// Both pointers are derived from `a` itself, not from a subslice: the tail
/// dereferences from the first one on, which a pointer derived from `a[..main]`
/// may not do (it would read past its own borrow, whatever the address).
/// `wrapping_add` is safe code and keeps `a`'s provenance; `main <= len`, so
/// the result stays within `a` or at its end.
#[cfg(target_arch = "aarch64")]
#[inline]
fn bounds_16wide(a: &[f32]) -> (usize, *const f32, *const f32) {
    let main = a.len() / 16 * 16;
    let range = a.as_ptr_range();
    (main, range.start.wrapping_add(main), range.end)
}

/// ARM NEON dot product with 4 accumulators for large vectors.
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
/// Every public `simd_native` entry point asserts it, in release too, before
/// dispatching here.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn dot_product_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let (_, end_main, end_ptr) = bounds_16wide(a);

    // SAFETY: 4-accumulator ILP loop of 16-wide NEON loads from `a` and `b`.
    // - Condition 1: `end_main` bounds every load of `a`; `b` is read at the same
    //   offsets and is as long as `a` (this function's `# Safety` precondition).
    // Reason: `neon_fma_compat` reorders args to match the macro's convention.
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
    // - Condition 1: NEON is always present on aarch64; `vaddvq_f32` is guaranteed available.
    // SAFETY: Horizontal reduction of the combined SIMD accumulator to a scalar result.
    let mut result = unsafe { vaddvq_f32(combined) };

    while a_ptr < end_ptr {
        // SAFETY: Raw pointer dereference for scalar tail processing.
        // - Condition 1: Loop condition `a_ptr < end_ptr` guarantees both pointers are within slice bounds.
        // - Condition 2: `b_ptr` advances in step with `a_ptr`, and this function's
        //   `# Safety` precondition, `a.len() == b.len()`, keeps it within `b`.
        // SAFETY: Handle the remaining 0-15 elements that the 16-wide SIMD loop did not cover.
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
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
/// Every public `simd_native` entry point asserts it, in release too, before
/// dispatching here.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) unsafe fn cosine_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        // SAFETY: `cosine_fused_neon_4acc` reads `b` at every index of `a`.
        // - Condition 1: this function's own `# Safety` precondition, `a.len() == b.len()`.
        // - Condition 2: NEON is always present on aarch64.
        // Reason: from 64 elements the 4-accumulator loop is faster; it has no length floor.
        return unsafe { cosine_fused_neon_4acc(a, b) };
    }
    // SAFETY: `cosine_fused_neon_1acc` requires NEON (guaranteed on aarch64).
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: this function's own `# Safety` precondition, `a.len() == b.len()`.
    // SAFETY: Single-accumulator variant for small/medium vectors.
    unsafe { cosine_fused_neon_1acc(a, b) }
}

/// Single-accumulator fused cosine for vectors with < 64 elements.
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn cosine_fused_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: Immediate value 0.0 is valid for the instruction.
    // SAFETY: Initialise three SIMD accumulators (dot, norm_a, norm_b).
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
        // SAFETY: Single-pass accumulation of dot, norm_a_sq, norm_b_sq.
        let va = vld1q_f32(a_ptr.add(offset));
        let vb = vld1q_f32(b_ptr.add(offset));
        dot_acc = vfmaq_f32(dot_acc, va, vb);
        na_acc = vfmaq_f32(na_acc, va, va);
        nb_acc = vfmaq_f32(nb_acc, vb, vb);
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to scalar on aarch64.
    // - Condition 1: All accumulators are valid float32x4_t values.
    // SAFETY: Horizontal reduction of the three accumulators.
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
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn cosine_fused_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    let (main_end, end_main, end_ptr) = bounds_16wide(a);

    // SAFETY: both helpers read `b` over the span they read of `a`.
    // - Condition 1: `a.as_ptr()..end_main` is `a`'s first `main_end` elements,
    //   a multiple of 16, and `end_main..end_ptr` the rest of `a`; every pointer
    //   is derived from `a` itself (`bounds_16wide`), so the tail may read it.
    // - Condition 2: this function's own `# Safety` precondition,
    //   `a.len() == b.len()`, so `b` is readable over both spans, and
    //   `b.as_ptr().add(main_end)` stays within `b`.
    // Reason: the 16-wide main body, then the scalar tail.
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
/// # Safety
///
/// None beyond NEON, which aarch64 always has: register additions only,
/// with no memory access.
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
    // SAFETY: Reduce 4 accumulators to scalar result.
    let ab01 = vaddq_f32(a0, a1);
    let ab23 = vaddq_f32(a2, a3);
    vaddvq_f32(vaddq_f32(ab01, ab23))
}

/// Main 16-wide SIMD loop for fused cosine (4-acc ILP).
///
/// Returns `(dot, norm_a_sq, norm_b_sq)` accumulated over full 16-element blocks.
///
/// # Safety
///
/// `a_ptr..end_main` is readable and spans a multiple of 16 elements, and
/// `b_ptr` is readable for as many elements.
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
    // SAFETY: Initialise 12 accumulators (3 products x 4-way ILP).
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
        // - Condition 2: this function's `# Safety`: the span is a multiple of 16,
        //   so 16 elements remain, and `b_ptr` is readable for as many elements.
        // SAFETY: 16-wide single-pass accumulation with 4-way ILP.
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

    // SAFETY: `reduce_4acc_neon` only adds registers.
    // - Condition 1: its `# Safety` asks for nothing beyond NEON, always present on aarch64.
    // Reason: collapse the 12 accumulators into (dot, norm_a_sq, norm_b_sq).
    (
        reduce_4acc_neon(d0, d1, d2, d3),
        reduce_4acc_neon(na0, na1, na2, na3),
        reduce_4acc_neon(nb0, nb1, nb2, nb3),
    )
}

/// Scalar tail for fused cosine — handles the remaining 0..15 elements.
///
/// # Safety
///
/// `a_ptr..end_ptr` is readable, and `b_ptr` is readable for as many elements.
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
        // - Condition 2: this function's `# Safety`: `b_ptr` is readable for as
        //   many elements as `a_ptr..end_ptr`.
        // SAFETY: Handle remaining elements the 16-wide loop did not cover.
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
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
/// Every public `simd_native` entry point asserts it, in release too, before
/// dispatching here.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) unsafe fn squared_l2_neon(a: &[f32], b: &[f32]) -> f32 {
    if a.len() >= 64 {
        // SAFETY: `squared_l2_neon_4acc` reads `b` at every index of `a`.
        // - Condition 1: this function's own `# Safety` precondition, `a.len() == b.len()`.
        // Reason: the 4-accumulator loop for vectors of 64 elements or more.
        return unsafe { squared_l2_neon_4acc(a, b) };
    }
    // SAFETY: `squared_l2_neon_1acc` requires NEON (guaranteed on aarch64).
    // - Condition 1: NEON is always present on aarch64.
    // - Condition 2: this function's own `# Safety` precondition, `a.len() == b.len()`.
    // SAFETY: Single-accumulator variant for small/medium vectors.
    unsafe { squared_l2_neon_1acc(a, b) }
}

/// Single-accumulator NEON squared L2 distance for vectors with < 64 elements.
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn squared_l2_neon_1acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    // SAFETY: `vdupq_n_f32` is a non-faulting register initialisation on aarch64.
    // - Condition 1: NEON is always present on aarch64; no runtime detection needed.
    // - Condition 2: Immediate value 0.0 is a valid f32 constant accepted by the instruction.
    // SAFETY: Initialise the SIMD accumulator register to zero before the squared-diff loop.
    let mut sum = vdupq_n_f32(0.0);

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        // SAFETY: `vld1q_f32`/`vsubq_f32`/`vfmaq_f32` are non-faulting NEON operations.
        // - Condition 1: `offset + 4 <= simd_len * 4 <= len`, so both pointers stay within slice bounds.
        // - Condition 2: `vld1q_f32` is documented to support unaligned loads on ARM64.
        // SAFETY: Compute squared element-wise differences for the L2 distance accumulator.
        let va = vld1q_f32(a_ptr.add(offset));
        let vb = vld1q_f32(b_ptr.add(offset));
        let diff = vsubq_f32(va, vb);
        sum = vfmaq_f32(sum, diff, diff);
    }

    // SAFETY: `vaddvq_f32` reduces a 128-bit register to a scalar f32 on aarch64.
    // - Condition 1: NEON is always present on aarch64; intrinsic is always available.
    // - Condition 2: `sum` is a valid float32x4_t value set by `vdupq_n_f32`/`vfmaq_f32`.
    // SAFETY: Horizontal reduction of the squared-difference accumulator to a scalar result.
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
///
/// [`simd_4acc_l2_loop!`]: crate::simd_4acc_l2_loop!
///
/// # Safety
///
/// `a.len() == b.len()`: the loads read `b` at every index of `a`, unchecked.
/// Every public `simd_native` entry point asserts it, in release too, before
/// dispatching here.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn squared_l2_neon_4acc(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let (_, end_main, end_ptr) = bounds_16wide(a);

    // SAFETY: 4-accumulator ILP loop of 16-wide NEON loads from `a` and `b`.
    // - Condition 1: `end_main` bounds every load of `a`; `b` is read at the same
    //   offsets and is as long as `a` (this function's `# Safety` precondition).
    // Reason: `vsubq_f32` takes the element-wise difference, then `neon_fma_compat`
    // (args reordered to the macro's convention) accumulates diff².
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
    // - Condition 1: NEON is always present on aarch64; `vaddvq_f32` is guaranteed available.
    // SAFETY: Horizontal reduction of the combined SIMD accumulator to a scalar result.
    let mut result = unsafe { vaddvq_f32(combined) };

    while a_ptr < end_ptr {
        // SAFETY: Raw pointer dereference for scalar tail processing.
        // - Condition 1: Loop condition `a_ptr < end_ptr` guarantees both pointers are within slice bounds.
        // - Condition 2: `b_ptr` advances in step with `a_ptr`, and this function's
        //   `# Safety` precondition, `a.len() == b.len()`, keeps it within `b`.
        // SAFETY: Handle the remaining 0-15 elements that the 16-wide SIMD loop did not cover.
        unsafe {
            let d = *a_ptr - *b_ptr;
            result += d * d;
            a_ptr = a_ptr.add(1);
            b_ptr = b_ptr.add(1);
        }
    }

    result
}

#[cfg(test)]
#[path = "neon_bounds_tests.rs"]
mod neon_bounds_tests;
