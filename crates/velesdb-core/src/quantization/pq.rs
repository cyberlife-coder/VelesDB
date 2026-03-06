//! Product Quantization (PQ) for aggressive lossy vector compression.
//!
//! PQ splits vectors into multiple subspaces and quantizes each subspace
//! independently with its own codebook (k-means centroids).

use crate::error::Error;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Per-subspace centroid tables learned with k-means.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PQCodebook {
    /// Flattened centroids, indexed as `[subspace][centroid][subspace_dim]`.
    pub centroids: Vec<Vec<Vec<f32>>>,
    /// Full vector dimension.
    pub dimension: usize,
    /// Number of subspaces `m`.
    pub num_subspaces: usize,
    /// Number of centroids `k` per subspace.
    pub num_centroids: usize,
    /// Dimension of each subspace.
    pub subspace_dim: usize,
}

/// Compressed representation of a vector: one centroid id per subspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PQVector {
    /// Selected centroid ids for each subspace.
    pub codes: Vec<u16>,
}

/// Product quantizer model and helpers for train/encode/decode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductQuantizer {
    /// Trained codebook.
    pub codebook: PQCodebook,
}

impl ProductQuantizer {
    /// Train a PQ codebook using simplified k-means for each subspace.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidQuantizerConfig` if:
    /// - `vectors` is empty
    /// - `num_subspaces` is 0
    /// - `num_centroids` is 0 or exceeds `u16::MAX`
    /// - vector dimension is not divisible by `num_subspaces`
    /// - `num_centroids` exceeds `vectors.len()`
    pub fn train(
        vectors: &[Vec<f32>],
        num_subspaces: usize,
        num_centroids: usize,
    ) -> Result<Self, Error> {
        if vectors.is_empty() {
            return Err(Error::InvalidQuantizerConfig(
                "cannot train PQ with empty dataset".into(),
            ));
        }
        if num_subspaces == 0 {
            return Err(Error::InvalidQuantizerConfig(
                "num_subspaces must be > 0".into(),
            ));
        }
        if num_centroids == 0 {
            return Err(Error::InvalidQuantizerConfig(
                "num_centroids must be > 0".into(),
            ));
        }
        if u16::try_from(num_centroids).is_err() {
            return Err(Error::InvalidQuantizerConfig(
                "num_centroids must fit in u16 (max 65535)".into(),
            ));
        }

        let dimension = vectors[0].len();
        if dimension == 0 {
            return Err(Error::InvalidQuantizerConfig(
                "vectors must have non-zero dimension".into(),
            ));
        }
        if !vectors.iter().all(|v| v.len() == dimension) {
            return Err(Error::InvalidQuantizerConfig(
                "all vectors must share the same dimension".into(),
            ));
        }
        if dimension % num_subspaces != 0 {
            return Err(Error::InvalidQuantizerConfig(
                "dimension must be divisible by num_subspaces".into(),
            ));
        }
        if num_centroids > vectors.len() {
            return Err(Error::InvalidQuantizerConfig(format!(
                "num_centroids ({num_centroids}) exceeds number of training vectors ({})",
                vectors.len()
            )));
        }

        let subspace_dim = dimension / num_subspaces;

        let mut centroids = Vec::with_capacity(num_subspaces);

        for subspace in 0..num_subspaces {
            let start = subspace * subspace_dim;
            let end = start + subspace_dim;
            let sub_vectors: Vec<Vec<f32>> =
                vectors.iter().map(|v| v[start..end].to_vec()).collect();
            centroids.push(kmeans_train(&sub_vectors, num_centroids, 25));
        }

        Ok(Self {
            codebook: PQCodebook {
                centroids,
                dimension,
                num_subspaces,
                num_centroids,
                subspace_dim,
            },
        })
    }

    /// Quantize a full-precision vector into PQ codes.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidQuantizerConfig` if `vector.len()` does not match
    /// the codebook dimension.
    pub fn quantize(&self, vector: &[f32]) -> Result<PQVector, Error> {
        if vector.len() != self.codebook.dimension {
            return Err(Error::InvalidQuantizerConfig(format!(
                "vector dimension mismatch: expected {}, got {}",
                self.codebook.dimension,
                vector.len()
            )));
        }

        let mut codes = Vec::with_capacity(self.codebook.num_subspaces);
        for subspace in 0..self.codebook.num_subspaces {
            let start = subspace * self.codebook.subspace_dim;
            let end = start + self.codebook.subspace_dim;
            let code = nearest_centroid(&vector[start..end], &self.codebook.centroids[subspace]);
            // SAFETY: `num_centroids` is validated to fit in u16 during `train()`.
            // `nearest_centroid` returns an index < num_centroids, so it always fits.
            #[allow(clippy::cast_possible_truncation)]
            codes.push(code as u16);
        }

        Ok(PQVector { codes })
    }

    /// Reconstruct an approximate vector from PQ codes.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidQuantizerConfig` if the number of codes does not
    /// match the number of subspaces, or if a code index is out of range.
    pub fn reconstruct(&self, pq_vector: &PQVector) -> Result<Vec<f32>, Error> {
        if pq_vector.codes.len() != self.codebook.num_subspaces {
            return Err(Error::InvalidQuantizerConfig(format!(
                "code count mismatch: expected {}, got {}",
                self.codebook.num_subspaces,
                pq_vector.codes.len()
            )));
        }

        let mut reconstructed = Vec::with_capacity(self.codebook.dimension);
        for (subspace, &code) in pq_vector.codes.iter().enumerate() {
            let code_idx = usize::from(code);
            if code_idx >= self.codebook.centroids[subspace].len() {
                return Err(Error::InvalidQuantizerConfig(format!(
                    "code index {code_idx} out of range for subspace {subspace} \
                     (max {})",
                    self.codebook.centroids[subspace].len() - 1
                )));
            }
            let centroid = &self.codebook.centroids[subspace][code_idx];
            reconstructed.extend_from_slice(centroid);
        }

        Ok(reconstructed)
    }
}

/// Asymmetric distance computation (ADC): query is f32, candidate is PQ-coded.
///
/// # Panics
///
/// Panics if `query_vector.len() != codebook.dimension` or
/// `pq_vector.codes.len() != codebook.num_subspaces`. These are internal
/// invariant checks (not user-facing).
#[must_use]
pub fn distance_pq_l2(query_vector: &[f32], pq_vector: &PQVector, codebook: &PQCodebook) -> f32 {
    assert_eq!(query_vector.len(), codebook.dimension);
    assert_eq!(pq_vector.codes.len(), codebook.num_subspaces);

    let mut lookup_tables = Vec::with_capacity(codebook.num_subspaces);
    for subspace in 0..codebook.num_subspaces {
        let start = subspace * codebook.subspace_dim;
        let end = start + codebook.subspace_dim;
        let q_sub = &query_vector[start..end];

        let table: Vec<f32> = codebook.centroids[subspace]
            .iter()
            .map(|centroid| l2_squared(q_sub, centroid))
            .collect();
        lookup_tables.push(table);
    }

    pq_vector
        .codes
        .iter()
        .enumerate()
        .map(|(subspace, &code)| lookup_tables[subspace][usize::from(code)])
        .sum::<f32>()
        .sqrt()
}

/// Backward-compatible alias for L2 ADC distance.
#[must_use]
pub fn distance_pq(query_vector: &[f32], pq_vector: &PQVector, codebook: &PQCodebook) -> f32 {
    distance_pq_l2(query_vector, pq_vector, codebook)
}

fn nearest_centroid(vector: &[f32], centroids: &[Vec<f32>]) -> usize {
    let mut best_idx = 0;
    let mut best_dist = f32::MAX;

    for (idx, centroid) in centroids.iter().enumerate() {
        let dist = l2_squared(vector, centroid);
        if dist < best_dist {
            best_dist = dist;
            best_idx = idx;
        }
    }

    best_idx
}

fn l2_squared(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

/// k-means++ initialization: picks well-spread initial centroids.
///
/// Step 1: Choose the first centroid uniformly at random.
/// Step 2: For each subsequent centroid, pick a sample with probability
///         proportional to D(x)^2 (squared distance to nearest existing centroid).
fn kmeans_plusplus_init(samples: &[Vec<f32>], k: usize, rng: &mut impl Rng) -> Vec<Vec<f32>> {
    debug_assert!(!samples.is_empty());
    debug_assert!(k > 0);
    debug_assert!(k <= samples.len());

    let n = samples.len();
    let mut centroids = Vec::with_capacity(k);

    // Step 1: Pick first centroid uniformly at random.
    let first_idx = rng.gen_range(0..n);
    centroids.push(samples[first_idx].clone());

    // Distances from each sample to its nearest centroid (initialized to MAX).
    let mut min_dists = vec![f32::MAX; n];

    // Step 2: Pick remaining centroids proportional to D(x)^2.
    for iter in 1..k {
        // `centroids` is non-empty: we pushed the first centroid before this
        // loop, and each iteration pushes exactly one more, so index `iter - 1`
        // is always valid.
        let last_centroid = &centroids[iter - 1];
        for (i, sample) in samples.iter().enumerate() {
            let dist = l2_squared(sample, last_centroid);
            if dist < min_dists[i] {
                min_dists[i] = dist;
            }
        }

        // Compute cumulative distribution of D(x)^2.
        let total: f64 = min_dists.iter().map(|&d| f64::from(d)).sum();
        if total <= 0.0 {
            // All remaining samples are identical to existing centroids.
            // Fall back to sequential selection for remaining centroids.
            for i in centroids.len()..k {
                centroids.push(samples[i % n].clone());
            }
            break;
        }

        let threshold = rng.gen::<f64>() * total;
        let mut cumulative = 0.0_f64;
        let mut chosen = n - 1; // default to last if rounding issues
        for (i, &d) in min_dists.iter().enumerate() {
            cumulative += f64::from(d);
            if cumulative >= threshold {
                chosen = i;
                break;
            }
        }
        centroids.push(samples[chosen].clone());
    }

    centroids
}

fn kmeans_train(samples: &[Vec<f32>], k: usize, max_iters: usize) -> Vec<Vec<f32>> {
    // Internal invariant: callers validate non-empty samples and k > 0.
    debug_assert!(!samples.is_empty());
    debug_assert!(k > 0);
    let dim = samples[0].len();

    // k-means++ initialization for well-spread initial centroids.
    let mut rng = rand::thread_rng();
    let mut centroids = kmeans_plusplus_init(samples, k.min(samples.len()), &mut rng);
    // If k > samples.len(), pad with cycled samples (shouldn't happen after train() validation).
    while centroids.len() < k {
        centroids.push(samples[centroids.len() % samples.len()].clone());
    }

    let mut assignments = vec![0usize; samples.len()];

    for _ in 0..max_iters {
        let mut changed = false;

        // Assignment step
        for (i, sample) in samples.iter().enumerate() {
            let new_assignment = nearest_centroid(sample, &centroids);
            if assignments[i] != new_assignment {
                assignments[i] = new_assignment;
                changed = true;
            }
        }

        // Update step
        let mut new_centroids = vec![vec![0.0; dim]; k];
        let mut counts = vec![0usize; k];

        for (sample, &cluster) in samples.iter().zip(assignments.iter()) {
            counts[cluster] += 1;
            for (d, &val) in sample.iter().enumerate() {
                new_centroids[cluster][d] += val;
            }
        }

        for cluster in 0..k {
            if counts[cluster] == 0 {
                // Re-seed empty cluster deterministically.
                new_centroids[cluster].clone_from(&samples[cluster % samples.len()]);
            } else {
                // `counts[cluster]` is a cluster-member count bounded by
                // `samples.len()`. In practice sub-vectors number in the
                // thousands at most, well within the 24-bit f32 mantissa
                // (exact for values ≤ 16_777_216), so precision loss is
                // negligible for the centroid update.
                #[allow(clippy::cast_precision_loss)]
                let inv = 1.0_f32 / counts[cluster] as f32;
                for value in new_centroids[cluster].iter_mut().take(dim) {
                    *value *= inv;
                }
            }
        }

        centroids = new_centroids;

        if !changed {
            break;
        }
    }

    centroids
}

#[cfg(test)]
mod tests {
    use super::{distance_pq_l2, ProductQuantizer};
    use crate::error::Error;

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

        let d_near = distance_pq_l2(&query, &near, &pq.codebook);
        let d_far = distance_pq_l2(&query, &far, &pq.codebook);

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
        let bad_pq_vec = super::PQVector { codes: vec![0] };
        let result = pq.reconstruct(&bad_pq_vec);
        assert!(result.is_err());
    }

    #[test]
    fn kmeans_plusplus_init_produces_k_distinct_centroids() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        // Generate 100 random 8-dim vectors in distinct clusters.
        let mut samples = Vec::with_capacity(100);
        for i in 0_u8..100 {
            let offset = f32::from(i / 25) * 10.0;
            let v: Vec<f32> = (0_u8..8).map(|d| offset + f32::from(d) * 0.1).collect();
            samples.push(v);
        }

        let centroids = super::kmeans_plusplus_init(&samples, 4, &mut rng);
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
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(123);

        // Generate 100 random 8-dim vectors with clear clusters.
        let mut samples = Vec::with_capacity(100);
        for i in 0_u8..100 {
            let cluster = f32::from(i / 25) * 100.0;
            let v: Vec<f32> = (0_u8..8).map(|d| cluster + f32::from(d) * 0.01).collect();
            samples.push(v);
        }

        let centroids = super::kmeans_plusplus_init(&samples, 4, &mut rng);

        // No two centroids should be closer than 1e-6 L2 squared.
        for i in 0..centroids.len() {
            for j in (i + 1)..centroids.len() {
                let dist = super::l2_squared(&centroids[i], &centroids[j]);
                assert!(
                    dist > 1e-6,
                    "centroids {i} and {j} too close: L2^2 = {dist}"
                );
            }
        }
    }

    #[test]
    fn kmeans_plusplus_init_k1_returns_single_centroid() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);

        let samples = vec![vec![1.0, 2.0], vec![3.0, 4.0], vec![5.0, 6.0]];
        let centroids = super::kmeans_plusplus_init(&samples, 1, &mut rng);
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
}
