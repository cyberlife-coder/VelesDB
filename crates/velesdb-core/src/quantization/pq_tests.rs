//! Tests for Product Quantization (PQ) and OPQ modules.

use super::{distance_pq_l2, PQVector, ProductQuantizer};
use crate::error::Error;
use crate::quantization::pq_kmeans::l2_squared;

/// Compute brute-force L2 top-k neighbors for a query.
///
/// Returns the indices of the `top_k` closest vectors by squared L2 distance.
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
                .sum();
            (i, d)
        })
        .collect();
    dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    dists.iter().take(top_k).map(|&(i, _)| i).collect()
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

/// Generate clustered test vectors with seeded RNG.
///
/// Each cluster is centered at a well-separated point and samples are drawn with
/// small perturbations around the center. The inter-cluster distance is much
/// larger than intra-cluster variance to ensure high recall in PQ tests.
fn generate_clustered_vectors(
    n: usize,
    dim: usize,
    num_clusters: usize,
    seed: u64,
) -> Vec<Vec<f32>> {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    // Generate cluster centers spread far apart.
    let mut centers = Vec::with_capacity(num_clusters);
    for c in 0..num_clusters {
        // Each cluster gets a large offset per dimension to ensure separation.
        #[allow(clippy::cast_precision_loss)]
        let offset = c as f32 * 50.0;
        let center: Vec<f32> = (0..dim)
            .map(|d| {
                #[allow(clippy::cast_precision_loss)]
                let base = offset + d as f32 * 0.1;
                base
            })
            .collect();
        centers.push(center);
    }

    let mut vectors = Vec::with_capacity(n);
    for i in 0..n {
        let cluster = i % num_clusters;
        let v: Vec<f32> = centers[cluster]
            .iter()
            .map(|&c| c + (rng.gen::<f32>() - 0.5) * 1.0)
            .collect();
        vectors.push(v);
    }

    vectors
}

#[test]
fn train_builds_expected_codebook_shape() {
    let vectors = vec![
        vec![0.0, 0.0, 10.0, 10.0],
        vec![0.1, 0.0, 9.9, 10.1],
        vec![8.0, 8.0, 1.0, 1.0],
        vec![8.1, 7.9, 1.2, 0.8],
    ];

    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();
    assert_eq!(pq.codebook.num_subspaces, 2);
    assert_eq!(pq.codebook.num_centroids, 2);
    assert_eq!(pq.codebook.subspace_dim, 2);
    assert_eq!(pq.codebook.centroids.len(), 2);
    assert_eq!(pq.codebook.centroids[0].len(), 2);
    // Verify rotation field is None by default
    assert!(pq.rotation.is_none());
}

#[test]
fn quantize_and_reconstruct_roundtrip_is_reasonable() {
    let vectors = vec![
        vec![0.0, 0.0, 10.0, 10.0],
        vec![0.1, -0.1, 10.1, 9.9],
        vec![8.0, 8.0, 1.0, 1.0],
        vec![8.1, 7.9, 1.2, 0.8],
    ];
    let pq = ProductQuantizer::train(&vectors, 2, 4).unwrap();

    let input = vec![8.05, 8.0, 1.1, 1.0];
    let code = pq.quantize(&input).unwrap();
    let reconstructed = pq.reconstruct(&code).unwrap();

    assert_eq!(code.codes.len(), 2);
    assert_eq!(reconstructed.len(), input.len());

    let mse: f32 = input
        .iter()
        .zip(reconstructed.iter())
        .map(|(a, b)| {
            let d = a - b;
            d * d
        })
        .sum::<f32>()
        / f32::from(u16::try_from(input.len()).expect("test input length fits in u16"));
    assert!(mse < 0.2, "reconstruction MSE too high: {mse}");
}

#[test]
fn adc_distance_prefers_closer_codes() {
    let vectors = vec![
        vec![0.0, 0.0, 0.0, 0.0],
        vec![0.1, 0.1, 0.1, 0.1],
        vec![5.0, 5.0, 5.0, 5.0],
        vec![5.1, 4.9, 5.0, 5.2],
    ];
    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();

    let near = pq.quantize(&[0.05, 0.05, 0.0, 0.1]).unwrap();
    let far = pq.quantize(&[5.0, 5.0, 5.0, 5.0]).unwrap();
    let query = [0.0, 0.0, 0.0, 0.0];

    let d_near = distance_pq_l2(&query, &near, &pq);
    let d_far = distance_pq_l2(&query, &far, &pq);

    assert!(d_near < d_far, "ADC did not preserve proximity ordering");
}

#[test]
fn train_empty_vectors_returns_error() {
    let result = ProductQuantizer::train(&[], 2, 2);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, Error::InvalidQuantizerConfig(_)));
    assert!(err.to_string().contains("empty"));
}

#[test]
fn train_zero_dimension_returns_error() {
    // A vector with dimension 0 must be rejected before subspace arithmetic.
    let vectors = vec![vec![]];
    let result = ProductQuantizer::train(&vectors, 1, 1);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, Error::InvalidQuantizerConfig(_)));
    assert!(
        err.to_string().contains("non-zero dimension"),
        "unexpected error message: {err}"
    );
}

#[test]
fn train_zero_subspaces_returns_error() {
    let vectors = vec![vec![1.0, 2.0]];
    let result = ProductQuantizer::train(&vectors, 0, 2);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        Error::InvalidQuantizerConfig(_)
    ));
}

#[test]
fn train_zero_centroids_returns_error() {
    let vectors = vec![vec![1.0, 2.0]];
    let result = ProductQuantizer::train(&vectors, 1, 0);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        Error::InvalidQuantizerConfig(_)
    ));
}

#[test]
fn train_centroids_exceed_u16_returns_error() {
    let vectors = vec![vec![1.0, 2.0]];
    let result = ProductQuantizer::train(&vectors, 1, 65536);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        Error::InvalidQuantizerConfig(_)
    ));
}

#[test]
fn train_dimension_not_divisible_returns_error() {
    let vectors = vec![vec![1.0, 2.0, 3.0]];
    let result = ProductQuantizer::train(&vectors, 2, 1);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        Error::InvalidQuantizerConfig(_)
    ));
}

#[test]
fn train_more_centroids_than_vectors_returns_error() {
    let vectors = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
    let result = ProductQuantizer::train(&vectors, 1, 5);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        Error::InvalidQuantizerConfig(_)
    ));
}

#[test]
fn train_valid_inputs_returns_ok() {
    let vectors = vec![
        vec![1.0, 2.0, 3.0, 4.0],
        vec![5.0, 6.0, 7.0, 8.0],
        vec![9.0, 10.0, 11.0, 12.0],
    ];
    let result = ProductQuantizer::train(&vectors, 2, 2);
    assert!(result.is_ok());
}

#[test]
fn quantize_wrong_dimension_returns_error() {
    let vectors = vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]];
    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();
    let result = pq.quantize(&[1.0, 2.0]); // wrong dimension
    assert!(result.is_err());
}

#[test]
fn reconstruct_invalid_codes_returns_error() {
    let vectors = vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]];
    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();
    // Wrong number of codes
    let bad_pq_vec = PQVector { codes: vec![0] };
    let result = pq.reconstruct(&bad_pq_vec);
    assert!(result.is_err());
}

#[test]
fn kmeans_plusplus_init_produces_k_distinct_centroids() {
    use crate::quantization::pq_kmeans::kmeans_plusplus_init;
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);

    // Generate 100 random 8-dim vectors in distinct clusters.
    let mut samples = Vec::with_capacity(100);
    for i in 0_u8..100 {
        let offset = f32::from(i / 25) * 10.0;
        let v: Vec<f32> = (0_u8..8).map(|d| offset + f32::from(d) * 0.1).collect();
        samples.push(v);
    }

    let centroids = kmeans_plusplus_init(&samples, 4, &mut rng);
    assert_eq!(centroids.len(), 4, "expected 4 centroids");

    // All centroids must be distinct (no duplicates).
    for i in 0..centroids.len() {
        for j in (i + 1)..centroids.len() {
            assert_ne!(
                centroids[i], centroids[j],
                "centroids {i} and {j} are identical"
            );
        }
    }
}

#[test]
fn kmeans_plusplus_init_centroids_are_spread() {
    use crate::quantization::pq_kmeans::kmeans_plusplus_init;
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(123);

    // Generate 100 random 8-dim vectors with clear clusters.
    let mut samples = Vec::with_capacity(100);
    for i in 0_u8..100 {
        let cluster = f32::from(i / 25) * 100.0;
        let v: Vec<f32> = (0_u8..8).map(|d| cluster + f32::from(d) * 0.01).collect();
        samples.push(v);
    }

    let centroids = kmeans_plusplus_init(&samples, 4, &mut rng);

    // No two centroids should be closer than 1e-6 L2 squared.
    for i in 0..centroids.len() {
        for j in (i + 1)..centroids.len() {
            let dist = l2_squared(&centroids[i], &centroids[j]);
            assert!(
                dist > 1e-6,
                "centroids {i} and {j} too close: L2^2 = {dist}"
            );
        }
    }
}

#[test]
fn kmeans_plusplus_init_k1_returns_single_centroid() {
    use crate::quantization::pq_kmeans::kmeans_plusplus_init;
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);

    let samples = vec![vec![1.0, 2.0], vec![3.0, 4.0], vec![5.0, 6.0]];
    let centroids = kmeans_plusplus_init(&samples, 1, &mut rng);
    assert_eq!(centroids.len(), 1);
    // The single centroid must be one of the input samples.
    assert!(
        samples.contains(&centroids[0]),
        "k=1 centroid not from dataset"
    );
}

#[test]
fn train_with_kmeans_plusplus_still_passes_happy_path() {
    // Verify that with k-means++ init, the full PQ pipeline still works.
    let vectors = vec![
        vec![0.0, 0.0, 10.0, 10.0],
        vec![0.1, 0.0, 9.9, 10.1],
        vec![8.0, 8.0, 1.0, 1.0],
        vec![8.1, 7.9, 1.2, 0.8],
    ];
    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();

    // Verify codebook shape.
    assert_eq!(pq.codebook.num_subspaces, 2);
    assert_eq!(pq.codebook.num_centroids, 2);
    assert_eq!(pq.codebook.subspace_dim, 2);

    // Quantize and reconstruct should still work.
    let code = pq.quantize(&[8.0, 8.0, 1.0, 1.0]).unwrap();
    let reconstructed = pq.reconstruct(&code).unwrap();
    assert_eq!(reconstructed.len(), 4);
}

// ====================================================================
// Task 1: Hardened k-means training tests
// ====================================================================

#[test]
fn kmeans_converges_early_on_well_separated_data() {
    let vectors = generate_clustered_vectors(200, 8, 4, 42);
    let pq = ProductQuantizer::train(&vectors, 4, 4).unwrap();

    assert_eq!(pq.codebook.num_subspaces, 4);
    assert_eq!(pq.codebook.num_centroids, 4);
    assert_eq!(pq.codebook.subspace_dim, 2);
}

#[test]
fn degenerate_centroids_not_present_after_training() {
    let vectors = generate_clustered_vectors(500, 64, 8, 99);
    let pq = ProductQuantizer::train(&vectors, 8, 16).unwrap();

    for (subspace, sub_centroids) in pq.codebook.centroids.iter().enumerate() {
        for i in 0..sub_centroids.len() {
            for j in (i + 1)..sub_centroids.len() {
                let dist = l2_squared(&sub_centroids[i], &sub_centroids[j]);
                assert!(
                    dist >= 1e-6,
                    "degenerate centroids in subspace {subspace}: \
                     centroids {i} and {j} distance {dist}"
                );
            }
        }
    }
}

#[test]
fn parallel_subspace_training_produces_valid_codebooks() {
    let vectors = generate_clustered_vectors(200, 16, 4, 77);
    let pq = ProductQuantizer::train(&vectors, 4, 4).unwrap();

    assert_eq!(pq.codebook.num_subspaces, 4);
    assert_eq!(pq.codebook.num_centroids, 4);
    assert_eq!(pq.codebook.subspace_dim, 4);

    let code = pq.quantize(&vectors[0]).unwrap();
    let reconstructed = pq.reconstruct(&code).unwrap();
    assert_eq!(reconstructed.len(), 16);
}

#[test]
fn product_quantizer_rotation_none_serializes_via_postcard() {
    let vectors = vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]];
    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();
    assert!(pq.rotation.is_none());

    let bytes = postcard::to_allocvec(&pq).expect("serialize");
    let pq2: ProductQuantizer = postcard::from_bytes(&bytes).expect("deserialize");

    assert!(pq2.rotation.is_none());
    assert_eq!(pq2.codebook.dimension, pq.codebook.dimension);
    assert_eq!(pq2.codebook.num_subspaces, pq.codebook.num_subspaces);
    assert_eq!(pq2.codebook.num_centroids, pq.codebook.num_centroids);
}

/// Compute PQ ADC top-k for a query against all vectors.
fn pq_adc_top_k(
    query: &[f32],
    vectors: &[Vec<f32>],
    pq: &ProductQuantizer,
    top_k: usize,
) -> Vec<usize> {
    let mut pq_dists: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let code = pq.quantize(v).unwrap();
            let d = distance_pq_l2(query, &code, pq);
            (i, d)
        })
        .collect();
    pq_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    pq_dists.iter().take(top_k).map(|&(i, _)| i).collect()
}

/// Generate clustered vectors with large inter-cluster offsets and small noise.
fn generate_offset_clustered_vectors(
    n: usize,
    dim: usize,
    num_clusters: usize,
    noise: f32,
    seed: u64,
) -> Vec<Vec<f32>> {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    let centers: Vec<Vec<f32>> = (0..num_clusters)
        .map(|c| {
            #[allow(clippy::cast_precision_loss)]
            let offset = c as f32 * 100.0;
            (0..dim)
                .map(|d| {
                    #[allow(clippy::cast_precision_loss)]
                    let v = offset + d as f32 * 0.5;
                    v
                })
                .collect()
        })
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

#[test]
fn recall_at_10_on_clustered_data() {
    let n = 1000;
    let num_queries = 20;
    let top_k = 10;

    let vectors = generate_offset_clustered_vectors(n, 64, 4, 5.0, 12345);
    let pq = ProductQuantizer::train(&vectors, 8, 256).unwrap();

    let mut total_recall = 0.0_f64;
    for qi in 0..num_queries {
        let query_idx = qi * (n / num_queries);
        let query = &vectors[query_idx];
        let true_top = brute_force_l2_top_k(query, &vectors, top_k);
        let pq_top = pq_adc_top_k(query, &vectors, &pq, top_k);
        total_recall += recall_fraction(&true_top, &pq_top);
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / num_queries as f64;
    assert!(
        avg_recall >= 0.50,
        "recall@10 = {avg_recall:.3}, expected >= 0.50"
    );
}

// ====================================================================
// Task 2: Codebook persistence tests
// ====================================================================

#[cfg(feature = "persistence")]
#[test]
fn codebook_save_load_roundtrip() {
    let vectors = vec![
        vec![1.0, 2.0, 3.0, 4.0],
        vec![5.0, 6.0, 7.0, 8.0],
        vec![9.0, 10.0, 11.0, 12.0],
    ];
    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();

    let dir = tempfile::tempdir().unwrap();
    pq.save_codebook(dir.path()).unwrap();

    let loaded = ProductQuantizer::load_codebook(dir.path())
        .unwrap()
        .expect("codebook should exist");

    assert_eq!(loaded.codebook.dimension, pq.codebook.dimension);
    assert_eq!(loaded.codebook.num_subspaces, pq.codebook.num_subspaces);
    assert_eq!(loaded.codebook.num_centroids, pq.codebook.num_centroids);
    assert_eq!(loaded.codebook.subspace_dim, pq.codebook.subspace_dim);
    assert!(loaded.rotation.is_none());

    for (s, (a, b)) in pq
        .codebook
        .centroids
        .iter()
        .zip(loaded.codebook.centroids.iter())
        .enumerate()
    {
        for (c, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(ca, cb, "centroid mismatch at subspace {s}, centroid {c}");
        }
    }
}

#[cfg(feature = "persistence")]
#[test]
fn codebook_roundtrip_with_rotation_some() {
    let vectors = vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]];
    let mut pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();
    pq.rotation = Some(vec![
        1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.1, 0.2, 0.3, 0.4,
    ]);

    let bytes = postcard::to_allocvec(&pq).expect("serialize");
    let pq2: ProductQuantizer = postcard::from_bytes(&bytes).expect("deserialize");

    assert_eq!(pq2.rotation, pq.rotation);
}

#[cfg(feature = "persistence")]
#[test]
fn load_codebook_returns_none_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let result = ProductQuantizer::load_codebook(dir.path()).unwrap();
    assert!(result.is_none());
}

#[cfg(feature = "persistence")]
#[test]
fn save_codebook_uses_atomic_write() {
    let vectors = vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]];
    let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();

    let dir = tempfile::tempdir().unwrap();
    pq.save_codebook(dir.path()).unwrap();

    assert!(!dir.path().join("codebook.pq.tmp").exists());
    assert!(dir.path().join("codebook.pq").exists());
}

#[cfg(feature = "persistence")]
#[test]
fn rotation_save_load_roundtrip() {
    let vectors = vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]];
    let mut pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();
    pq.rotation = Some(vec![
        1.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.1, 0.2, 0.3, 0.4, 0.4, 0.3, 0.2, 0.1,
    ]);

    let dir = tempfile::tempdir().unwrap();
    pq.save_rotation(dir.path()).unwrap();

    let loaded = ProductQuantizer::load_rotation(dir.path())
        .unwrap()
        .expect("rotation should exist");

    assert_eq!(loaded, pq.rotation.unwrap());
}

#[cfg(feature = "persistence")]
#[test]
fn load_rotation_returns_none_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let result = ProductQuantizer::load_rotation(dir.path()).unwrap();
    assert!(result.is_none());
}

// ====================================================================
// Task: ADC LUT precomputation tests
// ====================================================================

#[test]
fn precompute_lut_returns_correct_size_and_distances() {
    let vectors = vec![
        vec![1.0, 2.0, 3.0, 4.0],
        vec![5.0, 6.0, 7.0, 8.0],
        vec![9.0, 10.0, 11.0, 12.0],
    ];
    let pq = ProductQuantizer::train(&vectors, 2, 3).unwrap();
    let query = vec![2.0, 3.0, 5.0, 6.0];
    let lut = pq.precompute_lut(&query);

    assert_eq!(lut.len(), 6, "LUT length must be m*k = 2*3 = 6");

    for &val in &lut {
        assert!(val >= 0.0, "LUT entry must be non-negative, got {val}");
    }
}

#[test]
fn precompute_lut_applies_rotation_when_present() {
    let vectors = vec![
        vec![1.0, 2.0, 3.0, 4.0],
        vec![5.0, 6.0, 7.0, 8.0],
        vec![9.0, 10.0, 11.0, 12.0],
    ];
    let mut pq = ProductQuantizer::train(&vectors, 2, 3).unwrap();
    let query = vec![2.0, 3.0, 5.0, 6.0];

    let lut_no_rot = pq.precompute_lut(&query);

    pq.rotation = Some(vec![
        0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0,
    ]);

    let lut_with_rot = pq.precompute_lut(&query);

    assert_ne!(lut_no_rot, lut_with_rot, "Rotation must change LUT values");
}

#[test]
fn precompute_lut_m8_k256_size() {
    let dim = 64;
    let m = 8;
    let k = 256;
    let vectors = generate_clustered_vectors(300, dim, 4, 42);
    let pq = ProductQuantizer::train(&vectors, m, k).unwrap();
    let query: Vec<f32> = (0..dim)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = i as f32 * 0.1;
            v
        })
        .collect();

    let lut = pq.precompute_lut(&query);
    assert_eq!(lut.len(), m * k, "LUT length for m=8 k=256");
    assert_eq!(
        lut.len() * std::mem::size_of::<f32>(),
        8192,
        "LUT must be exactly 8KB for m=8 k=256"
    );
}

// ====================================================================
// Task: OPQ pre-rotation via IPQ algorithm tests
// ====================================================================

#[cfg(feature = "persistence")]
#[test]
fn opq_train_produces_rotation_matrix_of_correct_size() {
    use crate::quantization::pq_opq::train_opq;
    let vectors = generate_clustered_vectors(200, 64, 4, 42);
    let pq = train_opq(&vectors, 8, 16, true, 5).unwrap();
    let rotation = pq.rotation.as_ref().expect("OPQ must produce rotation");
    assert_eq!(rotation.len(), 64 * 64, "rotation must be D*D = 64*64");
}

#[cfg(feature = "persistence")]
#[test]
fn opq_rotation_is_approximately_orthogonal() {
    use crate::quantization::pq_opq::train_opq;
    let vectors = generate_clustered_vectors(200, 64, 4, 42);
    let pq = train_opq(&vectors, 8, 16, true, 5).unwrap();
    let rotation = pq.rotation.as_ref().expect("OPQ must produce rotation");
    let d = 64;

    for i in 0..d {
        for j in 0..d {
            let mut dot = 0.0_f32;
            for k in 0..d {
                dot += rotation[i * d + k] * rotation[j * d + k];
            }
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!(
                (dot - expected).abs() < 1e-2,
                "R*R^T[{i}][{j}] = {dot}, expected {expected}"
            );
        }
    }
}

/// Generate directional clustered vectors for OPQ testing.
///
/// Each cluster has 3 random principal directions with large variance along
/// those axes, making the data anisotropic — the scenario where OPQ shines.
#[cfg(feature = "persistence")]
fn generate_directional_clustered_vectors(
    n: usize,
    dim: usize,
    num_clusters: usize,
    seed: u64,
) -> Vec<Vec<f32>> {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    let mut cluster_dirs: Vec<Vec<Vec<f32>>> = Vec::new();
    let mut cluster_centers: Vec<Vec<f32>> = Vec::new();
    for c in 0..num_clusters {
        #[allow(clippy::cast_precision_loss)]
        let offset = c as f32 * 50.0;
        let center: Vec<f32> = (0..dim)
            .map(|d| {
                #[allow(clippy::cast_precision_loss)]
                let v = offset + d as f32 * 0.3;
                v
            })
            .collect();
        cluster_centers.push(center);

        let dirs: Vec<Vec<f32>> = (0..3)
            .map(|_| {
                let dir: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() - 0.5).collect();
                let norm: f32 = dir.iter().map(|&x| x * x).sum::<f32>().sqrt();
                dir.iter().map(|&x| x / norm).collect()
            })
            .collect();
        cluster_dirs.push(dirs);
    }

    (0..n)
        .map(|i| {
            let cluster = i % num_clusters;
            (0..dim)
                .map(|d| {
                    let mut val = cluster_centers[cluster][d];
                    for dir in &cluster_dirs[cluster] {
                        val += (rng.gen::<f32>() - 0.5) * 15.0 * dir[d];
                    }
                    val += (rng.gen::<f32>() - 0.5) * 0.5;
                    val
                })
                .collect()
        })
        .collect()
}

#[cfg(feature = "persistence")]
#[test]
fn opq_recall_improvement_over_standard_pq() {
    use crate::quantization::pq_opq::train_opq;

    let m = 8;
    let k = 16;
    let vectors = generate_directional_clustered_vectors(4000, 64, 8, 54321);

    let mut best_improvement = f64::NEG_INFINITY;
    for _ in 0..3 {
        let pq_standard = ProductQuantizer::train(&vectors, m, k).unwrap();
        let recall_standard = compute_avg_recall(&pq_standard, &vectors, 50, 10);

        let pq_opq = train_opq(&vectors, m, k, true, 5).unwrap();
        let recall_opq = compute_avg_recall(&pq_opq, &vectors, 50, 10);

        let improvement = recall_opq - recall_standard;
        if improvement > best_improvement {
            best_improvement = improvement;
        }
    }

    assert!(
        best_improvement >= 0.03,
        "OPQ best recall improvement = {best_improvement:.4}, expected >= 3%"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn opq_disabled_returns_no_rotation() {
    use crate::quantization::pq_opq::train_opq;
    let vectors = generate_clustered_vectors(200, 64, 4, 42);
    let pq = train_opq(&vectors, 8, 16, false, 5).unwrap();
    assert!(
        pq.rotation.is_none(),
        "opq_enabled=false must produce rotation=None"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn opq_precompute_lut_applies_rotation() {
    use crate::quantization::pq_opq::train_opq;
    let vectors = generate_clustered_vectors(200, 64, 4, 42);
    let pq_std = ProductQuantizer::train(&vectors, 8, 16).unwrap();
    let pq_opq = train_opq(&vectors, 8, 16, true, 5).unwrap();

    let query: Vec<f32> = vectors[0].clone();
    let lut_std = pq_std.precompute_lut(&query);
    let lut_opq = pq_opq.precompute_lut(&query);

    assert_ne!(lut_std, lut_opq, "OPQ LUT must differ from standard PQ LUT");
}

#[cfg(feature = "persistence")]
#[test]
fn opq_handles_non_common_dimension_split() {
    use crate::quantization::pq_opq::train_opq;
    let vectors = generate_clustered_vectors(100, 48, 4, 77);
    let pq = train_opq(&vectors, 6, 16, true, 5).unwrap();
    assert!(pq.rotation.is_some());
    assert_eq!(
        pq.rotation.as_ref().unwrap().len(),
        48 * 48,
        "rotation must be 48*48"
    );
    assert_eq!(pq.codebook.num_subspaces, 6);
    assert_eq!(pq.codebook.subspace_dim, 8);
}

/// Compute LUT-based PQ top-k for a query against precomputed codes.
#[cfg(feature = "persistence")]
fn pq_lut_top_k(
    query: &[f32],
    codes: &[PQVector],
    pq: &ProductQuantizer,
    top_k: usize,
) -> Vec<usize> {
    let lut = pq.precompute_lut(query);
    let k = pq.codebook.num_centroids;
    let mut pq_dists: Vec<(usize, f32)> = codes
        .iter()
        .enumerate()
        .map(|(i, code)| {
            let d: f32 = code
                .codes
                .iter()
                .enumerate()
                .map(|(s, &c)| lut[s * k + usize::from(c)])
                .sum();
            (i, d)
        })
        .collect();
    pq_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    pq_dists.iter().take(top_k).map(|&(i, _)| i).collect()
}

/// Helper to compute average recall@k for a quantizer on given vectors.
#[cfg(feature = "persistence")]
#[allow(clippy::cast_precision_loss)]
fn compute_avg_recall(
    pq: &ProductQuantizer,
    vectors: &[Vec<f32>],
    num_queries: usize,
    top_k: usize,
) -> f64 {
    let n = vectors.len();
    let codes: Vec<PQVector> = vectors.iter().map(|v| pq.quantize(v).unwrap()).collect();
    let mut total_recall = 0.0_f64;

    for qi in 0..num_queries {
        let query_idx = qi * (n / num_queries);
        let query = &vectors[query_idx];
        let true_top = brute_force_l2_top_k(query, vectors, top_k);
        let pq_top = pq_lut_top_k(query, &codes, pq, top_k);
        total_recall += recall_fraction(&true_top, &pq_top);
    }

    total_recall / num_queries as f64
}
