//! Tests for `gpu_backend` module
//!
//! GPU tests must run serially to avoid deadlocks on shared GPU resources.
//! Each test creates a new wgpu instance which can conflict with parallel execution.

use std::sync::Arc;

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
        let results = gpu.batch_cosine_similarity(&[1.0, 2.0, 3.0], &[1.0], 0);
        let values = results.expect("zero-dimension cosine must return Ok(empty)");
        assert!(values.is_empty(), "Zero dimension should return empty");
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
        let results = gpu.batch_cosine_similarity(&vectors, &query, 3);
        // Should produce 1 result (vectors.len() / dimension = 1)
        let values = results.expect("dimension mismatch should not panic");
        assert_eq!(values.len(), 1);
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

        let results = gpu.batch_cosine_similarity(&vectors, &query, dim);
        let values = results.expect("large cosine batch should succeed");
        assert_eq!(values.len(), num_vectors);
        // All identical vectors → similarity should be ~1.0
        for (i, &r) in values.iter().enumerate() {
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

        let results = gpu.batch_cosine_similarity(&vectors, &query, 3);
        let values = results.expect("zero-norm cosine should return Ok");
        assert_eq!(values.len(), 1);
        // Should return 0.0 (shader checks denom > 0.0)
        assert!(
            values[0].is_finite(),
            "Zero-norm cosine should be finite, got {}",
            values[0]
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

// =========================================================================
// GPU-vs-SIMD correctness tests (Phase 1)
// =========================================================================

#[test]
#[serial(gpu)]
fn test_batch_euclidean_gpu_matches_simd() {
    if let Some(gpu) = GpuAccelerator::new() {
        let dim = 4;
        let query = vec![1.0, 0.0, 0.0, 0.0];
        let vectors = vec![
            0.0, 1.0, 0.0, 0.0, // vec 0
            1.0, 0.0, 0.0, 0.0, // vec 1 (identical to query)
            2.0, 0.0, 0.0, 0.0, // vec 2
        ];

        let gpu_results = gpu
            .batch_euclidean_distance(&vectors, &query, dim)
            .expect("GPU euclidean must succeed");

        let simd_results: Vec<f32> = (0..3)
            .map(|i| {
                let v = &vectors[i * dim..(i + 1) * dim];
                crate::simd_native::euclidean_native(&query, v)
            })
            .collect();

        for (gpu_val, simd_val) in gpu_results.iter().zip(simd_results.iter()) {
            assert!(
                (gpu_val - simd_val).abs() < 1e-5,
                "GPU={gpu_val} vs SIMD={simd_val}"
            );
        }
    }
}

#[test]
#[serial(gpu)]
fn test_batch_dot_product_gpu_matches_simd() {
    if let Some(gpu) = GpuAccelerator::new() {
        let dim = 4;
        let query = vec![1.0, 0.0, 0.0, 0.0];
        let vectors = vec![
            0.0, 1.0, 0.0, 0.0, // vec 0
            1.0, 0.0, 0.0, 0.0, // vec 1 (identical to query)
            2.0, 0.0, 0.0, 0.0, // vec 2
        ];

        let gpu_results = gpu
            .batch_dot_product(&vectors, &query, dim)
            .expect("GPU dot product must succeed");

        let simd_results: Vec<f32> = (0..3)
            .map(|i| {
                let v = &vectors[i * dim..(i + 1) * dim];
                crate::simd_native::dot_product_native(&query, v)
            })
            .collect();

        for (gpu_val, simd_val) in gpu_results.iter().zip(simd_results.iter()) {
            assert!(
                (gpu_val - simd_val).abs() < 1e-5,
                "GPU={gpu_val} vs SIMD={simd_val}"
            );
        }
    }
}

// =========================================================================
// Multi-pipeline compilation tests
// =========================================================================

/// Verifies that `GpuAccelerator::new()` compiles all three compute pipelines
/// (cosine, euclidean, dot_product) without error. Construction would fail
/// if any WGSL shader had a syntax error or entry-point mismatch.
#[test]
#[serial(gpu)]
fn test_gpu_has_all_pipelines() {
    if let Some(gpu) = GpuAccelerator::new() {
        // Exercise each pipeline through its public method to confirm compilation.
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![1.0, 0.0, 0.0];

        let cosine = gpu.batch_cosine_similarity(&vectors, &query, 3);
        assert!(cosine.is_ok(), "cosine pipeline must produce results");

        let euclidean = gpu.batch_euclidean_distance(&vectors, &query, 3);
        assert!(euclidean.is_ok(), "euclidean pipeline must produce results");

        let dot = gpu.batch_dot_product(&vectors, &query, 3);
        assert!(dot.is_ok(), "dot_product pipeline must produce results");
    }
}

// =========================================================================
// batch_distance_for_metric unified dispatch tests
// =========================================================================

#[test]
#[serial(gpu)]
fn test_batch_distance_for_metric_cosine() {
    use crate::distance::DistanceMetric;

    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![1.0, 0.0, 0.0];

        let result = gpu.batch_distance_for_metric(DistanceMetric::Cosine, &vectors, &query, 3);
        let values = result.expect("Cosine must return Some").unwrap();
        assert_eq!(values.len(), 1);
        assert!((values[0] - 1.0).abs() < 0.01, "Identical vectors -> ~1.0");
    }
}

#[test]
#[serial(gpu)]
fn test_batch_distance_for_metric_euclidean() {
    use crate::distance::DistanceMetric;

    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![0.0, 0.0, 0.0];
        let vectors = vec![3.0, 4.0, 0.0];

        let result = gpu.batch_distance_for_metric(DistanceMetric::Euclidean, &vectors, &query, 3);
        let values = result.expect("Euclidean must return Some").unwrap();
        assert_eq!(values.len(), 1);
        assert!(
            (values[0] - 5.0).abs() < 0.01,
            "Expected ~5.0, got {}",
            values[0]
        );
    }
}

#[test]
#[serial(gpu)]
fn test_batch_distance_for_metric_dot_product() {
    use crate::distance::DistanceMetric;

    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![2.0, 3.0, 4.0];
        let vectors = vec![2.0, 3.0, 4.0]; // 4+9+16 = 29

        let result = gpu.batch_distance_for_metric(DistanceMetric::DotProduct, &vectors, &query, 3);
        let values = result.expect("DotProduct must return Some").unwrap();
        assert_eq!(values.len(), 1);
        assert!(
            (values[0] - 29.0).abs() < 0.01,
            "Expected ~29.0, got {}",
            values[0]
        );
    }
}

#[test]
#[serial(gpu)]
fn test_batch_distance_for_metric_unsupported_returns_none() {
    use crate::distance::DistanceMetric;

    if let Some(gpu) = GpuAccelerator::new() {
        let query = vec![1.0, 0.0, 0.0];
        let vectors = vec![1.0, 0.0, 0.0];

        assert!(
            gpu.batch_distance_for_metric(DistanceMetric::Hamming, &vectors, &query, 3)
                .is_none(),
            "Hamming must return None (no GPU pipeline)"
        );
        assert!(
            gpu.batch_distance_for_metric(DistanceMetric::Jaccard, &vectors, &query, 3)
                .is_none(),
            "Jaccard must return None (no GPU pipeline)"
        );
    }
}

// =========================================================================
// Singleton tests
// =========================================================================

/// Verifies that `GpuAccelerator::global()` does not panic when called
/// from inside a tokio async runtime (i.e., `pollster::block_on` must
/// not nest inside tokio's reactor).
#[test]
#[serial(gpu)]
#[cfg(feature = "persistence")]
fn test_gpu_global_async_safe() {
    let rt = tokio::runtime::Runtime::new().expect("invariant: tokio runtime must init");
    // If init_device() uses pollster::block_on directly, this will panic.
    // After the async-safety fix, it delegates to a background thread.
    rt.block_on(async {
        let _gpu = GpuAccelerator::global();
        // No assertion on Some/None — GPU may not be available.
        // Absence of panic is the test.
    });
}

#[test]
#[serial(gpu)]
fn test_gpu_global_thread_safe() {
    let handle_a = std::thread::spawn(GpuAccelerator::global);
    let handle_b = std::thread::spawn(GpuAccelerator::global);

    let result_a = handle_a.join().expect("thread A must not panic");
    let result_b = handle_b.join().expect("thread B must not panic");

    match (&result_a, &result_b) {
        (Some(a), Some(b)) => {
            assert!(
                Arc::ptr_eq(a, b),
                "global() from different threads must return the same Arc instance"
            );
        }
        (None, None) => {
            // No GPU available on either thread — consistent.
        }
        _ => panic!("global() returned inconsistent results across threads"),
    }
}

// =========================================================================
// GPU rerank threshold tests (Phase 2)
// =========================================================================

#[test]
#[serial(gpu)]
fn test_should_rerank_gpu_threshold() {
    if let Some(gpu) = GpuAccelerator::global() {
        let crossover = gpu.calibration().crossover_floats();

        // Well below crossover -> false
        assert!(
            !gpu.should_rerank_gpu(1, 1),
            "1 float is always below crossover ({crossover})"
        );

        // Exactly at crossover -> false (strictly greater required)
        if crossover > 0 && crossover < usize::MAX {
            assert!(
                !gpu.should_rerank_gpu(crossover, 1),
                "Exactly at crossover ({crossover}) should NOT trigger GPU"
            );
        }

        // Just above crossover -> true
        if crossover < usize::MAX {
            assert!(
                gpu.should_rerank_gpu(crossover + 1, 1),
                "Above crossover ({crossover}) should trigger GPU"
            );
        }
    }
}

#[test]
#[serial(gpu)]
fn test_gpu_global_returns_same_arc() {
    let first = GpuAccelerator::global();
    let second = GpuAccelerator::global();

    match (&first, &second) {
        (Some(a), Some(b)) => {
            assert!(
                Arc::ptr_eq(a, b),
                "global() must return the same Arc instance"
            );
        }
        (None, None) => {
            // No GPU available — both must be None
        }
        _ => panic!("global() returned inconsistent results (one Some, one None)"),
    }
}

// =========================================================================
// GPU auto-calibration tests (Phase C)
// =========================================================================

#[test]
#[serial(gpu)]
fn test_gpu_calibration_produces_valid_values() {
    if let Some(gpu) = GpuAccelerator::global() {
        let cal = gpu.calibration();
        // Overhead must be > 0 (there is always a fixed cost)
        assert!(cal.overhead_ns() > 0, "GPU overhead should be > 0");
        // Crossover must be > 0 (at minimum a few floats)
        assert!(cal.crossover_floats() > 0, "crossover should be > 0");
    }
}

// =========================================================================
// GPU brute-force calibrated threshold tests (Phase C — Step C.6)
// =========================================================================

#[test]
#[serial(gpu)]
fn test_should_brute_force_gpu_uses_calibration() {
    if let Some(gpu) = GpuAccelerator::global() {
        let crossover = gpu.calibration().crossover_floats();

        // Below crossover -> false (use dim=128 as a realistic dimension)
        if crossover > 128 {
            assert!(
                !gpu.should_brute_force_gpu(1, 128),
                "1 * 128 = 128 floats should be below crossover ({crossover})"
            );
        }

        // Above crossover -> true
        if crossover < usize::MAX / 2 {
            let n = crossover / 128 + 1;
            assert!(
                gpu.should_brute_force_gpu(n, 128),
                "{n} * 128 floats should be above crossover ({crossover})"
            );
        }
    }
}

// =========================================================================
// GPU PQ calibrated threshold tests (Phase C — Step C.8)
// =========================================================================

#[test]
#[serial(gpu)]
fn test_should_use_gpu_pq_uses_calibration() {
    if let Some(gpu) = GpuAccelerator::global() {
        let crossover = gpu.calibration().crossover_floats();

        // Tiny workload -> false (n * k * sd = 1)
        if crossover > 100 {
            assert!(
                !gpu.should_use_gpu_pq(1, 1, 1),
                "1 * 1 * 1 = 1 float should be below crossover ({crossover})"
            );
        }

        // Large workload -> true
        if crossover < usize::MAX / 3 {
            assert!(
                gpu.should_use_gpu_pq(crossover + 1, 1, 1),
                "crossover + 1 should exceed crossover ({crossover})"
            );
        }
    }
}

// =========================================================================
// GPU auto-calibration fallback tests (Phase C — Steps C.11 & C.12)
// =========================================================================

#[test]
fn test_gpu_calibration_default_has_conservative_values() {
    let cal = GpuCalibration::default();
    // Default values must match the old hardcoded thresholds (conservative).
    assert_eq!(
        cal.overhead_ns(),
        900_000,
        "Default overhead should be 900us"
    );
    assert_eq!(
        cal.crossover_floats(),
        262_144,
        "Default crossover should match old rerank threshold"
    );
}

#[test]
fn test_gpu_calibration_default_is_safe_fallback() {
    let cal = GpuCalibration::default();
    // With default values, all should_* methods must be callable without crash.
    // The crossover 262_144 means GPU is activated for > 262K floats (conservative).
    assert!(cal.crossover_floats() > 0, "crossover must be positive");
    assert!(
        cal.crossover_floats() < usize::MAX,
        "GPU must not be totally disabled by default"
    );
}

#[test]
#[serial(gpu)]
fn test_should_rerank_gpu_uses_calibrated_crossover() {
    if let Some(gpu) = GpuAccelerator::global() {
        let crossover = gpu.calibration().crossover_floats();
        // Below the crossover -> false
        if crossover > 100 {
            assert!(
                !gpu.should_rerank_gpu(1, crossover - 1),
                "Below crossover ({crossover}) should NOT trigger GPU"
            );
        }
        // Above the crossover -> true (unless crossover == usize::MAX)
        if crossover < usize::MAX / 2 {
            assert!(
                gpu.should_rerank_gpu(crossover + 1, 1),
                "Above crossover ({crossover}) should trigger GPU"
            );
        }
    }
}
