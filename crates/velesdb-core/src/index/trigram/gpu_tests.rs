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
