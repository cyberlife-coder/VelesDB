//! Unified SIMD dispatch with adaptive backend selection.
//!
//! This module provides a single entry point for all SIMD-accelerated vector operations,
//! automatically selecting the optimal backend based on runtime micro-benchmarks.
//!
//! # Adaptive Dispatch
//!
//! On first call, the module runs micro-benchmarks (~5-10ms) to determine the fastest
//! backend for each (metric, dimension) combination on the current machine. Results are
//! cached via `OnceLock` for zero-overhead subsequent calls.
//!
//! # Backends
//!
//! | Backend | Technology | Best For |
//! |---------|------------|----------|
//! | `NativeAvx512` | `core::arch` AVX-512 | Large vectors (768D+) on Zen4+/Skylake-X+ |
//! | `NativeAvx2` | `core::arch` AVX2 | Large vectors on Haswell+ |
//! | `NativeNeon` | `core::arch` NEON | aarch64 (Apple Silicon, ARM servers) |
//! | `Wide32` | `wide` crate 4×f32x8 | Medium vectors (128-768D) |
//! | `Wide8` | `wide` crate f32x8 | Small vectors, WASM |
//! | `Scalar` | Rust native | Fallback, very small vectors |
//!
//! # Example
//!
//! ```rust,ignore
//! use velesdb_core::simd_ops;
//! use velesdb_core::DistanceMetric;
//!
//! let a = vec![0.1; 768];
//! let b = vec![0.2; 768];
//!
//! // Automatically dispatches to the fastest backend
//! let sim = simd_ops::similarity(DistanceMetric::Cosine, &a, &b);
//! let dist = simd_ops::distance(DistanceMetric::Euclidean, &a, &b);
//! let n = simd_ops::norm(&a);
//! ```

use crate::distance::DistanceMetric;
use std::sync::OnceLock;
use std::time::Instant;

/// SIMD backends available for dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdBackend {
    /// Native AVX-512 intrinsics (x86_64 only)
    NativeAvx512,
    /// Native AVX2 intrinsics (x86_64 only)
    NativeAvx2,
    /// Native NEON intrinsics (aarch64 only)
    NativeNeon,
    /// Wide crate with 4×f32x8 (32-wide processing)
    Wide32,
    /// Wide crate with f32x8 (8-wide processing)
    Wide8,
    /// Scalar fallback
    Scalar,
}

impl std::fmt::Display for SimdBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NativeAvx512 => write!(f, "AVX-512"),
            Self::NativeAvx2 => write!(f, "AVX2"),
            Self::NativeNeon => write!(f, "NEON"),
            Self::Wide32 => write!(f, "Wide32"),
            Self::Wide8 => write!(f, "Wide8"),
            Self::Scalar => write!(f, "Scalar"),
        }
    }
}

/// Internal metric enum for dispatch table indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Metric {
    DotProduct,
    Cosine,
    Euclidean,
    Hamming,
    Jaccard,
    Norm,
}

impl From<DistanceMetric> for Metric {
    fn from(m: DistanceMetric) -> Self {
        match m {
            DistanceMetric::Cosine => Metric::Cosine,
            DistanceMetric::Euclidean => Metric::Euclidean,
            DistanceMetric::DotProduct => Metric::DotProduct,
            DistanceMetric::Hamming => Metric::Hamming,
            DistanceMetric::Jaccard => Metric::Jaccard,
        }
    }
}

/// Dimensions benchmarked for adaptive dispatch.
/// Covers common embedding model dimensions.
const BENCHMARK_DIMS: [usize; 6] = [128, 384, 768, 1024, 1536, 3072];

/// Number of iterations per micro-benchmark (balance precision vs init time).
const BENCHMARK_ITERATIONS: usize = 500;

/// Dispatch table built from micro-benchmarks.
/// Maps (metric, dimension_index) → optimal backend.
#[derive(Debug, Clone)]
pub struct DispatchTable {
    /// Backend per dimension for DotProduct
    dot_product: [SimdBackend; 6],
    /// Backend per dimension for Cosine
    cosine: [SimdBackend; 6],
    /// Backend per dimension for Euclidean
    euclidean: [SimdBackend; 6],
    /// Backend for Hamming (dimension-independent)
    hamming: SimdBackend,
    /// Backend for Jaccard (dimension-independent)
    jaccard: SimdBackend,
    /// Backend per dimension for Norm
    norm: [SimdBackend; 6],
    /// Initialization time in milliseconds
    init_time_ms: f64,
}

impl Default for DispatchTable {
    fn default() -> Self {
        Self {
            dot_product: [SimdBackend::Wide8; 6],
            cosine: [SimdBackend::Wide8; 6],
            euclidean: [SimdBackend::Wide8; 6],
            hamming: SimdBackend::Wide8,
            jaccard: SimdBackend::Wide8,
            norm: [SimdBackend::Wide8; 6],
            init_time_ms: 0.0,
        }
    }
}

impl DispatchTable {
    /// Builds the dispatch table via micro-benchmarks.
    fn from_benchmarks() -> Self {
        let start = Instant::now();
        let mut table = Self::default();

        // Detect available backends on this platform
        let backends = available_backends();

        for (i, &dim) in BENCHMARK_DIMS.iter().enumerate() {
            // Generate test vectors
            let a = generate_test_vector(dim, 0.0);
            let b = generate_test_vector(dim, 1.0);

            // Benchmark each metric
            table.dot_product[i] = find_fastest_backend(&backends, &a, &b, benchmark_dot_product);
            table.cosine[i] = find_fastest_backend(&backends, &a, &b, benchmark_cosine);
            table.euclidean[i] = find_fastest_backend(&backends, &a, &b, benchmark_euclidean);
            table.norm[i] = find_fastest_backend_unary(&backends, &a, benchmark_norm);
        }

        // Hamming and Jaccard: always use Wide8 (simd_explicit) - no benchmark needed
        // These metrics only have simd_explicit implementation, benchmarking is wasteful
        table.hamming = SimdBackend::Wide8;
        table.jaccard = SimdBackend::Wide8;

        table.init_time_ms = start.elapsed().as_secs_f64() * 1000.0;
        table
    }

    /// Selects the optimal backend for a given metric and dimension.
    fn select_backend(&self, metric: Metric, dim: usize) -> SimdBackend {
        // Find the closest dimension index
        let idx = BENCHMARK_DIMS.iter().position(|&d| dim <= d).unwrap_or(5);

        match metric {
            Metric::DotProduct => self.dot_product[idx],
            Metric::Cosine => self.cosine[idx],
            Metric::Euclidean => self.euclidean[idx],
            Metric::Hamming => self.hamming,
            Metric::Jaccard => self.jaccard,
            Metric::Norm => self.norm[idx],
        }
    }
}

/// Global dispatch table, initialized on first use.
static DISPATCH_TABLE: OnceLock<DispatchTable> = OnceLock::new();

// =============================================================================
// Public API
// =============================================================================

/// Computes similarity between two vectors using adaptive dispatch.
///
/// Automatically selects the fastest SIMD backend based on the metric,
/// vector dimension, and platform capabilities.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn similarity(metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch: {} vs {}", a.len(), b.len());
    let table = DISPATCH_TABLE.get_or_init(DispatchTable::from_benchmarks);
    let backend = table.select_backend(metric.into(), a.len());
    execute_similarity(backend, metric, a, b)
}

/// Computes distance between two vectors using adaptive dispatch.
///
/// For distance metrics (Euclidean, Hamming), returns the distance directly.
/// For similarity metrics (Cosine, DotProduct, Jaccard), returns 1 - similarity.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn distance(metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch: {} vs {}", a.len(), b.len());
    let table = DISPATCH_TABLE.get_or_init(DispatchTable::from_benchmarks);
    let backend = table.select_backend(metric.into(), a.len());
    execute_distance(backend, metric, a, b)
}

/// Computes the L2 norm of a vector using adaptive dispatch.
#[inline]
#[must_use]
pub fn norm(v: &[f32]) -> f32 {
    let table = DISPATCH_TABLE.get_or_init(DispatchTable::from_benchmarks);
    let backend = table.select_backend(Metric::Norm, v.len());
    execute_norm(backend, v)
}

/// Normalizes a vector in-place using adaptive dispatch.
#[inline]
pub fn normalize_inplace(v: &mut [f32]) {
    let n = norm(v);
    if n > 0.0 {
        let inv_norm = 1.0 / n;
        for x in v.iter_mut() {
            *x *= inv_norm;
        }
    }
}

/// Computes dot product using adaptive dispatch.
#[inline]
#[must_use]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    let table = DISPATCH_TABLE.get_or_init(DispatchTable::from_benchmarks);
    let backend = table.select_backend(Metric::DotProduct, a.len());
    execute_dot_product(backend, a, b)
}

/// Returns the current dispatch table (after initialization).
///
/// Useful for debugging and monitoring which backends are selected.
#[must_use]
pub fn dispatch_table() -> &'static DispatchTable {
    DISPATCH_TABLE.get_or_init(DispatchTable::from_benchmarks)
}

/// Initializes the SIMD dispatch table eagerly.
///
/// Call this at application startup (e.g., in `Database::open()`) to avoid
/// the ~5-10ms latency on the first SIMD operation. This is especially important
/// for latency-sensitive applications.
///
/// # Performance Note (Intel Skylake+ AVX-512 Warmup)
///
/// On Intel Skylake-X and later CPUs, AVX-512 instructions can incur a significant
/// warmup cost of up to 56,000 cycles (~14μs at 4GHz) due to:
/// - License-based frequency throttling (P-state transitions)
/// - Vector register file power-up
///
/// The 500-iteration micro-benchmarks capture this warmup cost, ensuring the
/// dispatch table reflects real-world performance after warmup.
///
/// # Returns
///
/// `DispatchInfo` containing initialization time and selected backends.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::simd_ops;
///
/// // Call at application startup
/// let info = simd_ops::init_dispatch();
/// tracing::info!(
///     "SIMD dispatch initialized in {:.2}ms, cosine backend: {:?}",
///     info.init_time_ms,
///     info.cosine_backends[2] // 768D
/// );
/// ```
#[must_use]
pub fn init_dispatch() -> DispatchInfo {
    dispatch_info()
}

/// Forces a re-benchmark of all SIMD backends.
///
/// This is useful for:
/// - Testing different configurations
/// - Debugging performance issues
/// - CLI benchmark mode (`--simd-rebenchmark`)
///
/// **Note**: This function is thread-safe. It creates a new local `DispatchTable`
/// without modifying the global cached table. Multiple threads can call this
/// concurrently without issues.
///
/// # Returns
///
/// New `DispatchInfo` with fresh benchmark results.
///
/// # Example
///
/// ```rust,ignore
/// // CLI flag: --simd-rebenchmark
/// if args.simd_rebenchmark {
///     let info = simd_ops::force_rebenchmark();
///     println!("Re-benchmarked SIMD in {:.2}ms", info.init_time_ms);
/// }
/// ```
pub fn force_rebenchmark() -> DispatchInfo {
    // Build a new dispatch table (ignores the cached one)
    let table = DispatchTable::from_benchmarks();

    // Log the results
    tracing::info!(
        init_time_ms = table.init_time_ms,
        "SIMD dispatch re-benchmarked"
    );

    DispatchInfo {
        init_time_ms: table.init_time_ms,
        dot_product_backends: table.dot_product,
        cosine_backends: table.cosine,
        euclidean_backends: table.euclidean,
        hamming_backend: table.hamming,
        jaccard_backend: table.jaccard,
        norm_backends: table.norm,
        dimensions: BENCHMARK_DIMS,
        available_backends: available_backends(),
    }
}

/// Returns information about the dispatch table initialization.
#[must_use]
pub fn dispatch_info() -> DispatchInfo {
    let table = dispatch_table();
    DispatchInfo {
        init_time_ms: table.init_time_ms,
        dot_product_backends: table.dot_product,
        cosine_backends: table.cosine,
        euclidean_backends: table.euclidean,
        hamming_backend: table.hamming,
        jaccard_backend: table.jaccard,
        norm_backends: table.norm,
        dimensions: BENCHMARK_DIMS,
        available_backends: available_backends(),
    }
}

/// Logs SIMD dispatch information at startup.
///
/// Call this after `init_dispatch()` to log detailed backend selection
/// for monitoring and debugging purposes.
///
/// # Example
///
/// ```rust,ignore
/// simd_ops::init_dispatch();
/// simd_ops::log_dispatch_info();
/// // Logs: "SIMD dispatch: init=5.2ms, cosine=[AVX2, AVX2, AVX-512, ...], ..."
/// ```
pub fn log_dispatch_info() {
    let info = dispatch_info();

    tracing::info!(
        init_time_ms = format!("{:.2}", info.init_time_ms),
        available_backends = ?info.available_backends,
        cosine_768d = %info.cosine_backends[2],
        euclidean_768d = %info.euclidean_backends[2],
        dot_product_768d = %info.dot_product_backends[2],
        hamming = %info.hamming_backend,
        jaccard = %info.jaccard_backend,
        "SIMD adaptive dispatch initialized"
    );
}

/// Information about the dispatch table configuration.
#[derive(Debug, Clone)]
pub struct DispatchInfo {
    /// Time taken to initialize the dispatch table (ms)
    pub init_time_ms: f64,
    /// Backends selected for DotProduct at each dimension
    pub dot_product_backends: [SimdBackend; 6],
    /// Backends selected for Cosine at each dimension
    pub cosine_backends: [SimdBackend; 6],
    /// Backends selected for Euclidean at each dimension
    pub euclidean_backends: [SimdBackend; 6],
    /// Backend selected for Hamming
    pub hamming_backend: SimdBackend,
    /// Backend selected for Jaccard
    pub jaccard_backend: SimdBackend,
    /// Backends selected for Norm at each dimension
    pub norm_backends: [SimdBackend; 6],
    /// Benchmark dimensions
    pub dimensions: [usize; 6],
    /// Available backends on this platform
    pub available_backends: Vec<SimdBackend>,
}

impl std::fmt::Display for DispatchInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "SIMD Dispatch Info:")?;
        writeln!(f, "  Init time: {:.2}ms", self.init_time_ms)?;
        writeln!(f, "  Available backends: {:?}", self.available_backends)?;
        writeln!(f, "  Dimensions: {:?}", self.dimensions)?;
        writeln!(f, "  Cosine backends: {:?}", self.cosine_backends)?;
        writeln!(f, "  Euclidean backends: {:?}", self.euclidean_backends)?;
        writeln!(f, "  DotProduct backends: {:?}", self.dot_product_backends)?;
        writeln!(f, "  Norm backends: {:?}", self.norm_backends)?;
        writeln!(f, "  Hamming backend: {}", self.hamming_backend)?;
        write!(f, "  Jaccard backend: {}", self.jaccard_backend)
    }
}

// =============================================================================
// Backend Detection
// =============================================================================

/// Returns the list of available backends on this platform.
fn available_backends() -> Vec<SimdBackend> {
    let mut backends = vec![SimdBackend::Scalar, SimdBackend::Wide8, SimdBackend::Wide32];

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            backends.push(SimdBackend::NativeAvx2);
        }
        if is_x86_feature_detected!("avx512f") {
            backends.push(SimdBackend::NativeAvx512);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // NEON is always available on aarch64
        backends.push(SimdBackend::NativeNeon);
    }

    backends
}

// =============================================================================
// Micro-Benchmark Functions
// =============================================================================

/// Generates a test vector for benchmarking.
fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let x = (seed + i as f32 * 0.1).sin();
            x
        })
        .collect()
}

/// Finds the fastest backend for a binary operation.
fn find_fastest_backend<F>(
    backends: &[SimdBackend],
    a: &[f32],
    b: &[f32],
    benchmark_fn: F,
) -> SimdBackend
where
    F: Fn(SimdBackend, &[f32], &[f32]) -> f64,
{
    let mut best_backend = SimdBackend::Scalar;
    let mut best_time = f64::MAX;

    for &backend in backends {
        let time = benchmark_fn(backend, a, b);
        if time < best_time {
            best_time = time;
            best_backend = backend;
        }
    }

    best_backend
}

/// Finds the fastest backend for a unary operation.
fn find_fastest_backend_unary<F>(
    backends: &[SimdBackend],
    a: &[f32],
    benchmark_fn: F,
) -> SimdBackend
where
    F: Fn(SimdBackend, &[f32]) -> f64,
{
    let mut best_backend = SimdBackend::Scalar;
    let mut best_time = f64::MAX;

    for &backend in backends {
        let time = benchmark_fn(backend, a);
        if time < best_time {
            best_time = time;
            best_backend = backend;
        }
    }

    best_backend
}

/// Benchmarks dot product for a backend, returns average time in nanoseconds.
fn benchmark_dot_product(backend: SimdBackend, a: &[f32], b: &[f32]) -> f64 {
    // Warmup
    for _ in 0..10 {
        let _ = std::hint::black_box(execute_dot_product(backend, a, b));
    }

    // Measure
    let start = Instant::now();
    for _ in 0..BENCHMARK_ITERATIONS {
        let _ = std::hint::black_box(execute_dot_product(backend, a, b));
    }
    #[allow(clippy::cast_precision_loss)]
    let elapsed = start.elapsed().as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    elapsed
}

/// Benchmarks cosine similarity for a backend.
fn benchmark_cosine(backend: SimdBackend, a: &[f32], b: &[f32]) -> f64 {
    for _ in 0..10 {
        let _ = std::hint::black_box(execute_cosine(backend, a, b));
    }

    let start = Instant::now();
    for _ in 0..BENCHMARK_ITERATIONS {
        let _ = std::hint::black_box(execute_cosine(backend, a, b));
    }
    #[allow(clippy::cast_precision_loss)]
    let elapsed = start.elapsed().as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    elapsed
}

/// Benchmarks euclidean distance for a backend.
fn benchmark_euclidean(backend: SimdBackend, a: &[f32], b: &[f32]) -> f64 {
    for _ in 0..10 {
        let _ = std::hint::black_box(execute_euclidean(backend, a, b));
    }

    let start = Instant::now();
    for _ in 0..BENCHMARK_ITERATIONS {
        let _ = std::hint::black_box(execute_euclidean(backend, a, b));
    }
    #[allow(clippy::cast_precision_loss)]
    let elapsed = start.elapsed().as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    elapsed
}

/// Benchmarks hamming distance for a backend.
fn benchmark_hamming(backend: SimdBackend, a: &[f32], b: &[f32]) -> f64 {
    for _ in 0..10 {
        let _ = std::hint::black_box(execute_hamming(backend, a, b));
    }

    let start = Instant::now();
    for _ in 0..BENCHMARK_ITERATIONS {
        let _ = std::hint::black_box(execute_hamming(backend, a, b));
    }
    #[allow(clippy::cast_precision_loss)]
    let elapsed = start.elapsed().as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    elapsed
}

/// Benchmarks jaccard similarity for a backend.
fn benchmark_jaccard(backend: SimdBackend, a: &[f32], b: &[f32]) -> f64 {
    for _ in 0..10 {
        let _ = std::hint::black_box(execute_jaccard(backend, a, b));
    }

    let start = Instant::now();
    for _ in 0..BENCHMARK_ITERATIONS {
        let _ = std::hint::black_box(execute_jaccard(backend, a, b));
    }
    #[allow(clippy::cast_precision_loss)]
    let elapsed = start.elapsed().as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    elapsed
}

/// Benchmarks norm for a backend.
fn benchmark_norm(backend: SimdBackend, a: &[f32]) -> f64 {
    for _ in 0..10 {
        let _ = std::hint::black_box(execute_norm(backend, a));
    }

    let start = Instant::now();
    for _ in 0..BENCHMARK_ITERATIONS {
        let _ = std::hint::black_box(execute_norm(backend, a));
    }
    #[allow(clippy::cast_precision_loss)]
    let elapsed = start.elapsed().as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    elapsed
}

// =============================================================================
// Backend Execution Functions
// =============================================================================

/// Executes similarity calculation using the specified backend.
fn execute_similarity(backend: SimdBackend, metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        DistanceMetric::Cosine => execute_cosine(backend, a, b),
        DistanceMetric::Euclidean => execute_euclidean(backend, a, b),
        DistanceMetric::DotProduct => execute_dot_product(backend, a, b),
        DistanceMetric::Hamming => execute_hamming(backend, a, b),
        DistanceMetric::Jaccard => execute_jaccard(backend, a, b),
    }
}

/// Executes distance calculation using the specified backend.
fn execute_distance(backend: SimdBackend, metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        DistanceMetric::Euclidean | DistanceMetric::Hamming => {
            execute_similarity(backend, metric, a, b)
        }
        DistanceMetric::Cosine | DistanceMetric::DotProduct | DistanceMetric::Jaccard => {
            1.0 - execute_similarity(backend, metric, a, b)
        }
    }
}

/// Executes dot product using the specified backend.
fn execute_dot_product(backend: SimdBackend, a: &[f32], b: &[f32]) -> f32 {
    match backend {
        SimdBackend::NativeAvx512 | SimdBackend::NativeAvx2 => {
            crate::simd_native::dot_product_native(a, b)
        }
        SimdBackend::NativeNeon => {
            #[cfg(target_arch = "aarch64")]
            {
                crate::simd_neon::dot_product_neon_safe(a, b)
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                crate::simd_explicit::dot_product_simd(a, b)
            }
        }
        SimdBackend::Wide32 => crate::simd_avx512::dot_product_auto(a, b),
        SimdBackend::Wide8 => crate::simd_explicit::dot_product_simd(a, b),
        SimdBackend::Scalar => dot_product_scalar(a, b),
    }
}

/// Executes cosine similarity using the specified backend.
fn execute_cosine(backend: SimdBackend, a: &[f32], b: &[f32]) -> f32 {
    match backend {
        SimdBackend::NativeAvx512 | SimdBackend::NativeAvx2 => {
            crate::simd_native::cosine_similarity_native(a, b)
        }
        SimdBackend::NativeNeon => {
            #[cfg(target_arch = "aarch64")]
            {
                crate::simd_neon::cosine_neon_safe(a, b)
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                crate::simd_explicit::cosine_similarity_simd(a, b)
            }
        }
        SimdBackend::Wide32 => crate::simd_avx512::cosine_similarity_auto(a, b),
        SimdBackend::Wide8 => crate::simd_explicit::cosine_similarity_simd(a, b),
        SimdBackend::Scalar => cosine_scalar(a, b),
    }
}

/// Executes euclidean distance using the specified backend.
fn execute_euclidean(backend: SimdBackend, a: &[f32], b: &[f32]) -> f32 {
    match backend {
        SimdBackend::NativeAvx512 | SimdBackend::NativeAvx2 => {
            crate::simd_native::euclidean_native(a, b)
        }
        SimdBackend::NativeNeon => {
            #[cfg(target_arch = "aarch64")]
            {
                crate::simd_neon::euclidean_neon_safe(a, b)
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                crate::simd_explicit::euclidean_distance_simd(a, b)
            }
        }
        SimdBackend::Wide32 => crate::simd_avx512::euclidean_auto(a, b),
        SimdBackend::Wide8 => crate::simd_explicit::euclidean_distance_simd(a, b),
        SimdBackend::Scalar => euclidean_scalar(a, b),
    }
}

/// Executes hamming distance using the specified backend.
/// Note: Hamming uses Wide8 or Scalar only (no native intrinsics implementation).
fn execute_hamming(backend: SimdBackend, a: &[f32], b: &[f32]) -> f32 {
    match backend {
        SimdBackend::Wide8
        | SimdBackend::Wide32
        | SimdBackend::NativeAvx512
        | SimdBackend::NativeAvx2
        | SimdBackend::NativeNeon => crate::simd_explicit::hamming_distance_simd(a, b),
        SimdBackend::Scalar => hamming_scalar(a, b),
    }
}

/// Executes jaccard similarity using the specified backend.
/// Note: Jaccard uses Wide8 or Scalar only (no native intrinsics implementation).
fn execute_jaccard(backend: SimdBackend, a: &[f32], b: &[f32]) -> f32 {
    match backend {
        SimdBackend::Wide8
        | SimdBackend::Wide32
        | SimdBackend::NativeAvx512
        | SimdBackend::NativeAvx2
        | SimdBackend::NativeNeon => crate::simd_explicit::jaccard_similarity_simd(a, b),
        SimdBackend::Scalar => jaccard_scalar(a, b),
    }
}

/// Executes norm using the specified backend.
fn execute_norm(backend: SimdBackend, v: &[f32]) -> f32 {
    match backend {
        SimdBackend::NativeAvx512 | SimdBackend::NativeAvx2 => crate::simd_native::norm_native(v),
        SimdBackend::NativeNeon => {
            #[cfg(target_arch = "aarch64")]
            {
                // NEON doesn't have a dedicated norm, use dot product with self
                crate::simd_neon::dot_product_neon_safe(v, v).sqrt()
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                crate::simd_explicit::norm_simd(v)
            }
        }
        SimdBackend::Wide32 | SimdBackend::Wide8 => crate::simd_explicit::norm_simd(v),
        SimdBackend::Scalar => norm_scalar(v),
    }
}

// =============================================================================
// Scalar Fallback Implementations
// =============================================================================

fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

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
        (dot / denom).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

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

fn hamming_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    #[allow(clippy::cast_precision_loss)]
    let count = a
        .iter()
        .zip(b.iter())
        .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
        .count() as f32;
    count
}

fn jaccard_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    let mut intersection = 0.0f32;
    let mut union = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        let ax = x.abs();
        let ay = y.abs();
        intersection += ax.min(ay);
        union += ax.max(ay);
    }

    if union > 0.0 {
        intersection / union
    } else {
        1.0
    }
}

fn norm_scalar(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_table_initialization() {
        let table = dispatch_table();
        assert!(table.init_time_ms > 0.0, "Init time should be positive");
        assert!(
            table.init_time_ms < 30000.0,
            "Init time should be < 30 seconds"
        );
    }

    #[test]
    fn test_similarity_cosine() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = similarity(DistanceMetric::Cosine, &a, &b);
        assert!(
            (sim - 1.0).abs() < 1e-5,
            "Identical vectors should have cosine 1.0"
        );

        let c = vec![0.0, 1.0, 0.0];
        let sim2 = similarity(DistanceMetric::Cosine, &a, &c);
        assert!(
            sim2.abs() < 1e-5,
            "Orthogonal vectors should have cosine 0.0"
        );
    }

    #[test]
    fn test_similarity_euclidean() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        let dist = similarity(DistanceMetric::Euclidean, &a, &b);
        assert!((dist - 5.0).abs() < 1e-5, "Distance should be 5.0");
    }

    #[test]
    fn test_similarity_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let dot = similarity(DistanceMetric::DotProduct, &a, &b);
        assert!((dot - 32.0).abs() < 1e-5, "Dot product should be 32.0");
    }

    #[test]
    fn test_norm() {
        let v = vec![3.0, 4.0];
        let n = norm(&v);
        assert!((n - 5.0).abs() < 1e-5, "Norm should be 5.0");
    }

    #[test]
    fn test_normalize_inplace() {
        let mut v = vec![3.0, 4.0];
        normalize_inplace(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-5);
        assert!((v[1] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn test_dispatch_info() {
        let info = dispatch_info();
        assert_eq!(info.dimensions, BENCHMARK_DIMS);
        assert!(info.init_time_ms >= 0.0);
    }

    #[test]
    fn test_large_vectors() {
        let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
        let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

        let sim = similarity(DistanceMetric::Cosine, &a, &b);
        assert!((-1.0..=1.0).contains(&sim), "Cosine should be in [-1, 1]");

        let dist = similarity(DistanceMetric::Euclidean, &a, &b);
        assert!(dist >= 0.0, "Euclidean distance should be non-negative");
    }
}
