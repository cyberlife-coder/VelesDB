//! Trigram compute backend selection.
//!
//! Provides auto-selection of CPU SIMD vs GPU backends for trigram operations.
//!
//! # Note
//!
//! `GpuTrigramAccelerator` was removed (C-02 fix) because it contained
//! zero GPU code — all methods were pure CPU. Real GPU trigram shaders
//! may be added in the future.

/// Compute backend selection for trigram operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrigramComputeBackend {
    /// CPU SIMD (default, always available)
    #[default]
    CpuSimd,
    /// GPU via wgpu (requires `gpu` feature)
    #[cfg(feature = "gpu")]
    Gpu,
}

impl TrigramComputeBackend {
    /// Select best available backend based on workload size.
    #[must_use]
    pub fn auto_select(doc_count: usize, pattern_count: usize) -> Self {
        #[cfg(not(feature = "gpu"))]
        let _ = (doc_count, pattern_count);

        #[cfg(feature = "gpu")]
        {
            // GPU is better for large workloads
            if doc_count > 500_000 || (doc_count > 100_000 && pattern_count > 10) {
                if crate::gpu::ComputeBackend::gpu_available() {
                    return Self::Gpu;
                }
            }
        }

        Self::CpuSimd
    }

    /// Get backend name for logging.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::CpuSimd => "CPU SIMD",
            #[cfg(feature = "gpu")]
            Self::Gpu => "GPU (wgpu)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_auto_select_small() {
        let backend = TrigramComputeBackend::auto_select(10_000, 1);
        assert_eq!(backend, TrigramComputeBackend::CpuSimd);
    }

    #[test]
    fn test_backend_auto_select_medium() {
        let backend = TrigramComputeBackend::auto_select(100_000, 5);
        // Should still be CPU for medium workloads
        assert_eq!(backend, TrigramComputeBackend::CpuSimd);
    }

    #[test]
    fn test_backend_name() {
        assert_eq!(TrigramComputeBackend::CpuSimd.name(), "CPU SIMD");
    }
}
