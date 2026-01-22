//! Tests for `gpu_backend` module
//!
//! GPU tests must run serially to avoid deadlocks on shared GPU resources.
//! Each test creates a new wgpu instance which can conflict with parallel execution.

use super::gpu_backend::*;
use serial_test::serial;

#[test]
#[serial(gpu)]
fn test_gpu_available_check() {
    // Should not panic
    let _ = GpuAccelerator::is_available();
}

#[test]
#[serial(gpu)]
fn test_gpu_accelerator_creation() {
    // May return None if no GPU available (CI environment)
    let gpu = GpuAccelerator::new();
    if gpu.is_some() {
        println!("GPU available for testing");
    } else {
        println!("No GPU available, skipping GPU tests");
    }
}

#[test]
#[serial(gpu)]
fn test_batch_cosine_empty_input() {
    if let Some(gpu) = GpuAccelerator::new() {
        let results = gpu.batch_cosine_similarity(&[], &[1.0, 0.0, 0.0], 3);
        assert!(results.is_empty());
    }
}

#[test]
#[serial(gpu)]
fn test_batch_cosine_identical_vectors() {
    if let Some(gpu) = GpuAccelerator::new() {
        // Query and vector are identical -> similarity should be 1.0
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![1.0, 0.0, 0.0]; // One vector

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3);

        assert_eq!(results.len(), 1);
        assert!(
            (results[0] - 1.0).abs() < 0.01,
            "Expected ~1.0, got {}",
            results[0]
        );
    }
}

#[test]
#[serial(gpu)]
fn test_batch_cosine_orthogonal_vectors() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![0.0, 1.0, 0.0]; // Orthogonal

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3);

        assert_eq!(results.len(), 1);
        assert!(results[0].abs() < 0.01, "Expected ~0.0, got {}", results[0]);
    }
}

#[test]
#[serial(gpu)]
fn test_batch_cosine_multiple_vectors() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 0.0, 0.0];
        // 3 vectors of dimension 3
        let vectors = vec![
            1.0, 0.0, 0.0, // Identical -> 1.0
            0.0, 1.0, 0.0, // Orthogonal -> 0.0
            -1.0, 0.0, 0.0, // Opposite -> -1.0
        ];

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3);

        assert_eq!(results.len(), 3);
        assert!((results[0] - 1.0).abs() < 0.01, "Expected ~1.0");
        assert!(results[1].abs() < 0.01, "Expected ~0.0");
        assert!((results[2] + 1.0).abs() < 0.01, "Expected ~-1.0");
    }
}

// =========================================================================
// Euclidean Distance Tests
// =========================================================================

#[test]
#[serial(gpu)]
fn test_batch_euclidean_empty_input() {
    if let Some(gpu) = GpuAccelerator::new() {
        let results = gpu.batch_euclidean_distance(&[], &[1.0, 0.0, 0.0], 3);
        assert!(results.is_empty());
    }
}

#[test]
#[serial(gpu)]
fn test_batch_euclidean_identical_vectors() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 2.0, 3.0];
        let vectors = vec![1.0, 2.0, 3.0];

        let results = gpu.batch_euclidean_distance(&vectors, &query, 3);

        assert_eq!(results.len(), 1);
        assert!(results[0].abs() < 0.01, "Expected ~0.0, got {}", results[0]);
    }
}

#[test]
#[serial(gpu)]
fn test_batch_euclidean_known_distance() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![0.0, 0.0, 0.0];
        let vectors = vec![3.0, 4.0, 0.0]; // Distance should be 5.0

        let results = gpu.batch_euclidean_distance(&vectors, &query, 3);

        assert_eq!(results.len(), 1);
        assert!(
            (results[0] - 5.0).abs() < 0.01,
            "Expected ~5.0, got {}",
            results[0]
        );
    }
}

// =========================================================================
// Dot Product Tests
// =========================================================================

#[test]
#[serial(gpu)]
fn test_batch_dot_product_empty_input() {
    if let Some(gpu) = GpuAccelerator::new() {
        let results = gpu.batch_dot_product(&[], &[1.0, 0.0, 0.0], 3);
        assert!(results.is_empty());
    }
}

#[test]
#[serial(gpu)]
fn test_batch_dot_product_orthogonal() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![0.0, 1.0, 0.0];

        let results = gpu.batch_dot_product(&vectors, &query, 3);

        assert_eq!(results.len(), 1);
        assert!(results[0].abs() < 0.01, "Expected ~0.0, got {}", results[0]);
    }
}

#[test]
#[serial(gpu)]
fn test_batch_dot_product_parallel() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![2.0, 3.0, 4.0];
        let vectors = vec![2.0, 3.0, 4.0]; // Dot = 4+9+16 = 29

        let results = gpu.batch_dot_product(&vectors, &query, 3);

        assert_eq!(results.len(), 1);
        assert!(
            (results[0] - 29.0).abs() < 0.01,
            "Expected ~29.0, got {}",
            results[0]
        );
    }
}
