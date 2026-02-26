//! Product Quantization (PQ) for aggressive lossy vector compression.
//!
//! PQ splits vectors into multiple subspaces and quantizes each subspace
//! independently with its own codebook (k-means centroids).

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
    #[must_use]
    pub fn train(vectors: &[Vec<f32>], num_subspaces: usize, num_centroids: usize) -> Self {
        assert!(!vectors.is_empty(), "Cannot train PQ with empty dataset");
        assert!(num_subspaces > 0, "num_subspaces must be > 0");
        assert!(num_centroids > 0, "num_centroids must be > 0");
        assert!(
            num_centroids <= usize::from(u16::MAX),
            "num_centroids must fit in u16 (max 65535)"
        );

        let dimension = vectors[0].len();
        assert!(
            vectors.iter().all(|v| v.len() == dimension),
            "All vectors must share the same dimension"
        );
        assert!(
            dimension.is_multiple_of(num_subspaces),
            "Dimension must be divisible by num_subspaces"
        );

        let subspace_dim = dimension / num_subspaces;
        let mut centroids = Vec::with_capacity(num_subspaces);

        for subspace in 0..num_subspaces {
            let start = subspace * subspace_dim;
            let end = start + subspace_dim;
            let sub_vectors: Vec<Vec<f32>> =
                vectors.iter().map(|v| v[start..end].to_vec()).collect();
            centroids.push(kmeans_train(&sub_vectors, num_centroids, 25));
        }

        Self {
            codebook: PQCodebook {
                centroids,
                dimension,
                num_subspaces,
                num_centroids,
                subspace_dim,
            },
        }
    }

    /// Quantize a full-precision vector into PQ codes.
    #[must_use]
    pub fn quantize(&self, vector: &[f32]) -> PQVector {
        assert_eq!(vector.len(), self.codebook.dimension);

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

        PQVector { codes }
    }

    /// Reconstruct an approximate vector from PQ codes.
    #[must_use]
    pub fn reconstruct(&self, pq_vector: &PQVector) -> Vec<f32> {
        assert_eq!(pq_vector.codes.len(), self.codebook.num_subspaces);

        let mut reconstructed = Vec::with_capacity(self.codebook.dimension);
        for (subspace, &code) in pq_vector.codes.iter().enumerate() {
            let centroid = &self.codebook.centroids[subspace][usize::from(code)];
            reconstructed.extend_from_slice(centroid);
        }

        reconstructed
    }
}

/// Asymmetric distance computation (ADC): query is f32, candidate is PQ-coded.
#[must_use]
pub fn distance_pq(query_vector: &[f32], pq_vector: &PQVector, codebook: &PQCodebook) -> f32 {
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

fn kmeans_train(samples: &[Vec<f32>], k: usize, max_iters: usize) -> Vec<Vec<f32>> {
    assert!(!samples.is_empty());
    let dim = samples[0].len();

    // Deterministic init: first k (cycled if needed).
    let mut centroids: Vec<Vec<f32>> = (0..k).map(|i| samples[i % samples.len()].clone()).collect();

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
                new_centroids[cluster] = samples[cluster % samples.len()].clone();
            } else {
                let inv = 1.0 / counts[cluster] as f32;
                for d in 0..dim {
                    new_centroids[cluster][d] *= inv;
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
    use super::{distance_pq, ProductQuantizer};

    #[test]
    fn train_builds_expected_codebook_shape() {
        let vectors = vec![
            vec![0.0, 0.0, 10.0, 10.0],
            vec![0.1, 0.0, 9.9, 10.1],
            vec![8.0, 8.0, 1.0, 1.0],
            vec![8.1, 7.9, 1.2, 0.8],
        ];

        let pq = ProductQuantizer::train(&vectors, 2, 2);
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
        let pq = ProductQuantizer::train(&vectors, 2, 4);

        let input = vec![8.05, 8.0, 1.1, 1.0];
        let code = pq.quantize(&input);
        let reconstructed = pq.reconstruct(&code);

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
            / input.len() as f32;
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
        let pq = ProductQuantizer::train(&vectors, 2, 2);

        let near = pq.quantize(&[0.05, 0.05, 0.0, 0.1]);
        let far = pq.quantize(&[5.0, 5.0, 5.0, 5.0]);
        let query = [0.0, 0.0, 0.0, 0.0];

        let d_near = distance_pq(&query, &near, &pq.codebook);
        let d_far = distance_pq(&query, &far, &pq.codebook);

        assert!(d_near < d_far, "ADC did not preserve proximity ordering");
    }
}
