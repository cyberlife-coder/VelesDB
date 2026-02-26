//! AVX2+FMA kernel implementations for x86_64.
//!
//! This module re-exports dot product and squared L2 distance kernels
//! from dedicated sub-files for maintainability (< 500 lines each).
//!
//! All functions require runtime AVX2+FMA detection before calling.
//! Dispatch is handled by `dispatch.rs` after `simd_level()` confirms support.

mod dot;
mod l2;

pub(crate) use dot::{dot_product_avx2, dot_product_avx2_1acc, dot_product_avx2_4acc};
pub(crate) use l2::{squared_l2_avx2, squared_l2_avx2_1acc, squared_l2_avx2_4acc};
