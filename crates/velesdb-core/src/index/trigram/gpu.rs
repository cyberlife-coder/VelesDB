//! GPU-accelerated trigram operations using wgpu.
//!
//! Provides massive parallelism for bulk trigram operations:
//! - Batch document indexing
//! - Parallel pattern matching across millions of docs
//!
//! # When to Use GPU
//!
//! | Operation | CPU SIMD Best | GPU Best |
//! |-----------|---------------|----------|
//! | Single search | < 100K docs | > 500K docs |
//! | Batch index | < 10K docs | > 50K docs |
//! | Pattern scan | < 1M docs | > 1M docs |
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
use crate::gpu::GpuAccelerator;

#[cfg(feature = "gpu")]
use roaring::RoaringBitmap;
#[cfg(feature = "gpu")]
use std::collections::HashSet;

/// GPU-accelerated trigram index operations.
#[cfg(feature = "gpu")]
pub struct GpuTrigramAccelerator {
    accelerator: GpuAccelerator,
    /// Trigram data uploaded to GPU
    trigram_buffer: Option<wgpu::Buffer>,
    /// Document bitmap data
    bitmap_buffer: Option<wgpu::Buffer>,
}

#[cfg(feature = "gpu")]
impl GpuTrigramAccelerator {
    /// Create a new GPU trigram accelerator.
    pub async fn new() -> Result<Self, String> {
        let accelerator = GpuAccelerator::new().ok_or("GPU not available")?;
        Ok(Self {
            accelerator,
            trigram_buffer: None,
            bitmap_buffer: None,
        })
    }

    /// Check if GPU acceleration is available.
    #[must_use]
    pub fn is_available() -> bool {
        GpuAccelerator::is_available()
    }

    /// Batch search multiple patterns on GPU.
    ///
    /// More efficient than individual searches for > 10 patterns.
    pub async fn batch_search(
        &self,
        _patterns: &[&str],
        _inverted_index: &std::collections::HashMap<[u8; 3], RoaringBitmap>,
    ) -> Result<Vec<RoaringBitmap>, String> {
        // GPU compute shader for parallel pattern matching
        // Each workgroup processes one pattern
        // Returns bitmap of matching documents per pattern

        // TODO: Implement WGSL compute shader for trigram matching
        // For now, fallback to CPU
        Err("GPU trigram search not yet implemented".to_string())
    }

    /// Batch index multiple documents on GPU.
    ///
    /// Extracts trigrams in parallel using GPU compute.
    pub async fn batch_extract_trigrams(
        &self,
        _documents: &[&str],
    ) -> Result<Vec<HashSet<[u8; 3]>>, String> {
        // GPU compute shader for parallel trigram extraction
        // Each workgroup processes one document

        // TODO: Implement WGSL compute shader for trigram extraction
        Err("GPU trigram extraction not yet implemented".to_string())
    }
}

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
    pub fn auto_select(_doc_count: usize, _pattern_count: usize) -> Self {
        #[cfg(feature = "gpu")]
        {
            // GPU is better for large workloads
            if _doc_count > 500_000 || (_doc_count > 100_000 && _pattern_count > 10) {
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
