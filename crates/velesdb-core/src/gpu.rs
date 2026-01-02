//! GPU-accelerated vector operations using wgpu (WebGPU).
//!
//! This module provides optional GPU acceleration for batch distance calculations.
//! Enable with feature flag `gpu`.
//!
//! # When to use GPU
//!
//! - **Batch operations** (100+ queries at once)
//! - **Large datasets** (500K+ vectors)
//! - **Index construction** (HNSW graph building)
//!
//! For single queries on datasets ≤100K, CPU SIMD remains faster.
//!
//! # Platform Support
//!
//! | Platform | Backend |
//! |----------|---------|
//! | Windows | DirectX 12 / Vulkan |
//! | macOS | Metal |
//! | Linux | Vulkan |
//! | Browser | WebGPU |

#[cfg(feature = "gpu")]
#[path = "gpu/gpu_backend.rs"]
mod gpu_backend;

#[cfg(feature = "gpu")]
pub use gpu_backend::GpuAccelerator;

/// Compute backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComputeBackend {
    /// CPU SIMD (default, always available)
    #[default]
    Simd,
    /// GPU via wgpu (requires `gpu` feature)
    #[cfg(feature = "gpu")]
    Gpu,
}

impl ComputeBackend {
    /// Returns the best available backend.
    ///
    /// Prefers GPU if available, falls back to SIMD.
    #[must_use]
    pub fn best_available() -> Self {
        #[cfg(feature = "gpu")]
        {
            if gpu_backend::GpuAccelerator::is_available() {
                return Self::Gpu;
            }
        }
        Self::Simd
    }

    /// Returns true if GPU backend is available.
    #[must_use]
    pub fn gpu_available() -> bool {
        #[cfg(feature = "gpu")]
        {
            gpu_backend::GpuAccelerator::is_available()
        }
        #[cfg(not(feature = "gpu"))]
        {
            false
        }
    }
}

// =============================================================================
// Tests (TDD - Written FIRST)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_backend_default_is_simd() {
        let backend = ComputeBackend::default();
        assert_eq!(backend, ComputeBackend::Simd);
    }

    #[test]
    fn test_best_available_returns_simd_without_gpu_feature() {
        // Without GPU feature, should always return SIMD
        #[cfg(not(feature = "gpu"))]
        {
            let backend = ComputeBackend::best_available();
            assert_eq!(backend, ComputeBackend::Simd);
        }
    }

    #[test]
    fn test_gpu_available_false_without_feature() {
        #[cfg(not(feature = "gpu"))]
        {
            assert!(!ComputeBackend::gpu_available());
        }
    }
}
