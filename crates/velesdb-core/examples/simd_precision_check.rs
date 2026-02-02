//! SIMD Precision Validation Test
//!
//! This test validates that SIMD implementations produce correct results
//! compared to scalar reference implementations.

#![allow(clippy::uninlined_format_args)]
#![allow(clippy::bool_to_int_with_if)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::too_many_lines)]

use velesdb_core::simd_dispatch::{
    cosine_dispatched, dot_product_dispatched, euclidean_dispatched, hamming_dispatched,
};

/// Scalar reference implementation for dot product
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Scalar reference implementation for euclidean distance
fn euclidean_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum::<f32>()
        .sqrt()
}

/// Scalar reference implementation for cosine similarity
fn cosine_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom > 0.0 {
        (dot / denom).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Scalar reference implementation for Hamming distance
fn hamming_scalar(a: &[f32], b: &[f32]) -> u32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b.iter())
        .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
        .count() as u32
}

/// Generate test vectors with various patterns
fn generate_vector(dim: usize, pattern: &str) -> Vec<f32> {
    match pattern {
        "ones" => vec![1.0f32; dim],
        "zeros" => vec![0.0f32; dim],
        "alternating" => (0..dim)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect(),
        "incremental" => (0..dim).map(|i| i as f32 * 0.01).collect(),
        "sinusoidal" => (0..dim).map(|i| (i as f32 * 0.1).sin()).collect(),
        "random_01" => (0..dim)
            .map(|i| ((i * 7 + 3) % 100) as f32 / 100.0)
            .collect(),
        _ => vec![1.0f32; dim],
    }
}

/// Generate binary vector
fn generate_binary_vector(dim: usize, pattern: &str) -> Vec<f32> {
    match pattern {
        "all_ones" => vec![1.0f32; dim],
        "all_zeros" => vec![0.0f32; dim],
        "alternating" => (0..dim)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect(),
        "sparse" => (0..dim)
            .map(|i| if i % 10 == 0 { 1.0 } else { 0.0 })
            .collect(),
        _ => (0..dim)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect(),
    }
}

fn main() {
    println!("╔═══════════════════════════════════════════════════════════════════╗");
    println!("║       SIMD Precision Validation Test - Post-Refactoring          ║");
    println!("╚═══════════════════════════════════════════════════════════════════╝\n");

    let dims = vec![
        1, 2, 3, 4, 5, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256, 257, 383,
        384, 385, 767, 768, 769,
    ];
    let patterns = vec![
        "ones",
        "zeros",
        "alternating",
        "incremental",
        "sinusoidal",
        "random_01",
    ];

    let mut all_passed = true;
    // Note: SIMD operations may have slightly different precision than scalar
    // due to different operation ordering (parallel vs sequential accumulation).
    // For large vectors, relative error can be ~1e-7 * magnitude.
    let epsilon = 5e-3; // Tolerance for f32 SIMD rounding differences on large vectors

    // Test Dot Product
    println!("▶ Testing Dot Product...");
    for dim in &dims {
        for pattern in &patterns {
            let a = generate_vector(*dim, pattern);
            let b = generate_vector(*dim, pattern);

            let scalar_result = dot_product_scalar(&a, &b);
            let simd_result = dot_product_dispatched(&a, &b);

            let diff = (scalar_result - simd_result).abs();
            if diff > epsilon {
                println!(
                    "  ✗ FAILED: dim={}, pattern={}, scalar={:.10}, simd={:.10}, diff={:.10}",
                    dim, pattern, scalar_result, simd_result, diff
                );
                all_passed = false;
            }
        }
    }
    if all_passed {
        println!("  ✓ All dot product tests passed\n");
    }

    // Test Euclidean Distance
    println!("▶ Testing Euclidean Distance...");
    let mut euclidean_passed = true;
    for dim in &dims {
        for pattern in &patterns {
            let a = generate_vector(*dim, pattern);
            let b = generate_vector(*dim, "sinusoidal"); // Different pattern

            let scalar_result = euclidean_scalar(&a, &b);
            let simd_result = euclidean_dispatched(&a, &b);

            let diff = (scalar_result - simd_result).abs();
            if diff > epsilon {
                println!(
                    "  ✗ FAILED: dim={}, pattern={}, scalar={:.10}, simd={:.10}, diff={:.10}",
                    dim, pattern, scalar_result, simd_result, diff
                );
                euclidean_passed = false;
                all_passed = false;
            }
        }
    }
    if euclidean_passed {
        println!("  ✓ All euclidean distance tests passed\n");
    }

    // Test Cosine Similarity
    println!("▶ Testing Cosine Similarity...");
    let mut cosine_passed = true;
    for dim in &dims {
        for pattern in &patterns {
            let a = generate_vector(*dim, pattern);
            let b = generate_vector(*dim, "sinusoidal");

            let scalar_result = cosine_scalar(&a, &b);
            let simd_result = cosine_dispatched(&a, &b);

            let diff = (scalar_result - simd_result).abs();
            if diff > epsilon {
                println!(
                    "  ✗ FAILED: dim={}, pattern={}, scalar={:.10}, simd={:.10}, diff={:.10}",
                    dim, pattern, scalar_result, simd_result, diff
                );
                cosine_passed = false;
                all_passed = false;
            }
        }
    }
    if cosine_passed {
        println!("  ✓ All cosine similarity tests passed\n");
    }

    // Test Hamming Distance
    println!("▶ Testing Hamming Distance...");
    let binary_patterns = vec!["all_ones", "all_zeros", "alternating", "sparse"];
    let mut hamming_passed = true;
    for dim in &dims {
        for pattern in &binary_patterns {
            let a = generate_binary_vector(*dim, pattern);
            let b = generate_binary_vector(*dim, "alternating");

            let scalar_result = hamming_scalar(&a, &b);
            let simd_result = hamming_dispatched(&a, &b);

            if scalar_result != simd_result {
                println!(
                    "  ✗ FAILED: dim={}, pattern={}, scalar={}, simd={}",
                    dim, pattern, scalar_result, simd_result
                );
                hamming_passed = false;
                all_passed = false;
            }
        }
    }
    if hamming_passed {
        println!("  ✓ All hamming distance tests passed\n");
    }

    // Special edge cases
    println!("▶ Testing Edge Cases...");
    let mut edge_passed = true;

    // Empty vectors
    let empty: Vec<f32> = vec![];
    let empty_result = dot_product_dispatched(&empty, &empty);
    if empty_result != 0.0 {
        println!(
            "  ✗ FAILED: Empty vector should return 0.0, got {}",
            empty_result
        );
        edge_passed = false;
        all_passed = false;
    }

    // Single element
    let a = vec![2.5f32];
    let b = vec![4.0f32];
    let single_result = dot_product_dispatched(&a, &b);
    if (single_result - 10.0f32).abs() > epsilon {
        println!(
            "  ✗ FAILED: Single element dot product should be 10.0, got {}",
            single_result
        );
        edge_passed = false;
        all_passed = false;
    }

    // Zero vectors for cosine
    let zeros = vec![0.0f32; 128];
    let ones = vec![1.0f32; 128];
    let cosine_zero = cosine_dispatched(&zeros, &ones);
    if cosine_zero != 0.0 {
        println!(
            "  ✗ FAILED: Cosine of zero vector should be 0.0, got {}",
            cosine_zero
        );
        edge_passed = false;
        all_passed = false;
    }

    // Identical vectors for cosine (should be 1.0)
    let identical = vec![0.5f32; 128];
    let cosine_identical = cosine_dispatched(&identical, &identical);
    if (cosine_identical - 1.0).abs() > epsilon {
        println!(
            "  ✗ FAILED: Cosine of identical vectors should be 1.0, got {}",
            cosine_identical
        );
        edge_passed = false;
        all_passed = false;
    }

    if edge_passed {
        println!("  ✓ All edge case tests passed\n");
    }

    // Summary
    println!("╔═══════════════════════════════════════════════════════════════════╗");
    if all_passed {
        println!("║                    ✓ ALL TESTS PASSED                             ║");
    } else {
        println!("║                    ✗ SOME TESTS FAILED                            ║");
    }
    println!("╚═══════════════════════════════════════════════════════════════════╝");

    std::process::exit(if all_passed { 0 } else { 1 });
}
