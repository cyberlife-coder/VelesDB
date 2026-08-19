use super::*;

#[test]
fn test_prefetch_read_l1_safe() {
    // Test that prefetch doesn't crash on valid pointer
    let data = vec![0u8; 4096];
    prefetch_read_l1(data.as_ptr());
}

#[test]
fn test_prefetch_read_l1_null_safe() {
    // Test that prefetch doesn't crash on null pointer
    // (prefetch is a hint, should be safe to ignore)
    prefetch_read_l1(std::ptr::null());
}

#[test]
fn test_prefetch_vector_neon() {
    let vector: Vec<f32> = (0_u16..768).map(f32::from).collect();
    prefetch_vector_neon(&vector);
    // No crash = success
}

#[test]
fn test_prefetch_vector_neon_empty() {
    let vector: Vec<f32> = vec![];
    prefetch_vector_neon(&vector);
    // No crash = success
}

#[test]
fn test_calculate_prefetch_distance() {
    assert_eq!(calculate_prefetch_distance_neon(128), 4);
    assert_eq!(calculate_prefetch_distance_neon(384), 6);
    assert_eq!(calculate_prefetch_distance_neon(768), 10);
    assert_eq!(calculate_prefetch_distance_neon(1536), 14);
    assert_eq!(calculate_prefetch_distance_neon(3072), 16);
}

#[test]
fn test_all_prefetch_variants() {
    let data = vec![0u8; 256];
    let ptr = data.as_ptr();

    prefetch_read_l1(ptr);
    prefetch_read_l2(ptr);
    prefetch_read_l3(ptr);
    prefetch_write_l1(ptr);
    // No crash = success
}
