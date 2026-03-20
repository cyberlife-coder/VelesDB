//! Runtime SIMD level detection and dispatch wiring.

mod cosine;
mod dot;
mod euclidean;
mod hamming;

pub use cosine::{batch_cosine_native, cosine_normalized_native, cosine_similarity_native};
pub use dot::{batch_dot_product_native, dot_product_native};
pub use euclidean::{
    batch_euclidean_native, batch_squared_l2_native, euclidean_native, norm_native,
    normalize_inplace_native, squared_l2_native,
};
pub use hamming::{
    batch_hamming_native, batch_jaccard_native, hamming_binary_native, hamming_distance_native,
    jaccard_similarity_native,
};

/// SIMD capability level detected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    /// AVX-512F available (x86_64 only).
    Avx512,
    /// AVX2 + FMA available (x86_64 only).
    Avx2,
    /// NEON available (aarch64, always true).
    Neon,
    /// Scalar fallback.
    Scalar,
}

static SIMD_LEVEL: std::sync::OnceLock<SimdLevel> = std::sync::OnceLock::new();

#[inline]
pub(super) fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[inline]
pub(super) fn squared_l2_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

fn detect_simd_level() -> SimdLevel {
    let level;

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            level = SimdLevel::Avx512;
        } else if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            level = SimdLevel::Avx2;
        } else {
            level = SimdLevel::Scalar;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        level = SimdLevel::Neon;
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        level = SimdLevel::Scalar;
    }

    level
}

#[inline]
#[must_use]
/// Returns the cached SIMD level for the current process.
pub fn simd_level() -> SimdLevel {
    *SIMD_LEVEL.get_or_init(detect_simd_level)
}

#[inline]
#[must_use]
pub fn has_avx512vl() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        return is_x86_feature_detected!("avx512vl");
    }
    #[allow(unreachable_code)]
    false
}

#[inline]
#[must_use]
pub fn has_avx512bw() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        return is_x86_feature_detected!("avx512bw");
    }
    #[allow(unreachable_code)]
    false
}

#[inline]
#[must_use]
pub fn has_avx512vnni() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        return is_x86_feature_detected!("avx512vnni");
    }
    #[allow(unreachable_code)]
    false
}

#[inline]
/// Warms up runtime SIMD dispatch cache and CPU caches for common dimensions.
///
/// Logs the detected SIMD level on non-WASM targets for diagnostics.
pub fn warmup_simd_cache() {
    let level = simd_level();

    // Log detected SIMD level for diagnostics (skipped in WASM)
    #[cfg(feature = "persistence")]
    eprintln!("[velesdb] SIMD dispatch: {level:?} detected");

    let warmup_size = 768;
    let a: Vec<f32> = vec![0.01; warmup_size];
    let b: Vec<f32> = vec![0.01; warmup_size];
    for _ in 0..3 {
        let _ = dot_product_native(&a, &b);
        let _ = cosine_similarity_native(&a, &b);
    }
}

/// Pre-resolved SIMD kernels for repeated distance operations at a fixed dimension.
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
        let avx512_ext = (has_avx512vl(), has_avx512bw(), has_avx512vnni());
        f.debug_struct("DistanceEngine")
            .field("dimension", &self.dimension)
            .field("simd_level", &simd_level())
            .field("avx512_ext(vl,bw,vnni)", &avx512_ext)
            .finish_non_exhaustive()
    }
}

// SAFETY: `DistanceEngine` stores only plain function pointers and a `usize`.
unsafe impl Send for DistanceEngine {}
// SAFETY: Function pointers are immutable references to static code.
unsafe impl Sync for DistanceEngine {}

impl DistanceEngine {
    /// Creates a distance engine and resolves SIMD kernels once for `dimension`.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        let level = simd_level();
        Self {
            dot_product_fn: dot::resolve_dot_product(level, dimension),
            squared_l2_fn: euclidean::resolve_squared_l2(level, dimension),
            cosine_fn: cosine::resolve_cosine(level, dimension),
            hamming_fn: hamming::resolve_hamming(level, dimension),
            jaccard_fn: hamming::resolve_jaccard(level, dimension),
            dimension,
        }
    }

    /// Computes dot product with the pre-resolved kernel.
    #[allow(clippy::inline_always)]
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

    /// Computes squared L2 with the pre-resolved kernel.
    #[allow(clippy::inline_always)]
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

    /// Computes Euclidean distance.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    #[must_use]
    pub fn euclidean(&self, a: &[f32], b: &[f32]) -> f32 {
        self.squared_l2(a, b).sqrt()
    }

    /// Computes cosine similarity with the pre-resolved kernel.
    #[allow(clippy::inline_always)]
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

    /// Computes Hamming distance with the pre-resolved kernel.
    #[allow(clippy::inline_always)]
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

    /// Computes Jaccard similarity with the pre-resolved kernel.
    #[allow(clippy::inline_always)]
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

    /// Returns the dimension this engine is specialized for.
    #[inline]
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }
}
