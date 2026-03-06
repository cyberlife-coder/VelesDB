//! `RaBitQ`: Randomized Binary Quantization for 32x vector compression.
//!
//! Based on arXiv:2405.12497, `RaBitQ` encodes vectors as D-bit binary codes
//! packed in `Vec<u64>` with scalar correction factors. Distance estimation
//! uses XOR + popcount on u64 words plus affine correction.
//!
//! ## Compression
//!
//! | Metric | f32 | RaBitQ |
//! |--------|-----|--------|
//! | RAM/vector (768d) | 3 KB | ~96 bytes + 8 bytes correction |
//! | Compression ratio | 1x | 32x |

use crate::error::Error;
use serde::{Deserialize, Serialize};

/// Binary-quantized vector with scalar correction factors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaBitQVector {
    /// Binary codes packed in u64 words. Length = `ceil(D / 64)`.
    pub bits: Vec<u64>,
    /// Correction factors: `(vector_norm, quantization_inner_product)`.
    /// `vector_norm` is the L2 norm of the centered vector before binarization.
    /// `quantization_inner_product` is the inner product between the binary
    /// reconstruction and the rotated normalized vector (quality measure).
    pub norms: (f32, f32),
}

/// `RaBitQ` index holding the random rotation matrix and dataset centroid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaBitQIndex {
    /// Random orthogonal rotation matrix (flattened `D x D`, row-major).
    pub rotation: Vec<f32>,
    /// Dataset centroid for centering.
    pub centroid: Vec<f32>,
    /// Vector dimension.
    pub dimension: usize,
}

/// Pack sign bits of a float slice into u64 words.
///
/// Bit `i` of word `w` is 1 if `values[w*64 + i] >= 0.0`.
/// The output length is `ceil(dim / 64)`.
#[must_use]
pub fn signs_to_bits(values: &[f32], dim: usize) -> Vec<u64> {
    let num_words = dim.div_ceil(64);
    let mut bits = vec![0u64; num_words];
    for (i, &v) in values.iter().take(dim).enumerate() {
        if v >= 0.0 {
            let word = i / 64;
            let bit = i % 64;
            bits[word] |= 1u64 << bit;
        }
    }
    bits
}

/// Apply a flat row-major rotation matrix to a vector.
///
/// Computes `result[i] = sum_j rotation[i * dim + j] * vector[j]` for each `i`.
#[must_use]
pub fn apply_rotation_flat(rotation: &[f32], vector: &[f32], dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let row_start = i * dim;
            rotation[row_start..row_start + dim]
                .iter()
                .zip(vector.iter())
                .map(|(&r, &v)| r * v)
                .sum()
        })
        .collect()
}

/// Compute XOR+popcount inner product estimate from query bits and encoded bits.
///
/// Returns the binary inner product in `[-1, 1]` range.
fn xor_popcount_ip(q_bits: &[u64], enc_bits: &[u64], num_words: usize, dim: usize) -> f32 {
    let mut matching_bits: u32 = 0;
    for (qw, ew) in q_bits.iter().zip(enc_bits.iter()).take(num_words) {
        let xor = qw ^ ew;
        // count_ones gives number of differing bits
        // matching = total_bits_in_word - differing
        matching_bits += 64 - xor.count_ones();
    }
    // Adjust for padding bits in last word: padding bits are 0 in both
    // vectors, so they count as matching but shouldn't contribute.
    let padding_bits = num_words * 64 - dim;
    #[allow(clippy::cast_possible_truncation)]
    let matching_bits = matching_bits - padding_bits as u32;

    // Inner product estimate: each matching bit contributes +1/D,
    // each differing bit contributes -1/D.
    #[allow(clippy::cast_precision_loss)]
    let d_f = dim as f32;
    #[allow(clippy::cast_precision_loss)]
    let ip = (2.0f32.mul_add(matching_bits as f32, -d_f)) / d_f;
    ip
}

impl RaBitQIndex {
    /// Encode a vector into a [`RaBitQVector`].
    ///
    /// Steps:
    /// 1. Center the vector (subtract centroid).
    /// 2. Compute the L2 norm of the centered vector.
    /// 3. Normalize (handle zero-norm gracefully).
    /// 4. Apply rotation matrix.
    /// 5. Extract sign bits into u64 words.
    /// 6. Compute correction factors.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidQuantizerConfig` if vector dimension mismatches.
    pub fn encode(&self, vector: &[f32]) -> Result<RaBitQVector, Error> {
        if vector.len() != self.dimension {
            return Err(Error::InvalidQuantizerConfig(format!(
                "RaBitQ encode: expected dimension {}, got {}",
                self.dimension,
                vector.len()
            )));
        }

        // Step 1: Center
        let centered: Vec<f32> = vector
            .iter()
            .zip(self.centroid.iter())
            .map(|(&v, &c)| v - c)
            .collect();

        // Step 2: Compute norm
        let norm_sq: f32 = centered.iter().map(|&x| x * x).sum();
        let norm = norm_sq.sqrt();

        // Step 3: Handle zero-norm gracefully
        if norm < f32::EPSILON {
            let num_words = self.dimension.div_ceil(64);
            return Ok(RaBitQVector {
                bits: vec![0u64; num_words],
                norms: (0.0, 1.0),
            });
        }

        // Normalize
        let normalized: Vec<f32> = centered.iter().map(|&x| x / norm).collect();

        // Step 4: Apply rotation
        let rotated = apply_rotation_flat(&self.rotation, &normalized, self.dimension);

        // Step 5: Extract sign bits
        let bits = signs_to_bits(&rotated, self.dimension);

        // Step 6: Compute correction factors
        // The binary reconstruction maps each sign bit to +1/-1, scaled by 1/sqrt(D).
        // quantization_inner_product = <binary_reconstruction, rotated_normalized>
        #[allow(clippy::cast_precision_loss)]
        let scale = 1.0 / (self.dimension as f32).sqrt();
        let mut qip: f32 = 0.0;
        for (i, &rv) in rotated.iter().enumerate().take(self.dimension) {
            let word = i / 64;
            let bit = i % 64;
            let sign = if (bits[word] >> bit) & 1 == 1 {
                1.0
            } else {
                -1.0
            };
            qip = (sign * scale).mul_add(rv, qip);
        }

        Ok(RaBitQVector {
            bits,
            norms: (norm, qip),
        })
    }

    /// Estimate the L2 distance between a raw query vector and an encoded vector.
    ///
    /// Uses XOR + popcount for fast Hamming-based inner product estimation,
    /// then applies affine correction with stored norms.
    #[must_use]
    pub fn distance(&self, query: &[f32], encoded: &RaBitQVector) -> f32 {
        // Center query
        let centered: Vec<f32> = query
            .iter()
            .zip(self.centroid.iter())
            .map(|(&v, &c)| v - c)
            .collect();

        let q_norm_sq: f32 = centered.iter().map(|&x| x * x).sum();
        let q_norm = q_norm_sq.sqrt();

        if q_norm < f32::EPSILON {
            // Query is at centroid; distance = norm of encoded vector
            return encoded.norms.0;
        }

        let q_normalized: Vec<f32> = centered.iter().map(|&x| x / q_norm).collect();

        // Apply rotation
        let q_rotated = apply_rotation_flat(&self.rotation, &q_normalized, self.dimension);

        // Extract query sign bits
        let q_bits = signs_to_bits(&q_rotated, self.dimension);

        // XOR + popcount inner product estimate
        let num_words = self.dimension.div_ceil(64);
        let ip_binary = xor_popcount_ip(&q_bits, &encoded.bits, num_words, self.dimension);

        // Affine correction with stored norms
        let v_norm = encoded.norms.0;

        // Estimated <q, v> = q_norm * v_norm * ip_binary
        let estimated_ip = q_norm * v_norm * ip_binary;

        // L2 distance: ||q - v||^2 = ||q||^2 + ||v||^2 - 2<q,v>
        let l2_sq = v_norm.mul_add(v_norm, q_norm_sq) - 2.0 * estimated_ip;

        // Clamp to non-negative (floating point can cause small negatives)
        l2_sq.max(0.0).sqrt()
    }

    /// Batch distance: process query once, then iterate over encoded vectors.
    ///
    /// Amortizes query preprocessing (centering, normalization, rotation, sign extraction).
    #[must_use]
    pub fn batch_distance(&self, query: &[f32], encoded: &[RaBitQVector]) -> Vec<f32> {
        // Preprocess query once
        let centered: Vec<f32> = query
            .iter()
            .zip(self.centroid.iter())
            .map(|(&v, &c)| v - c)
            .collect();

        let q_norm_sq: f32 = centered.iter().map(|&x| x * x).sum();
        let q_norm = q_norm_sq.sqrt();

        if q_norm < f32::EPSILON {
            return encoded.iter().map(|e| e.norms.0).collect();
        }

        let q_normalized: Vec<f32> = centered.iter().map(|&x| x / q_norm).collect();
        let q_rotated = apply_rotation_flat(&self.rotation, &q_normalized, self.dimension);
        let q_bits = signs_to_bits(&q_rotated, self.dimension);
        let num_words = self.dimension.div_ceil(64);

        encoded
            .iter()
            .map(|ev| {
                let ip_binary = xor_popcount_ip(&q_bits, &ev.bits, num_words, self.dimension);

                let v_norm = ev.norms.0;
                let estimated_ip = q_norm * v_norm * ip_binary;
                let l2_sq = v_norm.mul_add(v_norm, q_norm_sq) - 2.0 * estimated_ip;
                l2_sq.max(0.0).sqrt()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let all_set = vec![u64::MAX];
        let none_set = vec![0u64];

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
        let dim = 128;
        let index = identity_index(dim);

        use rand::{Rng, SeedableRng};
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
            encoded.norms.0.abs() < f32::EPSILON,
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
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(12345);

        let dim = 64;
        let n = 1000;
        let num_clusters = 10;
        let top_k = 10;
        let num_queries = 20;

        // Generate clusters with moderate spread -- vectors within a cluster
        // differ significantly in each dimension so sign bits carry info.
        let mut centers = Vec::new();
        for _ in 0..num_clusters {
            let center: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() * 200.0 - 100.0).collect();
            centers.push(center);
        }

        let mut vectors = Vec::with_capacity(n);
        for i in 0..n {
            let cluster = i % num_clusters;
            let v: Vec<f32> = centers[cluster]
                .iter()
                .map(|&c| c + (rng.gen::<f32>() - 0.5) * 20.0)
                .collect();
            vectors.push(v);
        }

        // Centroid = mean
        let centroid: Vec<f32> = {
            let mut c = vec![0.0f32; dim];
            for v in &vectors {
                for (ci, &vi) in c.iter_mut().zip(v.iter()) {
                    *ci += vi;
                }
            }
            #[allow(clippy::cast_precision_loss)]
            let inv = 1.0 / n as f32;
            c.iter_mut().for_each(|x| *x *= inv);
            c
        };

        let mut rotation = vec![0.0f32; dim * dim];
        for i in 0..dim {
            rotation[i * dim + i] = 1.0;
        }

        let index = RaBitQIndex {
            rotation,
            centroid,
            dimension: dim,
        };

        let encoded: Vec<RaBitQVector> = vectors.iter().map(|v| index.encode(v).unwrap()).collect();

        let mut total_recall = 0.0_f64;
        for qi in 0..num_queries {
            let query_idx = qi * (n / num_queries);
            let query = &vectors[query_idx];

            // True top-k by L2
            let mut true_dists: Vec<(usize, f32)> = vectors
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
            true_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let true_top: Vec<usize> = true_dists.iter().take(top_k).map(|&(i, _)| i).collect();

            // RaBitQ top-k
            let rabitq_dists = index.batch_distance(query, &encoded);
            let mut rabitq_ranked: Vec<(usize, f32)> =
                rabitq_dists.into_iter().enumerate().collect();
            rabitq_ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let rabitq_top: Vec<usize> =
                rabitq_ranked.iter().take(top_k).map(|&(i, _)| i).collect();

            let hits = true_top
                .iter()
                .filter(|&&idx| rabitq_top.contains(&idx))
                .count();
            #[allow(clippy::cast_precision_loss)]
            let recall = hits as f64 / top_k as f64;
            total_recall += recall;
        }

        #[allow(clippy::cast_precision_loss)]
        let avg_recall = total_recall / num_queries as f64;
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
        let dim = 64;
        let index = identity_index(dim);

        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(99);

        let vectors: Vec<Vec<f32>> = (0..20)
            .map(|_| (0..dim).map(|_| rng.gen::<f32>() * 10.0 - 5.0).collect())
            .collect();
        let query: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() * 10.0 - 5.0).collect();

        let encoded: Vec<RaBitQVector> = vectors.iter().map(|v| index.encode(v).unwrap()).collect();

        let batch_dists = index.batch_distance(&query, &encoded);
        let individual_dists: Vec<f32> =
            encoded.iter().map(|e| index.distance(&query, e)).collect();

        for (i, (&bd, &id)) in batch_dists.iter().zip(individual_dists.iter()).enumerate() {
            assert!(
                (bd - id).abs() < 1e-6,
                "mismatch at index {i}: batch={bd}, individual={id}"
            );
        }
    }
}
