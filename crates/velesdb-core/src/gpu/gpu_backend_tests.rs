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
        let results = gpu
            .batch_cosine_similarity(&[], &[1.0, 0.0, 0.0], 3)
            .unwrap();
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

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3).unwrap();

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

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3).unwrap();

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

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3).unwrap();

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
        let results = gpu
            .batch_euclidean_distance(&[], &[1.0, 0.0, 0.0], 3)
            .unwrap();
        assert!(results.is_empty());
    }
}

#[test]
#[serial(gpu)]
fn test_batch_euclidean_identical_vectors() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 2.0, 3.0];
        let vectors = vec![1.0, 2.0, 3.0];

        let results = gpu.batch_euclidean_distance(&vectors, &query, 3).unwrap();

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

        let results = gpu.batch_euclidean_distance(&vectors, &query, 3).unwrap();

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
        let results = gpu.batch_dot_product(&[], &[1.0, 0.0, 0.0], 3).unwrap();
        assert!(results.is_empty());
    }
}

#[test]
#[serial(gpu)]
fn test_batch_dot_product_orthogonal() {
    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![0.0, 1.0, 0.0];

        let results = gpu.batch_dot_product(&vectors, &query, 3).unwrap();

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

        let results = gpu.batch_dot_product(&vectors, &query, 3).unwrap();

        assert_eq!(results.len(), 1);
        assert!(
            (results[0] - 29.0).abs() < 0.01,
            "Expected ~29.0, got {}",
            results[0]
        );
    }
}

// =========================================================================
// Plan 04-09 Task 2: Parameter validation errors
// =========================================================================

#[test]
#[serial(gpu)]
fn test_batch_cosine_zero_dimension() {
    if let Some(gpu) = GpuAccelerator::new() {
        // dimension=0 should return empty (early exit in batch_cosine_similarity)
        let results = gpu
            .batch_cosine_similarity(&[1.0, 2.0, 3.0], &[1.0], 0)
            .unwrap();
        assert!(results.is_empty(), "Zero dimension should return empty");
    }
}

#[test]
#[serial(gpu)]
fn test_batch_cosine_dimension_mismatch() {
    if let Some(gpu) = GpuAccelerator::new() {
        // Query is 2D but vectors declared as 3D — should not panic
        // The GPU processes whatever data is there; result may be wrong but no crash
        let query = vec![1.0, 0.0];
        let vectors = vec![1.0, 0.0, 0.0]; // 1 vector of dim 3
        let results = gpu.batch_cosine_similarity(&vectors, &query, 3).unwrap();
        // Should produce 1 result (vectors.len() / dimension = 1)
        assert_eq!(results.len(), 1);
    }
}

#[test]
#[serial(gpu)]
fn test_batch_euclidean_empty_vectors() {
    if let Some(gpu) = GpuAccelerator::new() {
        let results = gpu
            .batch_euclidean_distance(&[], &[1.0, 2.0, 3.0], 3)
            .unwrap();
        assert!(results.is_empty(), "Empty vectors should return empty");
    }
}

#[test]
#[serial(gpu)]
fn test_batch_dot_product_single_element() {
    if let Some(gpu) = GpuAccelerator::new() {
        // Edge case: dimension=1
        let query = vec![3.0];
        let vectors = vec![4.0];
        let results = gpu.batch_dot_product(&vectors, &query, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            (results[0] - 12.0).abs() < 0.01,
            "Expected 3*4=12, got {}",
            results[0]
        );
    }
}

#[test]
#[serial(gpu)]
fn test_batch_cosine_large_batch() {
    if let Some(gpu) = GpuAccelerator::new() {
        // 1000+ vectors of dimension 8
        let dim = 8;
        let num_vectors = 1024;
        let query: Vec<f32> = vec![1.0; dim];
        let vectors: Vec<f32> = vec![1.0; dim * num_vectors];

        let results = gpu.batch_cosine_similarity(&vectors, &query, dim).unwrap();
        assert_eq!(results.len(), num_vectors);
        // All identical vectors → similarity should be ~1.0
        for (i, &r) in results.iter().enumerate() {
            assert!((r - 1.0).abs() < 0.05, "Vector {i}: expected ~1.0, got {r}");
        }
    }
}

// =========================================================================
// Plan 04-09 Task 3: Edge-case inputs (no-panic guarantee)
// =========================================================================

#[test]
#[serial(gpu)]
fn test_gpu_no_panic_on_edge_inputs() {
    if let Some(gpu) = GpuAccelerator::new() {
        // Each case: (vectors, query, dimension)
        let cases: Vec<(&[f32], &[f32], usize)> = vec![
            (&[], &[], 0),
            (&[1.0], &[1.0], 1),
            (&[f32::NAN], &[1.0], 1),
            (&[f32::INFINITY], &[1.0], 1),
            (&[f32::NEG_INFINITY], &[1.0], 1),
            (&[0.0; 1024], &[0.0; 8], 8), // 128 zero vectors of dim 8
        ];

        for (vectors, query, dim) in cases {
            // None of these should panic
            let _ = gpu.batch_cosine_similarity(vectors, query, dim);
            let _ = gpu.batch_euclidean_distance(vectors, query, dim);
            let _ = gpu.batch_dot_product(vectors, query, dim);
        }
    }
}

#[test]
#[serial(gpu)]
fn test_gpu_cosine_zero_norm_vectors() {
    if let Some(gpu) = GpuAccelerator::new() {
        // Zero vector has norm 0 — GPU shader guards against division by zero
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![0.0, 0.0, 0.0]; // Zero vector

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3).unwrap();
        assert_eq!(results.len(), 1);
        // Should return 0.0 (shader checks denom > 0.0)
        assert!(
            results[0].is_finite(),
            "Zero-norm cosine should be finite, got {}",
            results[0]
        );
    }
}

#[test]
#[serial(gpu)]
fn test_gpu_euclidean_zero_dimension() {
    if let Some(gpu) = GpuAccelerator::new() {
        let results = gpu.batch_euclidean_distance(&[1.0], &[1.0], 0).unwrap();
        assert!(results.is_empty(), "Zero dimension should return empty");
    }
}

#[test]
#[serial(gpu)]
fn test_gpu_dot_product_zero_dimension() {
    if let Some(gpu) = GpuAccelerator::new() {
        let results = gpu.batch_dot_product(&[1.0], &[1.0], 0).unwrap();
        assert!(results.is_empty(), "Zero dimension should return empty");
    }
}
