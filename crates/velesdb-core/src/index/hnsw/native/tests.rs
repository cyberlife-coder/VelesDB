//! Tests for native HNSW implementation.

#![allow(clippy::cast_precision_loss)]

use super::distance::{CpuDistance, SimdDistance};
use super::graph::NativeHnsw;
use crate::distance::DistanceMetric;

#[test]
fn test_native_hnsw_basic_insert_search() {
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

    // Insert 100 vectors
    for i in 0..100_u64 {
        let v: Vec<f32> = (0..128).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
        hnsw.insert(v);
    }

    assert_eq!(hnsw.len(), 100);

    // Search for first vector
    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search(&query, 10, 50);

    assert_eq!(results.len(), 10);
    // First result should be node 0 or very close
    assert!(results[0].1 < 0.1, "First result should be very close");
}

#[test]
fn test_native_hnsw_recall() {
    let engine = SimdDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 32, 200, 10_000);

    // Insert 1000 vectors
    let vectors: Vec<Vec<f32>> = (0..1000)
        .map(|i| {
            (0..768)
                .map(|j| ((i * 768 + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v.clone());
    }

    // Test recall with multiple queries
    let mut total_recall = 0.0;
    let n_queries = 10;
    let k = 10;

    for q_idx in 0..n_queries {
        let query = &vectors[q_idx * 100]; // Use existing vectors as queries

        // Get HNSW results
        let hnsw_results: Vec<usize> = hnsw
            .search(query, k, 128)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        // Compute ground truth (brute force)
        let mut distances: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let dist = cosine_distance(query, v);
                (i, dist)
            })
            .collect();
        distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let ground_truth: Vec<usize> = distances.iter().take(k).map(|(i, _)| *i).collect();

        // Calculate recall
        let hits = hnsw_results
            .iter()
            .filter(|id| ground_truth.contains(id))
            .count();
        total_recall += hits as f64 / k as f64;
    }

    let avg_recall = total_recall / n_queries as f64;
    assert!(
        avg_recall >= 0.8,
        "Recall should be at least 80%, got {:.1}%",
        avg_recall * 100.0
    );
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        1.0
    } else {
        1.0 - (dot / (norm_a * norm_b))
    }
}

#[test]
fn test_cpu_vs_simd_consistency() {
    let cpu_engine = CpuDistance::new(DistanceMetric::Euclidean);
    let simd_engine = SimdDistance::new(DistanceMetric::Euclidean);

    let cpu_hnsw = NativeHnsw::new(cpu_engine, 16, 100, 100);
    let simd_hnsw = NativeHnsw::new(simd_engine, 16, 100, 100);

    // Insert same vectors
    for i in 0..50_u64 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32).collect();
        cpu_hnsw.insert(v.clone());
        simd_hnsw.insert(v);
    }

    // Search should return similar results
    let query: Vec<f32> = (0..64).map(|j| j as f32).collect();
    let cpu_results = cpu_hnsw.search(&query, 5, 30);
    let simd_results = simd_hnsw.search(&query, 5, 30);

    // First result should match
    assert_eq!(
        cpu_results[0].0, simd_results[0].0,
        "CPU and SIMD should find same nearest neighbor"
    );
}
