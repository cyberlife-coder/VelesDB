//! Zero-overhead SIMD function dispatch using `OnceLock`.
//!
//! This module provides compile-time-like dispatch for SIMD functions
//! by detecting CPU features once at startup and caching function pointers.
//!
//! # Performance
//!
//! - **Zero branch overhead**: Function pointer is resolved once, called directly thereafter
//! - **No per-call checks**: Eliminates `is_x86_feature_detected!` in hot loops
//! - **Inlinable**: Function pointers can be inlined by LLVM in some cases
//!
//! # EPIC-C.2: TS-SIMD-002

use std::sync::OnceLock;

/// Type alias for distance function pointers.
type DistanceFn = fn(&[f32], &[f32]) -> f32;

/// Type alias for binary distance function pointers (returns u32).
type BinaryDistanceFn = fn(&[f32], &[f32]) -> u32;

// =============================================================================
// Static dispatch tables - initialized once on first use
// =============================================================================

/// Dispatched dot product function.
static DOT_PRODUCT_FN: OnceLock<DistanceFn> = OnceLock::new();

/// Dispatched euclidean distance function.
static EUCLIDEAN_FN: OnceLock<DistanceFn> = OnceLock::new();

/// Dispatched cosine similarity function.
static COSINE_FN: OnceLock<DistanceFn> = OnceLock::new();

/// Dispatched cosine similarity for normalized vectors.
static COSINE_NORMALIZED_FN: OnceLock<DistanceFn> = OnceLock::new();

/// Dispatched Hamming distance function.
static HAMMING_FN: OnceLock<BinaryDistanceFn> = OnceLock::new();

// =============================================================================
// Feature detection and dispatch selection
// =============================================================================

/// Selects the best dot product implementation for the current CPU.
fn select_dot_product() -> DistanceFn {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return dot_product_avx512;
        }
        if is_x86_feature_detected!("avx2") {
            return dot_product_avx2;
        }
    }
    dot_product_scalar
}

/// Selects the best euclidean distance implementation for the current CPU.
fn select_euclidean() -> DistanceFn {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return euclidean_avx512;
        }
        if is_x86_feature_detected!("avx2") {
            return euclidean_avx2;
        }
    }
    euclidean_scalar
}

/// Selects the best cosine similarity implementation for the current CPU.
fn select_cosine() -> DistanceFn {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return cosine_avx512;
        }
        if is_x86_feature_detected!("avx2") {
            return cosine_avx2;
        }
    }
    cosine_scalar
}

/// Selects the best cosine similarity (normalized) implementation.
fn select_cosine_normalized() -> DistanceFn {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return cosine_normalized_avx512;
        }
        if is_x86_feature_detected!("avx2") {
            return cosine_normalized_avx2;
        }
    }
    cosine_normalized_scalar
}

/// Selects the best Hamming distance implementation.
fn select_hamming() -> BinaryDistanceFn {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512vpopcntdq") {
            return hamming_avx512_popcnt;
        }
        if is_x86_feature_detected!("popcnt") {
            return hamming_popcnt;
        }
    }
    hamming_scalar
}

// =============================================================================
// Public dispatch API
// =============================================================================

/// Computes dot product using the best available SIMD implementation.
///
/// The implementation is selected once on first call and cached.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn dot_product_dispatched(a: &[f32], b: &[f32]) -> f32 {
    let f = DOT_PRODUCT_FN.get_or_init(select_dot_product);
    f(a, b)
}

/// Computes euclidean distance using the best available SIMD implementation.
#[inline]
#[must_use]
pub fn euclidean_dispatched(a: &[f32], b: &[f32]) -> f32 {
    let f = EUCLIDEAN_FN.get_or_init(select_euclidean);
    f(a, b)
}

/// Computes cosine similarity using the best available SIMD implementation.
#[inline]
#[must_use]
pub fn cosine_dispatched(a: &[f32], b: &[f32]) -> f32 {
    let f = COSINE_FN.get_or_init(select_cosine);
    f(a, b)
}

/// Computes cosine similarity for pre-normalized vectors.
#[inline]
#[must_use]
pub fn cosine_normalized_dispatched(a: &[f32], b: &[f32]) -> f32 {
    let f = COSINE_NORMALIZED_FN.get_or_init(select_cosine_normalized);
    f(a, b)
}

/// Computes Hamming distance using the best available implementation.
#[inline]
#[must_use]
pub fn hamming_dispatched(a: &[f32], b: &[f32]) -> u32 {
    let f = HAMMING_FN.get_or_init(select_hamming);
    f(a, b)
}

/// Returns information about which SIMD features are available.
#[must_use]
pub fn simd_features_info() -> SimdFeatures {
    SimdFeatures::detect()
}

/// Information about available SIMD features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SimdFeatures {
    /// AVX-512 foundation instructions available.
    pub avx512f: bool,
    /// AVX-512 VPOPCNTDQ (population count) available.
    pub avx512_popcnt: bool,
    /// AVX2 instructions available.
    pub avx2: bool,
    /// POPCNT instruction available.
    pub popcnt: bool,
}

impl SimdFeatures {
    /// Detects available SIMD features on the current CPU.
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self {
                avx512f: is_x86_feature_detected!("avx512f"),
                avx512_popcnt: is_x86_feature_detected!("avx512vpopcntdq"),
                avx2: is_x86_feature_detected!("avx2"),
                popcnt: is_x86_feature_detected!("popcnt"),
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                avx512f: false,
                avx512_popcnt: false,
                avx2: false,
                popcnt: false,
            }
        }
    }

    /// Returns the best available instruction set name.
    #[must_use]
    pub const fn best_instruction_set(&self) -> &'static str {
        if self.avx512f {
            "AVX-512"
        } else if self.avx2 {
            "AVX2"
        } else {
            "Scalar"
        }
    }
}

// =============================================================================
// Implementation functions - delegating to simd_avx512 and simd_explicit
// =============================================================================

// --- Dot Product implementations ---

fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(target_arch = "x86_64")]
fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_explicit::dot_product_simd(a, b)
}

#[cfg(target_arch = "x86_64")]
fn dot_product_avx512(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_avx512::dot_product_auto(a, b)
}

// --- Euclidean implementations ---

fn euclidean_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum::<f32>()
        .sqrt()
}

#[cfg(target_arch = "x86_64")]
fn euclidean_avx2(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_explicit::euclidean_distance_simd(a, b)
}

#[cfg(target_arch = "x86_64")]
fn euclidean_avx512(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_avx512::euclidean_auto(a, b)
}

// --- Cosine implementations ---

fn cosine_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom > 0.0 {
        dot / denom
    } else {
        0.0
    }
}

#[cfg(target_arch = "x86_64")]
fn cosine_avx2(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_explicit::cosine_similarity_simd(a, b)
}

#[cfg(target_arch = "x86_64")]
fn cosine_avx512(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_avx512::cosine_similarity_auto(a, b)
}

// --- Cosine Normalized implementations ---

fn cosine_normalized_scalar(a: &[f32], b: &[f32]) -> f32 {
    // For normalized vectors, cosine = dot product
    dot_product_scalar(a, b)
}

#[cfg(target_arch = "x86_64")]
fn cosine_normalized_avx2(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_explicit::dot_product_simd(a, b)
}

#[cfg(target_arch = "x86_64")]
fn cosine_normalized_avx512(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_avx512::dot_product_auto(a, b)
}

// --- Hamming implementations ---

fn hamming_scalar(a: &[f32], b: &[f32]) -> u32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    #[allow(clippy::cast_possible_truncation)]
    let count = a
        .iter()
        .zip(b.iter())
        .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
        .count() as u32;
    count
}

#[cfg(target_arch = "x86_64")]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn hamming_popcnt(a: &[f32], b: &[f32]) -> u32 {
    // Use existing implementation - safe cast as hamming distance is always positive integer
    crate::simd_explicit::hamming_distance_simd(a, b) as u32
}

/// AVX-512 VPOPCNTDQ implementation for Hamming distance.
///
/// Uses AVX-512 VPOPCNTDQ instruction when available for ~2x speedup
/// over regular POPCNT on large vectors.
///
/// # Safety
///
/// This function uses runtime CPU feature detection via `is_x86_feature_detected!`.
/// Falls back to regular POPCNT if VPOPCNTDQ is not available.
#[cfg(target_arch = "x86_64")]
fn hamming_avx512_popcnt(a: &[f32], b: &[f32]) -> u32 {
    // Runtime detection of AVX-512 VPOPCNTDQ
    // This instruction is available on Ice Lake+ and Zen 4+ CPUs
    #[cfg(target_feature = "avx512vpopcntdq")]
    {
        // SAFETY: Feature is compile-time enabled
        unsafe { hamming_avx512_vpopcntdq_impl(a, b) }
    }

    #[cfg(not(target_feature = "avx512vpopcntdq"))]
    {
        // Runtime detection fallback
        if is_x86_feature_detected!("avx512vpopcntdq") && is_x86_feature_detected!("avx512f") {
            // SAFETY: Runtime check passed
            unsafe { hamming_avx512_vpopcntdq_impl(a, b) }
        } else {
            // Fallback to regular POPCNT
            hamming_popcnt(a, b)
        }
    }
}

/// AVX-512 VPOPCNTDQ inner implementation.
///
/// # Safety
///
/// Caller must ensure AVX-512F and AVX-512VPOPCNTDQ are available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f", enable = "avx512vpopcntdq")]
unsafe fn hamming_avx512_vpopcntdq_impl(a: &[f32], b: &[f32]) -> u32 {
    use std::arch::x86_64::*;

    debug_assert_eq!(a.len(), b.len());
    let len = a.len();

    // Reinterpret f32 slices as u32 for bit operations
    let a_bits = std::slice::from_raw_parts(a.as_ptr() as *const u32, len);
    let b_bits = std::slice::from_raw_parts(b.as_ptr() as *const u32, len);

    let mut total: u64 = 0;
    let chunks = len / 16; // Process 16 u32 (512 bits) at a time

    for i in 0..chunks {
        let base = i * 16;

        // Load 512 bits (16 x u32) from each vector
        let va = _mm512_loadu_si512(a_bits.as_ptr().add(base) as *const __m512i);
        let vb = _mm512_loadu_si512(b_bits.as_ptr().add(base) as *const __m512i);

        // XOR to get differing bits
        let xor_result = _mm512_xor_si512(va, vb);

        // VPOPCNTDQ: count set bits in each 64-bit element (8 elements)
        let popcnt = _mm512_popcnt_epi64(xor_result);

        // Horizontal sum of 8 x u64 popcount values
        total += _mm512_reduce_add_epi64(popcnt) as u64;
    }

    // Handle remainder with scalar POPCNT
    let remainder_start = chunks * 16;
    for i in remainder_start..len {
        let xor = a_bits[i] ^ b_bits[i];
        total += xor.count_ones() as u64;
    }

    total as u32
}

// =============================================================================
// Prefetch constants - EPIC-C.1
// =============================================================================

/// Cache line size in bytes (standard for modern x86/ARM CPUs).
pub const CACHE_LINE_SIZE: usize = 64;

/// Prefetch distance for 768-dimensional vectors (3072 bytes).
/// Calculated at compile time: `768 * 4 / 64 = 48` cache lines.
pub const PREFETCH_DISTANCE_768D: usize = 768 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Prefetch distance for 384-dimensional vectors.
pub const PREFETCH_DISTANCE_384D: usize = 384 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Prefetch distance for 1536-dimensional vectors.
pub const PREFETCH_DISTANCE_1536D: usize = 1536 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Calculates prefetch distance for a given dimension at compile time.
#[inline]
#[must_use]
pub const fn prefetch_distance(dimension: usize) -> usize {
    (dimension * std::mem::size_of::<f32>()) / CACHE_LINE_SIZE
}

// =============================================================================
