//! `bounds_16wide`'s tail pointer is read past the main body (#1965).
//!
//! Natively, any pointer with the right address passes this test. Under Miri's
//! Stacked Borrows it does not: a tail pointer derived from the subslice
//! `a[..main]` may not be read past that subslice, whatever its address, and
//! the scalar tail reads exactly there. That is the regression this pins. It
//! calls no NEON intrinsic, so Miri can run it
//! (`cargo miri test --target aarch64-unknown-linux-gnu -p velesdb-core --lib -- neon_bounds`).

use super::{bounds_16wide, cosine_fused_neon_scalar_tail};

#[test]
fn test_bounds_16wide_tail_pointer_reads_the_whole_tail() {
    for len in [0u16, 1, 15, 16, 17, 65] {
        let a: Vec<f32> = (0..len).map(f32::from).collect();
        let (main, end_main, end_ptr) = bounds_16wide(&a);
        assert_eq!(main, usize::from(len) / 16 * 16);

        // SAFETY: the tail reads `a` from `end_main` to `end_ptr`, and `b` over as many elements.
        // - Condition 1: both pointers come from `bounds_16wide(&a)`, so they span `a[main..]`.
        // - Condition 2: `b` is `a` itself, so `a.as_ptr().add(main)` is readable for as many.
        // Reason: exercise the exact pointer the 16-wide kernels hand to their tail.
        let (dot, norm_a_sq, norm_b_sq) = unsafe {
            cosine_fused_neon_scalar_tail(end_main, a.as_ptr().add(main), end_ptr, 0.0, 0.0, 0.0)
        };

        // Accumulated from 0.0 in order, as the tail does (`sum` starts at -0.0).
        let expected = a[main..].iter().fold(0.0_f32, |acc, x| acc + x * x);
        for got in [dot, norm_a_sq, norm_b_sq] {
            assert_eq!(got.to_bits(), expected.to_bits(), "len {len}");
        }
    }
}
