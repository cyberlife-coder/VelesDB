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

#[cfg(target_arch = "x86_64")]
fn hamming_avx512_popcnt(a: &[f32], b: &[f32]) -> u32 {
    // For now, delegate to regular popcnt
    // TODO: Implement true AVX-512 VPOPCNTDQ when available
    hamming_popcnt(a, b)
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
// TDD TESTS
// =============================================================================

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::uninlined_format_args,
    clippy::float_cmp
)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Dispatch correctness tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_dot_product_dispatched_correctness() {
        // Arrange
        let a = vec![1.0f32, 2.0, 3.0, 4.0];
        let b = vec![5.0f32, 6.0, 7.0, 8.0];

        // Act
        let result = dot_product_dispatched(&a, &b);

        // Assert - 1*5 + 2*6 + 3*7 + 4*8 = 5 + 12 + 21 + 32 = 70
        assert!((result - 70.0).abs() < 1e-5);
    }

    #[test]
    fn test_euclidean_dispatched_correctness() {
        // Arrange
        let a = vec![0.0f32, 0.0, 0.0];
        let b = vec![3.0f32, 4.0, 0.0];

        // Act
        let result = euclidean_dispatched(&a, &b);

        // Assert - sqrt(9 + 16) = 5
        assert!((result - 5.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_dispatched_correctness() {
        // Arrange - same vector should have cosine = 1.0
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![1.0f32, 2.0, 3.0];

        // Act
        let result = cosine_dispatched(&a, &b);

        // Assert
        assert!((result - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_dispatched_orthogonal() {
        // Arrange - orthogonal vectors should have cosine = 0
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];

        // Act
        let result = cosine_dispatched(&a, &b);

        // Assert
        assert!(result.abs() < 1e-5);
    }

    #[test]
    fn test_cosine_normalized_dispatched() {
        // Arrange - pre-normalized vectors
        let a = vec![1.0f32, 0.0];
        let b = vec![0.707f32, 0.707]; // ~45 degrees

        // Act
        let result = cosine_normalized_dispatched(&a, &b);

        // Assert - cos(45°) ≈ 0.707
        assert!((result - 0.707).abs() < 0.01);
    }

    #[test]
    fn test_hamming_dispatched_correctness() {
        // Arrange - binary vectors encoded as f32
        let a = vec![1.0f32, 0.0, 1.0, 0.0]; // bits: 1010
        let b = vec![1.0f32, 1.0, 0.0, 0.0]; // bits: 1100

        // Act
        let result = hamming_dispatched(&a, &b);

        // Assert - differs in positions 1 and 2
        assert_eq!(result, 2);
    }

    // -------------------------------------------------------------------------
    // Large vector tests (768D like real embeddings)
    // -------------------------------------------------------------------------

    #[test]
    fn test_dot_product_dispatched_768d() {
        // Arrange
        let a: Vec<f32> = (0..768).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..768).map(|i| ((768 - i) as f32) * 0.001).collect();

        // Act
        let result = dot_product_dispatched(&a, &b);

        // Assert - just verify it doesn't panic and returns reasonable value
        assert!(result.is_finite());
        assert!(result > 0.0);
    }

    #[test]
    fn test_euclidean_dispatched_768d() {
        // Arrange
        let a: Vec<f32> = vec![0.0; 768];
        let b: Vec<f32> = vec![1.0; 768];

        // Act
        let result = euclidean_dispatched(&a, &b);

        // Assert - sqrt(768 * 1) ≈ 27.71
        assert!((result - 768.0_f32.sqrt()).abs() < 0.01);
    }

    #[test]
    fn test_cosine_dispatched_768d() {
        // Arrange
        let a: Vec<f32> = (0..768).map(|i| (i as f32).sin()).collect();
        let b = a.clone();

        // Act
        let result = cosine_dispatched(&a, &b);

        // Assert - same vector = 1.0
        assert!((result - 1.0).abs() < 1e-4);
    }

    // -------------------------------------------------------------------------
    // SIMD features detection tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_simd_features_detect() {
        // Act
        let features = SimdFeatures::detect();

        // Assert - just verify it doesn't panic
        let _name = features.best_instruction_set();
        println!("SIMD features: {:?}", features);
        println!("Best instruction set: {}", features.best_instruction_set());
    }

    #[test]
    fn test_simd_features_info() {
        // Act
        let features = simd_features_info();

        // Assert - returns valid struct
        assert!(!features.best_instruction_set().is_empty());
    }

    // -------------------------------------------------------------------------
    // Prefetch constant tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_prefetch_distance_768d() {
        // 768 * 4 bytes / 64 bytes = 48 cache lines
        assert_eq!(PREFETCH_DISTANCE_768D, 48);
    }

    #[test]
    fn test_prefetch_distance_384d() {
        // 384 * 4 bytes / 64 bytes = 24 cache lines
        assert_eq!(PREFETCH_DISTANCE_384D, 24);
    }

    #[test]
    fn test_prefetch_distance_1536d() {
        // 1536 * 4 bytes / 64 bytes = 96 cache lines
        assert_eq!(PREFETCH_DISTANCE_1536D, 96);
    }

    #[test]
    fn test_prefetch_distance_function() {
        assert_eq!(prefetch_distance(768), 48);
        assert_eq!(prefetch_distance(384), 24);
        assert_eq!(prefetch_distance(128), 8);
    }

    // -------------------------------------------------------------------------
    // OnceLock initialization tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_dispatch_initialized_once() {
        // Multiple calls should use cached function pointer
        let a = vec![1.0f32; 100];
        let b = vec![2.0f32; 100];

        // First call initializes
        let r1 = dot_product_dispatched(&a, &b);

        // Second call uses cached pointer
        let r2 = dot_product_dispatched(&a, &b);

        // Results should be identical
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_dispatch_thread_safe() {
        use std::sync::Arc;
        use std::thread;

        // Arrange
        let a = Arc::new(vec![1.0f32; 768]);
        let b = Arc::new(vec![2.0f32; 768]);

        // Act - multiple threads calling dispatched functions
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let a = Arc::clone(&a);
                let b = Arc::clone(&b);
                thread::spawn(move || {
                    for _ in 0..100 {
                        let _ = dot_product_dispatched(&a, &b);
                        let _ = cosine_dispatched(&a, &b);
                        let _ = euclidean_dispatched(&a, &b);
                    }
                })
            })
            .collect();

        // Assert - no panics
        for h in handles {
            h.join().expect("Thread should not panic");
        }
    }

    // -------------------------------------------------------------------------
    // Edge case tests
    // -------------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "dimensions must match")]
    fn test_dot_product_dispatched_length_mismatch() {
        let a = vec![1.0f32, 2.0];
        let b = vec![1.0f32, 2.0, 3.0];
        let _ = dot_product_dispatched(&a, &b);
    }

    #[test]
    fn test_empty_vectors() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];

        // Should not panic, returns 0
        assert_eq!(dot_product_dispatched(&a, &b), 0.0);
    }

    #[test]
    fn test_single_element() {
        let a = vec![3.0f32];
        let b = vec![4.0f32];

        assert_eq!(dot_product_dispatched(&a, &b), 12.0);
    }
}
