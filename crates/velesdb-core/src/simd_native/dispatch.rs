//! Runtime SIMD level detection and dispatch wiring.
//!
//! This module provides:
//! - `SimdLevel` enum for representing detected SIMD capability
//! - `simd_level()` for cached runtime detection
//! - `warmup_simd_cache()` for eliminating cold-start latency
//! - All public dispatch functions that route to ISA-specific kernels

use super::scalar;

// =============================================================================
// Cached SIMD Level Detection (EPIC-033 US-002)
// =============================================================================

/// SIMD capability level detected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    /// AVX-512F available (`x86_64` only).
    Avx512,
    /// AVX2 + FMA available (`x86_64` only).
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

/// Warms up SIMD caches to eliminate cold-start latency.
///
/// Call this at application startup to ensure the first SIMD operations
/// are as fast as subsequent ones. This is particularly important for
/// latency-sensitive applications like real-time vector search.
///
/// # Example
///
/// ```
/// use velesdb_core::simd_native::warmup_simd_cache;
///
/// // Call once at startup
/// warmup_simd_cache();
/// ```
#[inline]
pub fn warmup_simd_cache() {
    // Force SIMD level detection
    let _ = simd_level();

    // Warm up CPU caches with dummy operations
    // Using 768D as it's a common embedding dimension
    let warmup_size = 768;
    let a: Vec<f32> = vec![0.01; warmup_size];
    let b: Vec<f32> = vec![0.01; warmup_size];

    // 3 iterations as recommended by SimSIMD research
    for _ in 0..3 {
        let _ = dot_product_native(&a, &b);
        let _ = cosine_similarity_native(&a, &b);
    }
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
///
/// # Panics
///
/// Panics when `a.len() != b.len()`.
#[allow(clippy::inline_always)] // Reason: Hot-path function called millions of times in similarity search loops
#[inline(always)]
#[must_use]
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    match simd_level() {
        // AVX-512: 4-acc pour très grands vecteurs, 1-acc pour le reste
        #[cfg(target_arch = "x86_64")]
        // SAFETY: AVX-512 dispatch after runtime feature detection.
        // Reason: Target-feature function is safe after simd_level() confirms AVX-512F support.
        SimdLevel::Avx512 if a.len() >= 512 => unsafe { super::dot_product_avx512_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: AVX-512 dispatch after runtime feature detection.
        // Reason: Target-feature function is safe after simd_level() confirms AVX-512F support.
        SimdLevel::Avx512 if a.len() >= 16 => unsafe { super::dot_product_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: AVX-512 dispatch after runtime feature detection.
        // - Condition 1: simd_level() confirms AVX-512F support via cached runtime detection
        // - Condition 2: Masked loads handle vectors <16 elements safely
        // Reason: Target-feature function is safe after runtime detection confirms AVX-512F.
        SimdLevel::Avx512 => unsafe { super::dot_product_avx512(a, b) },
        // AVX2: seuils optimisés basés sur la recherche
        #[cfg(target_arch = "x86_64")]
        // SAFETY: AVX2 dispatch after runtime feature detection.
        // Reason: Target-feature function is safe after simd_level() confirms AVX2+FMA support.
        SimdLevel::Avx2 if a.len() >= 256 => unsafe { super::dot_product_avx2_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: AVX2 dispatch after runtime feature detection.
        // Reason: Target-feature function is safe after simd_level() confirms AVX2+FMA support.
        SimdLevel::Avx2 if a.len() >= 64 => unsafe { super::dot_product_avx2(a, b) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: AVX2 dispatch after runtime feature detection.
        // Reason: Target-feature function is safe after simd_level() confirms AVX2+FMA support.
        SimdLevel::Avx2 if a.len() >= 16 => unsafe { super::dot_product_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: AVX2 dispatch after runtime feature detection.
        // Reason: Target-feature function is safe after simd_level() confirms AVX2+FMA support.
        SimdLevel::Avx2 if a.len() >= 8 => unsafe { super::dot_product_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 => a.iter().zip(b.iter()).map(|(x, y)| x * y).sum(),
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if a.len() >= 4 => super::dot_product_neon(a, b),
        _ => a.iter().zip(b.iter()).map(|(x, y)| x * y).sum(),
    }
}

/// Squared L2 distance with automatic dispatch to best available SIMD.
///
/// # Panics
///
/// Panics when `a.len() != b.len()`.
#[allow(clippy::inline_always)] // Reason: Hot-path function for Euclidean distance calculations
#[inline(always)]
#[must_use]
pub fn squared_l2_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 512 => unsafe { super::squared_l2_avx512_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 if a.len() >= 16 => unsafe { super::squared_l2_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx512 => unsafe { super::squared_l2_avx512(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 256 => unsafe { super::squared_l2_avx2_4acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 64 => unsafe { super::squared_l2_avx2(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 16 => unsafe { super::squared_l2_avx2_1acc(a, b) },
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 if a.len() >= 8 => unsafe { super::squared_l2_avx2_1acc(a, b) },
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
        SimdLevel::Neon if a.len() >= 4 => super::squared_l2_neon(a, b),
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
#[allow(clippy::inline_always)] // Reason: Thin wrapper over squared_l2_native, must inline
#[inline(always)]
#[must_use]
pub fn euclidean_native(a: &[f32], b: &[f32]) -> f32 {
    squared_l2_native(a, b).sqrt()
}

/// L2 norm with automatic dispatch to best available SIMD.
#[allow(clippy::inline_always)] // Reason: Thin wrapper over dot_product_native, must inline
#[inline(always)]
#[must_use]
pub fn norm_native(v: &[f32]) -> f32 {
    dot_product_native(v, v).sqrt()
}

/// Normalizes a vector in-place using native SIMD.
#[allow(clippy::inline_always)] // Reason: Called in tight loops during vector normalization
#[inline(always)]
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
#[allow(clippy::inline_always)] // Reason: Thin wrapper, direct delegation to dot_product_native
#[inline(always)]
#[must_use]
pub fn cosine_normalized_native(a: &[f32], b: &[f32]) -> f32 {
    dot_product_native(a, b)
}

/// Full cosine similarity (with normalization) using native SIMD.
///
/// # Panics
///
/// Panics when `a.len() != b.len()`.
#[allow(clippy::inline_always)] // Reason: Primary similarity function, hot-path in all vector searches
#[inline(always)]
#[must_use]
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    #[cfg(target_arch = "x86_64")]
    {
        match simd_level() {
            SimdLevel::Avx512 if a.len() >= 16 => {
                return unsafe { super::cosine_fused_avx512(a, b) }
            }
            SimdLevel::Avx2 if a.len() >= 1024 => return unsafe { super::cosine_fused_avx2(a, b) },
            SimdLevel::Avx2 if a.len() >= 64 => {
                return unsafe { super::cosine_fused_avx2_2acc(a, b) }
            }
            SimdLevel::Avx2 if a.len() >= 8 => return unsafe { super::cosine_fused_avx2(a, b) },
            _ => {}
        }
    }

    // Scalar fallback
    scalar::cosine_scalar(a, b)
}

/// Batch dot products with prefetching.
///
/// Computes dot products between a query and multiple candidates,
/// using software prefetch hints for cache optimization.
#[inline]
#[must_use]
pub fn batch_dot_product_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        // Prefetch ahead for cache warming
        #[cfg(target_arch = "x86_64")]
        if i + 4 < candidates.len() {
            // SAFETY: _mm_prefetch is a hint instruction that cannot cause memory faults.
            // - Condition 1: The pointer is derived from a valid slice reference
            // - Condition 2: Prefetch instructions are hints and never fault, even with invalid addresses
            // - Condition 3: Index i + 4 is bounds-checked by the condition above
            // Reason: Software prefetching for cache optimization before actual data access.
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                _mm_prefetch(candidates[i + 4].as_ptr().cast::<i8>(), _MM_HINT_T0);
            }
        }

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
/// # Panics
///
/// Panics when `a.len() != b.len()`.
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
/// # Panics
///
/// Panics when `a.len() != b.len()`.
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
            return unsafe { super::hamming_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            return unsafe { super::hamming_avx2(a, b) };
        }
    }
    scalar::hamming_scalar(a, b)
}

/// SIMD Jaccard similarity with runtime dispatch.
#[inline]
fn jaccard_simd(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            return unsafe { super::jaccard_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && a.len() >= 8 {
            return unsafe { super::jaccard_avx2(a, b) };
        }
    }
    scalar::jaccard_scalar(a, b)
}

// =============================================================================
// DistanceEngine — Zero-overhead SIMD dispatch via cached function pointers
// =============================================================================

/// Scalar fallback for dot product.
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Scalar fallback for squared L2 distance.
fn squared_l2_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

/// Zero-overhead SIMD dispatch via cached function pointers.
///
/// Eliminates per-call match dispatch by resolving the best SIMD kernel
/// once at construction time for a given vector dimension. Use for hot loops
/// (HNSW search, batch similarity operations) where the same dimension is
/// used repeatedly.
///
/// # Design
///
/// - Uses bare `fn` pointers (not `dyn Fn`) for zero-cost indirection
/// - `Send + Sync + Copy` for easy sharing across threads
/// - Backward compatible: existing `*_native()` functions are unchanged
///
/// # Example
///
/// ```
/// use velesdb_core::simd_native::DistanceEngine;
///
/// let engine = DistanceEngine::new(768);
/// let a = vec![0.1f32; 768];
/// let b = vec![0.2f32; 768];
/// let score = engine.dot_product(&a, &b);
/// ```
///
/// # References
///
/// - arXiv:2505.07621 "Bang for the Buck" — function pointer dispatch for SIMD
/// - `SimSIMD` library uses similar function-pointer tables
#[derive(Clone, Copy)]
pub struct DistanceEngine {
    dot_product_fn: fn(&[f32], &[f32]) -> f32,
    squared_l2_fn: fn(&[f32], &[f32]) -> f32,
    cosine_fn: fn(&[f32], &[f32]) -> f32,
    hamming_fn: fn(&[f32], &[f32]) -> f32,
    jaccard_fn: fn(&[f32], &[f32]) -> f32,
    dimension: usize,
}

impl std::fmt::Debug for DistanceEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DistanceEngine")
            .field("dimension", &self.dimension)
            .field("simd_level", &simd_level())
            .finish_non_exhaustive()
    }
}

// SAFETY: DistanceEngine only holds plain `fn` pointers and a usize.
// `fn` pointers are inherently Send + Sync (they point to static code).
// Reason: Required for sharing across rayon threads in HNSW search.
unsafe impl Send for DistanceEngine {}
// SAFETY: See Send impl above — fn pointers are thread-safe.
unsafe impl Sync for DistanceEngine {}

impl DistanceEngine {
    /// Creates a new `DistanceEngine` optimized for the given vector dimension.
    ///
    /// Resolves the best SIMD kernel for each operation based on runtime CPU
    /// detection and the dimension size. This resolution happens once; all
    /// subsequent calls go through a single indirect call with no branching.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        let level = simd_level();
        Self {
            dot_product_fn: Self::resolve_dot_product(level, dimension),
            squared_l2_fn: Self::resolve_squared_l2(level, dimension),
            cosine_fn: Self::resolve_cosine(level, dimension),
            hamming_fn: Self::resolve_hamming(level, dimension),
            jaccard_fn: Self::resolve_jaccard(level, dimension),
            dimension,
        }
    }

    /// Computes the dot product using the pre-resolved SIMD kernel.
    #[allow(clippy::inline_always)] // Reason: Single indirect call on hot path
    #[inline(always)]
    #[must_use]
    pub fn dot_product(&self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");
        debug_assert_eq!(
            a.len(),
            self.dimension,
            "Vector dimension mismatch with engine"
        );
        (self.dot_product_fn)(a, b)
    }

    /// Computes the squared L2 distance using the pre-resolved SIMD kernel.
    #[allow(clippy::inline_always)] // Reason: Single indirect call on hot path
    #[inline(always)]
    #[must_use]
    pub fn squared_l2(&self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");
        debug_assert_eq!(
            a.len(),
            self.dimension,
            "Vector dimension mismatch with engine"
        );
        (self.squared_l2_fn)(a, b)
    }

    /// Computes the Euclidean distance using the pre-resolved SIMD kernel.
    #[allow(clippy::inline_always)] // Reason: Thin wrapper over squared_l2
    #[inline(always)]
    #[must_use]
    pub fn euclidean(&self, a: &[f32], b: &[f32]) -> f32 {
        self.squared_l2(a, b).sqrt()
    }

    /// Computes the cosine similarity using the pre-resolved SIMD kernel.
    #[allow(clippy::inline_always)] // Reason: Single indirect call on hot path
    #[inline(always)]
    #[must_use]
    pub fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");
        debug_assert_eq!(
            a.len(),
            self.dimension,
            "Vector dimension mismatch with engine"
        );
        (self.cosine_fn)(a, b)
    }

    /// Computes Hamming distance using the pre-resolved SIMD kernel.
    #[allow(clippy::inline_always)] // Reason: Single indirect call on hot path
    #[inline(always)]
    #[must_use]
    pub fn hamming(&self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");
        debug_assert_eq!(
            a.len(),
            self.dimension,
            "Vector dimension mismatch with engine"
        );
        (self.hamming_fn)(a, b)
    }

    /// Computes Jaccard similarity using the pre-resolved SIMD kernel.
    #[allow(clippy::inline_always)] // Reason: Single indirect call on hot path
    #[inline(always)]
    #[must_use]
    pub fn jaccard(&self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");
        debug_assert_eq!(
            a.len(),
            self.dimension,
            "Vector dimension mismatch with engine"
        );
        (self.jaccard_fn)(a, b)
    }

    /// Returns the dimension this engine was optimized for.
    #[inline]
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Resolves the best dot product kernel for (level, dimension).
    fn resolve_dot_product(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
        match level {
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx512 if dim >= 512 => |a, b| {
                // SAFETY: simd_level() confirmed AVX-512F at engine construction.
                // Reason: Target-feature function safe after runtime detection.
                unsafe { super::dot_product_avx512_4acc(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx512 => |a, b| {
                // SAFETY: simd_level() confirmed AVX-512F at engine construction.
                unsafe { super::dot_product_avx512(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 256 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::dot_product_avx2_4acc(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 64 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::dot_product_avx2(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 8 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::dot_product_avx2_1acc(a, b) }
            },
            #[cfg(target_arch = "aarch64")]
            SimdLevel::Neon if dim >= 4 => |a, b| super::dot_product_neon(a, b),
            _ => dot_product_scalar,
        }
    }

    /// Resolves the best squared L2 kernel for (level, dimension).
    fn resolve_squared_l2(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
        match level {
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx512 if dim >= 512 => |a, b| {
                // SAFETY: simd_level() confirmed AVX-512F at engine construction.
                unsafe { super::squared_l2_avx512_4acc(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx512 => |a, b| {
                // SAFETY: simd_level() confirmed AVX-512F at engine construction.
                unsafe { super::squared_l2_avx512(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 256 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::squared_l2_avx2_4acc(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 64 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::squared_l2_avx2(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 8 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::squared_l2_avx2_1acc(a, b) }
            },
            #[cfg(target_arch = "aarch64")]
            SimdLevel::Neon if dim >= 4 => |a, b| super::squared_l2_neon(a, b),
            _ => squared_l2_scalar,
        }
    }

    /// Resolves the best hamming distance kernel for (level, dimension).
    fn resolve_hamming(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
        match level {
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx512 if dim >= 16 => |a, b| {
                // SAFETY: simd_level() confirmed AVX-512F at engine construction.
                // Reason: Target-feature function safe after runtime detection.
                unsafe { super::hamming_avx512(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 8 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                // Reason: Target-feature function safe after runtime detection.
                unsafe { super::hamming_avx2(a, b) }
            },
            _ => scalar::hamming_scalar,
        }
    }

    /// Resolves the best jaccard similarity kernel for (level, dimension).
    fn resolve_jaccard(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
        match level {
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx512 if dim >= 16 => |a, b| {
                // SAFETY: simd_level() confirmed AVX-512F at engine construction.
                // Reason: Target-feature function safe after runtime detection.
                unsafe { super::jaccard_avx512(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 8 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                // Reason: Target-feature function safe after runtime detection.
                unsafe { super::jaccard_avx2(a, b) }
            },
            _ => scalar::jaccard_scalar,
        }
    }

    /// Resolves the best cosine similarity kernel for (level, dimension).
    fn resolve_cosine(level: SimdLevel, dim: usize) -> fn(&[f32], &[f32]) -> f32 {
        match level {
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx512 if dim >= 16 => |a, b| {
                // SAFETY: simd_level() confirmed AVX-512F at engine construction.
                unsafe { super::cosine_fused_avx512(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 1024 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::cosine_fused_avx2(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 64 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::cosine_fused_avx2_2acc(a, b) }
            },
            #[cfg(target_arch = "x86_64")]
            SimdLevel::Avx2 if dim >= 8 => |a, b| {
                // SAFETY: simd_level() confirmed AVX2+FMA at engine construction.
                unsafe { super::cosine_fused_avx2(a, b) }
            },
            _ => scalar::cosine_scalar,
        }
    }
}
