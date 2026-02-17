//! Native SIMD intrinsics for maximum performance.
//!
//! This module provides hand-tuned SIMD implementations using `core::arch` intrinsics
//! for AVX-512, AVX2, and ARM NEON architectures.
//!
//! # Module Structure
//!
//! - `scalar` — Scalar fallback implementations and fast-rsqrt helpers
//! - `tail_unroll` — Remainder/tail handling macros for SIMD loops
//! - `prefetch` — CPU cache prefetch utilities
//! - `x86_avx512` — AVX-512F kernel implementations (x86_64 only)
//! - `x86_avx2` — AVX2+FMA dot product and squared L2 kernels (x86_64 only)
//! - `x86_avx2_similarity` — AVX2+FMA cosine, Hamming, Jaccard kernels (x86_64 only)
//! - `neon` — ARM NEON kernel implementations (aarch64 only)
//! - `dispatch` — Runtime SIMD level detection and dispatch wiring
//!
//! # Performance (based on arXiv research)
//!
//! - **AVX-512**: True 16-wide f32 operations with masked remainder
//! - **AVX2**: 8-wide f32 with FMA, multi-accumulator ILP
//! - **ARM NEON**: Native 128-bit SIMD for Apple Silicon/ARM64
//! - **Prefetch**: Software prefetching for cache optimization
//!
//! # References
//!
//! - arXiv:2505.07621 "Bang for the Buck: Vector Search on Cloud CPUs"
//! - arXiv:2502.18113 "Accelerating Graph Indexing for ANNS on Modern CPUs"
#![allow(clippy::doc_markdown)] // Contains ISA/architecture nomenclature in docs.
#![allow(clippy::cast_lossless)] // Numeric widening in SIMD kernels is intentional.
#![allow(clippy::missing_panics_doc)] // Dispatch APIs assert equal vector dimensions by design.

// =============================================================================
// Shared submodules (scalar, macros, prefetch)
// =============================================================================

pub mod prefetch;
pub mod scalar;
mod tail_unroll;

// Re-export macros from tail_unroll for crate-wide use
#[allow(unused_imports)]
pub(crate) use tail_unroll::sum_remainder_unrolled_8;
#[allow(unused_imports)]
pub(crate) use tail_unroll::sum_squared_remainder_unrolled_8;

// Re-export public API from scalar
pub use scalar::{cosine_similarity_fast, fast_rsqrt};

// Re-export public API from prefetch
pub use prefetch::{
    calculate_prefetch_distance, prefetch_vector, prefetch_vector_multi_cache_line,
    L2_CACHE_LINE_BYTES,
};

// =============================================================================
// Unsafe Invariants Reference
// =============================================================================
// SAFETY: Shared invariants for SIMD unsafe blocks in this module tree.
// - Condition 1: All pointer arithmetic is derived from slice pointers with loop bounds
//   proving in-range access for each lane width.
// - Condition 2: Target-featured functions are called only after runtime feature checks
//   or on architectures where the feature is guaranteed.
// - Condition 3: Unaligned loads use `*_loadu_*`/masked-load intrinsics or equivalent
//   APIs that permit unaligned access.
// Reason: Intrinsics and pointer math are required for hot-path SIMD performance.

// =============================================================================
// ISA kernel submodules
// =============================================================================

#[cfg(target_arch = "x86_64")]
mod x86_avx512;

#[cfg(target_arch = "x86_64")]
mod x86_avx2;

#[cfg(target_arch = "x86_64")]
mod x86_avx2_similarity;

#[cfg(target_arch = "aarch64")]
mod neon;

// Re-export ISA kernels so dispatch.rs can access them via `super::`
#[cfg(target_arch = "x86_64")]
pub(crate) use x86_avx512::{
    cosine_fused_avx512, dot_product_avx512, dot_product_avx512_4acc, hamming_avx512,
    jaccard_avx512, squared_l2_avx512, squared_l2_avx512_4acc,
};

#[cfg(target_arch = "x86_64")]
pub(crate) use x86_avx2::{
    dot_product_avx2, dot_product_avx2_1acc, dot_product_avx2_4acc, squared_l2_avx2,
    squared_l2_avx2_1acc, squared_l2_avx2_4acc,
};

#[cfg(target_arch = "x86_64")]
pub(crate) use x86_avx2_similarity::{
    cosine_fused_avx2, cosine_fused_avx2_2acc, hamming_avx2, jaccard_avx2,
};

#[cfg(target_arch = "aarch64")]
pub(crate) use neon::{dot_product_neon, squared_l2_neon};

// =============================================================================
// Dispatch module (public API)
// =============================================================================

mod dispatch;

pub use dispatch::{
    batch_dot_product_native, cosine_normalized_native, cosine_similarity_native,
    dot_product_native, euclidean_native, hamming_distance_native, jaccard_similarity_native,
    norm_native, normalize_inplace_native, simd_level, squared_l2_native, warmup_simd_cache,
    DistanceEngine, SimdLevel,
};

// =============================================================================
// Tests (separate files per project rules)
// =============================================================================

#[cfg(test)]
mod simd_native_dispatch_tests;

#[cfg(test)]
mod cosine_fused_tests;

#[cfg(test)]
mod harley_seal_tests;

#[cfg(test)]
mod warmup_tests;

#[cfg(test)]
mod distance_engine_tests;
