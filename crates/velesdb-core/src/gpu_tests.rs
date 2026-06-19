//! Tests for `gpu` module
//!
//! Covers both happy-path and error-path GPU scenarios:
//! - Graceful fallback when GPU is unavailable
//! - Consistency of availability checks
//! - `ComputeBackend` dispatch logic

use super::gpu::*;

#[test]
fn test_compute_backend_default_is_simd() {
    let backend = ComputeBackend::default();
    assert_eq!(backend, ComputeBackend::Simd);
}

#[cfg(not(feature = "gpu"))]
#[test]
fn test_gpu_available_false_without_feature() {
    assert!(!ComputeBackend::gpu_available());
}

// =========================================================================
// Plan 04-09 Task 1: GPU unavailability graceful fallback
// =========================================================================

#[test]
fn test_compute_backend_fallback_to_simd() {
    // best_available() must always return a valid backend (never panic).
    // On machines without GPU, it should return Simd.
    let backend = ComputeBackend::best_available();
    // Must select Gpu vs Simd consistently with the actual availability probe.
    #[cfg(feature = "gpu")]
    {
        if ComputeBackend::gpu_available() {
            assert_eq!(
                backend,
                ComputeBackend::Gpu,
                "best_available() must select Gpu when gpu_available() is true"
            );
        } else {
            assert_eq!(
                backend,
                ComputeBackend::Simd,
                "best_available() must fall back to Simd when GPU is unavailable"
            );
        }
    }
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
    #[cfg(not(feature = "gpu"))]
    assert!(
        !first,
        "gpu_available() must be false without the gpu feature"
    );
    #[cfg(feature = "gpu")]
    {
        use super::gpu::GpuAccelerator;
        assert_eq!(
            first,
            GpuAccelerator::is_available(),
            "gpu_available() must agree with the cached GpuAccelerator::is_available() probe"
        );
    }
}

#[cfg(feature = "gpu")]
#[test]
fn test_gpu_accelerator_none_without_gpu() {
    use super::gpu::GpuAccelerator;
    // GpuAccelerator::new() returns Option — must not panic regardless of hardware
    let gpu = GpuAccelerator::new();
    if gpu.is_none() {
        // Graceful degradation: unavailable accelerator must report unavailable.
        assert!(
            !GpuAccelerator::is_available(),
            "GpuAccelerator::new() returned None while is_available() reported true"
        );
    }
}
