use super::*;

/// Compute brute-force L2 (Euclidean) top-k neighbors for a query.
///
/// Returns the indices of the `top_k` closest vectors by L2 distance.
fn brute_force_l2_top_k(query: &[f32], vectors: &[Vec<f32>], top_k: usize) -> Vec<usize> {
    let mut dists: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let d: f32 = query
                .iter()
                .zip(v.iter())
                .map(|(a, b)| {
                    let diff = a - b;
                    diff * diff
                })
                .sum::<f32>()
                .sqrt();
            (i, d)
        })
        .collect();
    dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    dists.iter().take(top_k).map(|&(i, _)| i).collect()
}

/// Compute RaBitQ approximate top-k neighbors for a query.
fn rabitq_top_k(
    query: &[f32],
    encoded: &[RaBitQVector],
    index: &RaBitQIndex,
    top_k: usize,
) -> Vec<usize> {
    let dists = index.batch_distance(query, encoded);
    let mut ranked: Vec<(usize, f32)> = dists.into_iter().enumerate().collect();
    ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    ranked.iter().take(top_k).map(|&(i, _)| i).collect()
}

/// Compute recall: fraction of `ground_truth` entries present in `predicted`.
#[allow(clippy::cast_precision_loss)]
fn recall_fraction(ground_truth: &[usize], predicted: &[usize]) -> f64 {
    let hits = ground_truth
        .iter()
        .filter(|&&idx| predicted.contains(&idx))
        .count();
    hits as f64 / ground_truth.len() as f64
}

/// Generate random clustered vectors where each cluster center is uniformly
/// distributed and samples have controlled noise around the center.
fn generate_random_clustered_vectors(
    n: usize,
    dim: usize,
    num_clusters: usize,
    noise: f32,
    seed: u64,
) -> Vec<Vec<f32>> {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    let centers: Vec<Vec<f32>> = (0..num_clusters)
        .map(|_| (0..dim).map(|_| rng.gen::<f32>() * 200.0 - 100.0).collect())
        .collect();

    (0..n)
        .map(|i| {
            let cluster = i % num_clusters;
            centers[cluster]
                .iter()
                .map(|&c| c + (rng.gen::<f32>() - 0.5) * noise)
                .collect()
        })
        .collect()
}

/// Compute the centroid (mean) of a set of vectors.
#[allow(clippy::cast_precision_loss)]
fn compute_centroid(vectors: &[Vec<f32>], dim: usize) -> Vec<f32> {
    let mut c = vec![0.0f32; dim];
    for v in vectors {
        for (ci, &vi) in c.iter_mut().zip(v.iter()) {
            *ci += vi;
        }
    }
    let inv = 1.0 / vectors.len() as f32;
    for x in &mut c {
        *x *= inv;
    }
    c
}

/// Build a `RaBitQIndex` with identity rotation and computed centroid.
fn identity_index_with_centroid(vectors: &[Vec<f32>], dim: usize) -> RaBitQIndex {
    let centroid = compute_centroid(vectors, dim);
    let mut rotation = vec![0.0f32; dim * dim];
    for i in 0..dim {
        rotation[i * dim + i] = 1.0;
    }
    RaBitQIndex {
        rotation,
        centroid,
        dimension: dim,
    }
}

/// Compute average recall@k across evenly-spaced queries using RaBitQ.
#[allow(clippy::cast_precision_loss)]
fn rabitq_avg_recall(
    vectors: &[Vec<f32>],
    encoded: &[RaBitQVector],
    index: &RaBitQIndex,
    num_queries: usize,
    top_k: usize,
) -> f64 {
    let n = vectors.len();
    let mut total_recall = 0.0_f64;
    for qi in 0..num_queries {
        let query_idx = qi * (n / num_queries);
        let query = &vectors[query_idx];
        let true_top = brute_force_l2_top_k(query, vectors, top_k);
        let approx_top = rabitq_top_k(query, encoded, index, top_k);
        total_recall += recall_fraction(&true_top, &approx_top);
    }
    total_recall / num_queries as f64
}

/// Create an identity rotation matrix (for testing without training).
fn identity_index(dim: usize) -> RaBitQIndex {
    let mut rotation = vec![0.0f32; dim * dim];
    for i in 0..dim {
        rotation[i * dim + i] = 1.0;
    }
    RaBitQIndex {
        rotation,
        centroid: vec![0.0; dim],
        dimension: dim,
    }
}

#[test]
fn rabitq_bits_length_for_various_dimensions() {
    for &dim in &[64, 128, 768] {
        let index = identity_index(dim);
        let v: Vec<f32> = (0..dim)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let val = (i as f32).mul_add(0.1, -5.0);
                val
            })
            .collect();
        let encoded = index.encode(&v).unwrap();
        let expected_words = dim.div_ceil(64);
        assert_eq!(
            encoded.bits.len(),
            expected_words,
            "dim={dim}: expected {expected_words} u64 words, got {}",
            encoded.bits.len()
        );
    }
}

#[test]
fn rabitq_encode_distance_preserves_relative_ordering() {
    let dim = 64;
    let index = identity_index(dim);

    // Query at origin-ish
    let query: Vec<f32> = vec![1.0; dim];

    // Close vector: same direction
    let close: Vec<f32> = vec![1.1; dim];
    // Far vector: opposite direction
    let far: Vec<f32> = vec![-5.0; dim];

    let enc_close = index.encode(&close).unwrap();
    let enc_far = index.encode(&far).unwrap();

    let d_close = index.distance(&query, &enc_close);
    let d_far = index.distance(&query, &enc_far);

    assert!(
        d_close < d_far,
        "Expected close ({d_close}) < far ({d_far})"
    );
}

#[test]
fn rabitq_xor_popcount_correct_on_known_patterns() {
    // All bits set vs no bits set: maximum Hamming distance
    let all_set = [u64::MAX];
    let none_set = [0u64];

    let xor = all_set[0] ^ none_set[0];
    assert_eq!(xor.count_ones(), 64);

    // Same pattern: zero Hamming distance
    let xor2 = all_set[0] ^ all_set[0];
    assert_eq!(xor2.count_ones(), 0);

    // Known pattern: alternating bits
    let a = 0xAAAA_AAAA_AAAA_AAAAu64;
    let b = 0x5555_5555_5555_5555u64;
    assert_eq!((a ^ b).count_ones(), 64); // all differ
}

#[test]
fn rabitq_distance_non_negative() {
    use rand::{Rng, SeedableRng};
    let dim = 128;
    let index = identity_index(dim);

    let mut rng = rand::rngs::StdRng::seed_from_u64(42);

    for _ in 0..50 {
        let v: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() * 10.0 - 5.0).collect();
        let q: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() * 10.0 - 5.0).collect();
        let encoded = index.encode(&v).unwrap();
        let dist = index.distance(&q, &encoded);
        assert!(dist >= 0.0, "distance should be non-negative, got {dist}");
    }
}

#[test]
fn rabitq_encode_zero_vector_graceful() {
    let dim = 64;
    let index = identity_index(dim);
    let zero_vec = vec![0.0; dim];
    let encoded = index.encode(&zero_vec).unwrap();
    assert!(
        encoded.correction.vector_norm.abs() < f32::EPSILON,
        "zero vector should have zero norm"
    );
    // Bits should be all zero
    assert!(encoded.bits.iter().all(|&b| b == 0));
}

#[test]
fn rabitq_signs_to_bits_packing() {
    // Test with known values
    let mut values = vec![-1.0f32; 128];
    // Set first 3 values positive
    values[0] = 1.0;
    values[1] = 0.5;
    values[2] = 0.0; // >= 0.0, so bit should be set

    let bits = signs_to_bits(&values, 128);
    assert_eq!(bits.len(), 2);
    // First 3 bits should be set in word 0
    assert_eq!(bits[0] & 0b111, 0b111);
    // Bit 3 should NOT be set (values[3] = -1.0)
    assert_eq!(bits[0] & 0b1000, 0);
}

#[test]
fn rabitq_recall_at_10_identity_rotation() {
    // With identity rotation (no training), RaBitQ is essentially binary
    // quantization. Test that it correctly ranks cross-cluster neighbors
    // (coarse ranking). The 85% recall threshold with trained rotation
    // is validated in Task 2's tests after `RaBitQIndex::train` is available.
    let dim = 64;
    let vectors = generate_random_clustered_vectors(1000, dim, 10, 20.0, 12345);
    let index = identity_index_with_centroid(&vectors, dim);
    let encoded: Vec<RaBitQVector> = vectors.iter().map(|v| index.encode(v).unwrap()).collect();

    let avg_recall = rabitq_avg_recall(&vectors, &encoded, &index, 20, 10);
    // Identity rotation gives ~20-30% recall on random clustered data
    // (well above random baseline of 1%). Trained rotation (Task 2)
    // with orthogonal projection achieves 85%+.
    assert!(
        avg_recall >= 0.15,
        "RaBitQ recall@10 = {avg_recall:.3}, expected >= 0.15 with identity rotation"
    );
}

#[test]
fn rabitq_batch_distance_matches_individual() {
    use rand::{Rng, SeedableRng};
    let dim = 64;
    let index = identity_index(dim);

    let mut rng = rand::rngs::StdRng::seed_from_u64(99);

    let vectors: Vec<Vec<f32>> = (0..20)
        .map(|_| (0..dim).map(|_| rng.gen::<f32>() * 10.0 - 5.0).collect())
        .collect();
    let query: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() * 10.0 - 5.0).collect();

    let encoded: Vec<RaBitQVector> = vectors.iter().map(|v| index.encode(v).unwrap()).collect();

    let batch_dists = index.batch_distance(&query, &encoded);
    let individual_dists: Vec<f32> = encoded.iter().map(|e| index.distance(&query, e)).collect();

    for (i, (&bd, &id)) in batch_dists.iter().zip(individual_dists.iter()).enumerate() {
        assert!(
            (bd - id).abs() < 1e-6,
            "mismatch at index {i}: batch={bd}, individual={id}"
        );
    }
}

// ====================================================================
// Task 2: Training + StorageMode + Persistence tests
// ====================================================================

#[cfg(feature = "persistence")]
#[test]
fn rabitq_train_computes_centroid_as_mean() {
    let vectors = vec![
        vec![2.0, 4.0, 6.0, 8.0],
        vec![4.0, 6.0, 8.0, 10.0],
        vec![6.0, 8.0, 10.0, 12.0],
    ];
    let index = RaBitQIndex::train(&vectors, 42).unwrap();
    let expected = [4.0_f32, 6.0, 8.0, 10.0];
    for (i, (&c, &e)) in index.centroid.iter().zip(expected.iter()).enumerate() {
        assert!((c - e).abs() < 1e-5, "centroid[{i}] = {c}, expected {e}");
    }
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_train_rotation_is_orthogonal() {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(77);

    let dim = 32;
    let vectors: Vec<Vec<f32>> = (0..100)
        .map(|_| (0..dim).map(|_| rng.gen::<f32>() * 10.0 - 5.0).collect())
        .collect();

    let index = RaBitQIndex::train(&vectors, 42).unwrap();

    assert_rotation_is_orthogonal(&index.rotation, dim, 1e-4);
}

/// Check that the rotation matrix R satisfies R * Rt approx I within `tol`.
///
/// MGS orthogonality degrades slowly with dimension at f32 precision, so
/// the tolerance is intentionally looser for larger matrices.
#[cfg(feature = "persistence")]
fn assert_rotation_is_orthogonal(r: &[f32], dim: usize, tol: f32) {
    for i in 0..dim {
        for j in 0..dim {
            let mut dot = 0.0f32;
            for k in 0..dim {
                dot += r[i * dim + k] * r[j * dim + k];
            }
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!(
                (dot - expected).abs() < tol,
                "R*Rt[{i}][{j}] = {dot}, expected {expected} (dim={dim}, tol={tol})"
            );
        }
    }
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_rotation_orthogonal_d128() {
    // d=128 is a common embedding dimension (e.g. GloVe, FastText).
    // MGS at f32 remains near-orthogonal at this size; tolerance 5e-4.
    let rotation = generate_orthogonal_matrix(128, 13);
    assert_rotation_is_orthogonal(&rotation, 128, 5e-4);
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_rotation_orthogonal_d512() {
    // d=512 is near the MGS stability boundary at f32.
    // Tolerance is relaxed to 1e-3 to account for accumulated rounding.
    let rotation = generate_orthogonal_matrix(512, 17);
    assert_rotation_is_orthogonal(&rotation, 512, 1e-3);
}

#[test]
fn rabitq_storage_mode_serde_roundtrip() {
    use crate::quantization::StorageMode;

    let mode = StorageMode::RaBitQ;
    let json = serde_json::to_string(&mode).unwrap();
    let deserialized: StorageMode = serde_json::from_str(&json).unwrap();
    assert_eq!(mode, deserialized);
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_train_empty_returns_error() {
    let result = RaBitQIndex::train(&[], 42);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("empty"));
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_train_dim_less_than_64_works() {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(99);

    let dim = 16;
    let vectors: Vec<Vec<f32>> = (0..50)
        .map(|_| (0..dim).map(|_| rng.gen::<f32>()).collect())
        .collect();

    let index = RaBitQIndex::train(&vectors, 42).unwrap();
    assert_eq!(index.dimension, dim);
    assert_eq!(index.rotation.len(), dim * dim);
    assert_eq!(index.centroid.len(), dim);

    // Encode should work
    let encoded = index.encode(&vectors[0]).unwrap();
    assert_eq!(encoded.bits.len(), 1); // 16 bits fits in 1 u64
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_trained_recall_at_10_on_clustered_data() {
    // Full pipeline: train on 500+ vectors, encode, search top-10.
    // RaBitQ is a coarse quantizer (32x compression = 1 bit per dimension).
    // With 128 dimensions and many small clusters, sign bits provide
    // enough resolution for good recall.
    let vectors = generate_random_clustered_vectors(1000, 128, 100, 40.0, 12345);
    let index = RaBitQIndex::train(&vectors, 42).unwrap();
    let encoded: Vec<RaBitQVector> = vectors.iter().map(|v| index.encode(v).unwrap()).collect();

    let avg_recall = rabitq_avg_recall(&vectors, &encoded, &index, 20, 10);
    assert!(
        avg_recall >= 0.85,
        "RaBitQ trained recall@10 = {avg_recall:.3}, expected >= 0.85"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_save_load_roundtrip() {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(55);

    let dim = 32;
    let vectors: Vec<Vec<f32>> = (0..50)
        .map(|_| (0..dim).map(|_| rng.gen::<f32>()).collect())
        .collect();

    let index = RaBitQIndex::train(&vectors, 42).unwrap();
    let dir = tempfile::tempdir().unwrap();
    index.save(dir.path()).unwrap();

    let loaded = RaBitQIndex::load(dir.path())
        .unwrap()
        .expect("index should exist");
    assert_eq!(loaded.dimension, index.dimension);
    assert_eq!(loaded.centroid, index.centroid);
    assert_eq!(loaded.rotation, index.rotation);
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_load_returns_none_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let result = RaBitQIndex::load(dir.path()).unwrap();
    assert!(result.is_none());
}

#[cfg(feature = "persistence")]
#[test]
fn rabitq_save_uses_atomic_write() {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(66);

    let dim = 16;
    let vectors: Vec<Vec<f32>> = (0..20)
        .map(|_| (0..dim).map(|_| rng.gen::<f32>()).collect())
        .collect();

    let index = RaBitQIndex::train(&vectors, 42).unwrap();
    let dir = tempfile::tempdir().unwrap();
    index.save(dir.path()).unwrap();

    // .tmp file should not exist after successful save
    assert!(!dir.path().join("rabitq.idx.tmp").exists());
    // Final file should exist
    assert!(dir.path().join("rabitq.idx").exists());
}
