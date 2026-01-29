//! Tests for x86 prefetch optimizations (EPIC-073/US-001).

use crate::simd::{calculate_prefetch_distance, prefetch_vector, prefetch_vector_multi_cache_line};

#[test]
fn test_prefetch_vector_compiles_x86() {
    let vector: Vec<f32> = (0..768).map(|i| i as f32).collect();
    prefetch_vector(&vector);
    // No crash = success
}

#[test]
fn test_prefetch_vector_empty() {
    let vector: Vec<f32> = vec![];
    prefetch_vector(&vector);
    // No crash on empty vector
}

#[test]
fn test_prefetch_vector_null_safe() {
    // Prefetch should handle edge cases gracefully
    let small_vector: Vec<f32> = vec![1.0, 2.0, 3.0];
    prefetch_vector(&small_vector);
}

#[test]
fn test_prefetch_multi_cache_line_384d() {
    // 384D = 1536 bytes = 24 cache lines
    let vector: Vec<f32> = (0..384).map(|i| i as f32).collect();
    prefetch_vector_multi_cache_line(&vector);
}

#[test]
fn test_prefetch_multi_cache_line_768d() {
    // 768D = 3072 bytes = 48 cache lines
    let vector: Vec<f32> = (0..768).map(|i| i as f32).collect();
    prefetch_vector_multi_cache_line(&vector);
}

#[test]
fn test_prefetch_multi_cache_line_1536d() {
    // 1536D = 6144 bytes = 96 cache lines
    let vector: Vec<f32> = (0..1536).map(|i| i as f32).collect();
    prefetch_vector_multi_cache_line(&vector);
}

#[test]
fn test_calculate_prefetch_distance() {
    // Formula: (dimension * 4 bytes) / 64 bytes cache line, clamped to [4, 16]
    // 128D = 512 bytes / 64 = 8
    assert_eq!(calculate_prefetch_distance(128), 8);
    // 384D = 1536 bytes / 64 = 24 → clamped to 16
    assert_eq!(calculate_prefetch_distance(384), 16);
    // 768D = 3072 bytes / 64 = 48 → clamped to 16
    assert_eq!(calculate_prefetch_distance(768), 16);
    // 1536D = 6144 bytes / 64 = 96 → clamped to 16
    assert_eq!(calculate_prefetch_distance(1536), 16);
}

#[test]
fn test_prefetch_distance_bounds() {
    // Edge cases - minimum 4
    assert_eq!(calculate_prefetch_distance(0), 4);
    assert_eq!(calculate_prefetch_distance(1), 4);
    // 32D = 128 bytes / 64 = 2 → clamped to 4
    assert_eq!(calculate_prefetch_distance(32), 4);
    // Maximum capped at 16
    assert!(calculate_prefetch_distance(10000) == 16);
}
