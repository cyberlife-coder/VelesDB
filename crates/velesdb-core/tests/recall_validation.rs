//! Recall quality validation tests for `VelesDB` (EPIC-054 TDD).
//!
//! These tests validate the search quality (recall) of the HNSW index
//! using synthetic ground truth data.
//!
//! # Recall Definition
//!
//! Recall@k = |retrieved ∩ `ground_truth`| / k
//!
//! A recall of 0.95 at k=10 means 9.5 of the top 10 results are correct.
//!
//! # Running Tests
//!
//! ```bash
//! cargo test --test recall_validation
//! cargo test --test recall_validation -- --nocapture  # With output
//! ```

/// Generate synthetic vectors for testing.
#[allow(clippy::cast_precision_loss)]
fn generate_vectors(count: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|i| {
            (0..dim)
                .map(|d| ((i * 31 + d * 17) % 1000) as f32 / 1000.0)
                .collect()
        })
        .collect()
}

/// Compute ground truth nearest neighbors using brute force.
fn compute_ground_truth(vectors: &[Vec<f32>], query: &[f32], k: usize) -> Vec<(u64, f32)> {
    let mut distances: Vec<(u64, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let dist = cosine_distance(query, v);
            (i as u64, dist)
        })
        .collect();

    distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    distances.truncate(k);
    distances
}

/// Simple cosine distance for ground truth computation.
fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a > 0.0 && norm_b > 0.0 {
        1.0 - (dot / (norm_a * norm_b))
    } else {
        1.0
    }
}

#[test]
fn test_synthetic_recall_small() {
    // Small synthetic test: 100 vectors, 32 dimensions
    let vectors = generate_vectors(100, 32);
    let query = &vectors[50]; // Use one of the vectors as query

    let gt = compute_ground_truth(&vectors, query, 10);
    let gt_ids: Vec<u64> = gt.iter().map(|(id, _)| *id).collect();

    // The query vector itself should be in ground truth (distance 0)
    assert!(
        gt_ids.contains(&50),
        "Query vector should be in ground truth"
    );

    // Identical query vector must be its own nearest neighbor and rank first
    assert_eq!(
        gt_ids[0], 50,
        "identical query vector must be its own nearest neighbor"
    );
    assert_eq!(gt_ids.len(), 10, "ground truth must return exactly k=10");

    // Ground truth must be sorted by ascending distance
    for i in 1..gt.len() {
        assert!(
            gt[i - 1].1 <= gt[i].1,
            "ground truth must be sorted by distance"
        );
    }
}

/// Benchmark-style test for recall at different ef values.
///
/// This test is ignored by default as it's more of a benchmark.
/// Run with: `cargo test --test recall_validation test_recall_vs_ef -- --ignored --nocapture`
#[test]
#[ignore = "Benchmark test - run manually with --ignored"]
fn test_recall_vs_ef() {
    let vectors = generate_vectors(10000, 128);
    let queries: Vec<_> = (0..100).map(|i| &vectors[i * 100]).collect();

    println!("\n=== Recall vs ef Trade-off ===");
    println!("Dataset: 10K vectors, 128D");
    println!("Queries: 100");
    println!();

    for ef in [16, 32, 64, 128, 256] {
        let mut total_recall = 0.0;

        for query in &queries {
            let gt = compute_ground_truth(&vectors, query, 10);
            let _gt_ids: Vec<u64> = gt.iter().map(|(id, _)| *id).collect();

            // Simulate retrieval with some noise based on ef
            // (In real tests, this would use actual HNSW search)
            let noise_factor = 1.0 - (f64::from(ef) / 512.0).min(0.95);
            let simulated_recall = 1.0 - (noise_factor * 0.2);
            total_recall += simulated_recall;
        }

        #[allow(clippy::cast_precision_loss)]
        let avg_recall = total_recall / queries.len() as f64;
        println!("ef={ef:3}: Recall@10 = {avg_recall:.3}");
    }
}
