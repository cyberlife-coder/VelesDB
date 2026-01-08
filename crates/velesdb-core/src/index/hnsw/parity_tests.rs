//! Parity tests: `NativeHnswInner` vs `HnswInner` (`hnsw_rs`)
//!
//! These tests verify that our native HNSW implementation produces
//! results equivalent to the `hnsw_rs` library, ensuring we can safely
//! replace it.

#![allow(clippy::cast_precision_loss)]

#[cfg(test)]
mod tests {
    use crate::distance::DistanceMetric;
    use crate::index::hnsw::inner::HnswInner;
    use crate::index::hnsw::native_inner::NativeHnswInner;

    const DIM: usize = 64;
    const NUM_VECTORS: usize = 500;
    const K: usize = 10;
    const EF_SEARCH: usize = 100;
    const RECALL_THRESHOLD: f32 = 0.85; // Native should achieve at least 85% recall vs hnsw_rs

    /// Generate deterministic test vectors
    fn generate_vectors(count: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut vectors = Vec::with_capacity(count);
        let mut state = seed;

        for _ in 0..count {
            let mut vec = Vec::with_capacity(dim);
            for _ in 0..dim {
                // Simple LCG PRNG
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let val = ((state >> 33) as f32) / (u32::MAX as f32) * 2.0 - 1.0;
                vec.push(val);
            }
            vectors.push(vec);
        }
        vectors
    }

    /// Normalize a vector to unit length (for `DotProduct` metric)
    fn normalize(vec: &[f32]) -> Vec<f32> {
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            vec.iter().map(|x| x / norm).collect()
        } else {
            vec.to_vec()
        }
    }

    /// Generate normalized vectors (required for `DotProduct` with `hnsw_rs`)
    fn generate_normalized_vectors(count: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
        generate_vectors(count, dim, seed)
            .into_iter()
            .map(|v| normalize(&v))
            .collect()
    }

    /// Calculate recall: how many of native's results are in `hnsw_rs` results
    fn calculate_recall(native_ids: &[usize], hnsw_ids: &[usize]) -> f32 {
        if hnsw_ids.is_empty() {
            return if native_ids.is_empty() { 1.0 } else { 0.0 };
        }

        let matches = native_ids.iter().filter(|id| hnsw_ids.contains(id)).count();

        matches as f32 / hnsw_ids.len() as f32
    }

    // =========================================================================
    // Euclidean Distance Parity
    // =========================================================================

    #[test]
    fn test_parity_euclidean_insert_count() {
        let vectors = generate_vectors(NUM_VECTORS, DIM, 12345);

        // hnsw_rs
        let hnsw = HnswInner::new(DistanceMetric::Euclidean, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            hnsw.insert((v, i));
        }

        // Native
        let native = NativeHnswInner::new(DistanceMetric::Euclidean, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            native.insert((v, i));
        }

        assert_eq!(native.len(), NUM_VECTORS);
    }

    #[test]
    fn test_parity_euclidean_search_recall() {
        let vectors = generate_vectors(NUM_VECTORS, DIM, 12345);
        let queries = generate_vectors(20, DIM, 67890);

        // Build hnsw_rs index
        let hnsw = HnswInner::new(DistanceMetric::Euclidean, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            hnsw.insert((v, i));
        }

        // Build native index
        let native = NativeHnswInner::new(DistanceMetric::Euclidean, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            native.insert((v, i));
        }

        // Compare search results
        let mut total_recall = 0.0;
        for query in &queries {
            let hnsw_results = hnsw.search(query, K, EF_SEARCH);
            let native_results = native.search(query, K, EF_SEARCH);

            let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|n| n.d_id).collect();
            let native_ids: Vec<usize> = native_results.iter().map(|n| n.d_id).collect();

            total_recall += calculate_recall(&native_ids, &hnsw_ids);
        }

        let avg_recall = total_recall / queries.len() as f32;
        println!("Euclidean parity recall: {:.2}%", avg_recall * 100.0);

        assert!(
            avg_recall >= RECALL_THRESHOLD,
            "Native recall {:.2}% is below threshold {:.2}%",
            avg_recall * 100.0,
            RECALL_THRESHOLD * 100.0
        );
    }

    // =========================================================================
    // Cosine Distance Parity
    // =========================================================================

    #[test]
    fn test_parity_cosine_search_recall() {
        let vectors = generate_vectors(NUM_VECTORS, DIM, 11111);
        let queries = generate_vectors(20, DIM, 22222);

        // Build hnsw_rs index
        let hnsw = HnswInner::new(DistanceMetric::Cosine, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            hnsw.insert((v, i));
        }

        // Build native index
        let native = NativeHnswInner::new(DistanceMetric::Cosine, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            native.insert((v, i));
        }

        // Compare search results
        let mut total_recall = 0.0;
        for query in &queries {
            let hnsw_results = hnsw.search(query, K, EF_SEARCH);
            let native_results = native.search(query, K, EF_SEARCH);

            let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|n| n.d_id).collect();
            let native_ids: Vec<usize> = native_results.iter().map(|n| n.d_id).collect();

            total_recall += calculate_recall(&native_ids, &hnsw_ids);
        }

        let avg_recall = total_recall / queries.len() as f32;
        println!("Cosine parity recall: {:.2}%", avg_recall * 100.0);

        assert!(
            avg_recall >= RECALL_THRESHOLD,
            "Native recall {:.2}% is below threshold {:.2}%",
            avg_recall * 100.0,
            RECALL_THRESHOLD * 100.0
        );
    }

    // =========================================================================
    // Dot Product Distance Parity
    // =========================================================================

    #[test]
    fn test_parity_dot_product_search_recall() {
        // DotProduct requires normalized vectors for hnsw_rs (DistDot)
        let vectors = generate_normalized_vectors(NUM_VECTORS, DIM, 33333);
        let queries = generate_normalized_vectors(20, DIM, 44444);

        // Build hnsw_rs index
        let hnsw = HnswInner::new(DistanceMetric::DotProduct, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            hnsw.insert((v, i));
        }

        // Build native index
        let native = NativeHnswInner::new(DistanceMetric::DotProduct, 16, NUM_VECTORS, 100);
        for (i, v) in vectors.iter().enumerate() {
            native.insert((v, i));
        }

        // Compare search results
        let mut total_recall = 0.0;
        for query in &queries {
            let hnsw_results = hnsw.search(query, K, EF_SEARCH);
            let native_results = native.search(query, K, EF_SEARCH);

            let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|n| n.d_id).collect();
            let native_ids: Vec<usize> = native_results.iter().map(|n| n.d_id).collect();

            total_recall += calculate_recall(&native_ids, &hnsw_ids);
        }

        let avg_recall = total_recall / queries.len() as f32;
        println!("DotProduct parity recall: {:.2}%", avg_recall * 100.0);

        assert!(
            avg_recall >= RECALL_THRESHOLD,
            "Native recall {:.2}% is below threshold {:.2}%",
            avg_recall * 100.0,
            RECALL_THRESHOLD * 100.0
        );
    }

    // =========================================================================
    // Transform Score Parity
    // =========================================================================

    #[test]
    fn test_parity_transform_score_all_metrics() {
        let test_distances = [0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0];

        for metric in [
            DistanceMetric::Euclidean,
            DistanceMetric::Cosine,
            DistanceMetric::DotProduct,
        ] {
            let hnsw = HnswInner::new(metric, 16, 100, 100);
            let native = NativeHnswInner::new(metric, 16, 100, 100);

            for &dist in &test_distances {
                let hnsw_score = hnsw.transform_score(dist);
                let native_score = native.transform_score(dist);

                assert!(
                    (hnsw_score - native_score).abs() < 0.001,
                    "Metric {metric:?}: transform_score({dist}) differs - hnsw_rs={hnsw_score}, native={native_score}"
                );
            }
        }
    }

    // =========================================================================
    // Parallel Insert Parity
    // =========================================================================

    #[test]
    fn test_parity_parallel_insert() {
        let vectors = generate_vectors(200, DIM, 55555);
        let data: Vec<(&Vec<f32>, usize)> =
            vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();

        // hnsw_rs
        let hnsw = HnswInner::new(DistanceMetric::Euclidean, 16, 200, 100);
        hnsw.parallel_insert(&data);

        // Native
        let native = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 200, 100);
        native.parallel_insert(&data);

        // Both should have same count
        assert_eq!(native.len(), 200);

        // Search should work on both
        let query = &vectors[0];
        let hnsw_results = hnsw.search(query, 5, 50);
        let native_results = native.search(query, 5, 50);

        assert!(!hnsw_results.is_empty());
        assert!(!native_results.is_empty());
    }

    // =========================================================================
    // Edge Cases
    // =========================================================================

    #[test]
    fn test_parity_empty_index_search() {
        let hnsw = HnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
        let native = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);

        let query = vec![0.0; DIM];
        let hnsw_results = hnsw.search(&query, 5, 50);
        let native_results = native.search(&query, 5, 50);

        assert!(hnsw_results.is_empty());
        assert!(native_results.is_empty());
    }

    #[test]
    fn test_parity_single_vector() {
        let vector = vec![1.0; DIM];

        let hnsw = HnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
        hnsw.insert((&vector, 0));

        let native = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
        native.insert((&vector, 0));

        let hnsw_results = hnsw.search(&vector, 1, 50);
        let native_results = native.search(&vector, 1, 50);

        assert_eq!(hnsw_results.len(), 1);
        assert_eq!(native_results.len(), 1);
        assert_eq!(hnsw_results[0].d_id, 0);
        assert_eq!(native_results[0].d_id, 0);
    }

    #[test]
    fn test_parity_k_larger_than_index() {
        let vectors = generate_vectors(5, DIM, 77777);

        let hnsw = HnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
        let native = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);

        for (i, v) in vectors.iter().enumerate() {
            hnsw.insert((v, i));
            native.insert((v, i));
        }

        let query = &vectors[0];
        let hnsw_results = hnsw.search(query, 10, 50); // k > num vectors
        let native_results = native.search(query, 10, 50);

        // Both should return at most 5 results
        assert!(hnsw_results.len() <= 5);
        assert!(native_results.len() <= 5);
    }
}
