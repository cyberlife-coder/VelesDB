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

#[cfg(all(test, feature = "gpu"))]
#[path = "gpu/gpu_backend_tests.rs"]
mod gpu_backend_tests;

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
