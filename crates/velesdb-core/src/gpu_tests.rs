//! Tests for `gpu` module: graceful degradation when no GPU is present.

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

/// The availability probe is cached: repeated calls agree.
#[cfg(feature = "gpu")]
#[test]
fn test_gpu_availability_is_stable() {
    use super::gpu::GpuAccelerator;
    assert_eq!(
        GpuAccelerator::is_available(),
        GpuAccelerator::is_available()
    );
}
