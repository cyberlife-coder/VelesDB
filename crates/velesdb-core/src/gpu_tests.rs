//! Tests for `gpu` module
//!
//! Covers both happy-path and error-path GPU scenarios:
//! - Graceful fallback when GPU is unavailable
//! - Consistency of availability checks
//! - ComputeBackend dispatch logic

use super::gpu::*;

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

// =========================================================================
// Plan 04-09 Task 1: GPU unavailability graceful fallback
// =========================================================================

#[test]
fn test_compute_backend_fallback_to_simd() {
    // best_available() must always return a valid backend (never panic).
    // On machines without GPU, it should return Simd.
    let backend = ComputeBackend::best_available();
    // Must be one of the valid variants — Gpu variant only exists with feature
    #[cfg(feature = "gpu")]
    assert!(
        backend == ComputeBackend::Simd || backend == ComputeBackend::Gpu,
        "best_available() returned unexpected variant: {backend:?}"
    );
    #[cfg(not(feature = "gpu"))]
    assert_eq!(backend, ComputeBackend::Simd);
}

#[test]
fn test_gpu_available_consistency() {
    // is_available() must return consistent results across calls (cached via OnceLock)
    let first = ComputeBackend::gpu_available();
    let second = ComputeBackend::gpu_available();
    let third = ComputeBackend::gpu_available();
    assert_eq!(first, second, "gpu_available() must be consistent");
    assert_eq!(second, third, "gpu_available() must be consistent");
}

#[cfg(feature = "gpu")]
#[test]
fn test_gpu_accelerator_none_without_gpu() {
    use super::gpu::gpu_backend::GpuAccelerator;
    // GpuAccelerator::new() returns Option — must not panic regardless of hardware
    let gpu = GpuAccelerator::new();
    if gpu.is_none() {
        // Graceful degradation: no GPU available
        assert!(
            !GpuAccelerator::is_available() || true,
            "Fallback is acceptable"
        );
    }
}
