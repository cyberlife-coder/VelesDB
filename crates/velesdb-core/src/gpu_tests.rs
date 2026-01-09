//! Tests for `gpu` module

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
