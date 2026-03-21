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

/// Scalar correction factors for a `RaBitQ`-encoded vector.
///
/// These values are needed to apply the affine correction during distance estimation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaBitQCorrection {
    /// L2 norm of the centered vector before binarization.
    pub vector_norm: f32,
    /// Inner product between the binary reconstruction (`±1/√D` scaled) and the
    /// rotated normalized vector. Measures quantization quality; closer to 1.0 is better.
    pub quantization_ip: f32,
}

/// Binary-quantized vector with scalar correction factors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaBitQVector {
    /// Binary codes packed in u64 words. Length = `ceil(D / 64)`.
    pub bits: Vec<u64>,
    /// Affine correction factors used to recover an accurate distance estimate.
    pub correction: RaBitQCorrection,
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
/// The output length is `ceil(dim / 64)`. Padding bits in the last word are
/// always zero (values beyond `dim` are not written), which is required by
/// `xor_popcount_ip` for correct padding adjustment.
#[must_use]
pub(crate) fn signs_to_bits(values: &[f32], dim: usize) -> Vec<u64> {
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
///
/// F-12: Uses SIMD dot product per row instead of scalar iterator chain.
/// For dim=768, this is ~8x faster (SIMD dot product vs scalar sum).
#[must_use]
pub(crate) fn apply_rotation_flat(rotation: &[f32], vector: &[f32], dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let row_start = i * dim;
            crate::simd_native::dot_product_native(&rotation[row_start..row_start + dim], vector)
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
    // Adjust for padding bits in last word: padding bits are 0 in both vectors
    // (signs_to_bits only writes bits up to `dim`, zeroing the rest), so they
    // count as matching but shouldn't contribute.
    let padding_bits = num_words * 64 - dim;
    // Invariant: padding_bits <= num_words * 64 - dim.
    // matching_bits counts matching bits in `num_words * 64` positions. The
    // minimum number of matching bits equals the number of padding bits (since
    // padding bits are 0 in both vectors and thus always match). Therefore
    // matching_bits >= padding_bits is a guaranteed invariant.
    // `padding_bits` is at most `num_words * 64 - 1 < 64 * usize::MAX / 64`, which
    // is always less than u32::MAX for any realistic dimension. The cast is safe.
    #[allow(clippy::cast_possible_truncation)]
    let padding_bits_u32 = padding_bits as u32;
    debug_assert!(
        matching_bits >= padding_bits_u32,
        "matching_bits ({matching_bits}) < padding_bits ({padding_bits}): \
         signs_to_bits must zero padding bits"
    );
    let matching_bits = matching_bits - padding_bits_u32;

    // Inner product estimate: each matching bit contributes +1/D,
    // each differing bit contributes -1/D.
    #[allow(clippy::cast_precision_loss)]
    let d_f = dim as f32;
    #[allow(clippy::cast_precision_loss)]
    let ip = (2.0f32.mul_add(matching_bits as f32, -d_f)) / d_f;
    ip
}

/// Preprocessed query data for `RaBitQ` distance computation.
///
/// RF-DEDUP: Shared between `distance` and `batch_distance` to eliminate
/// the repeated center-normalize-rotate-bitsign preprocessing pipeline.
struct PreparedQuery {
    /// Squared L2 norm of the centered query.
    norm_sq: f32,
    /// L2 norm of the centered query.
    norm: f32,
    /// Sign bits of the rotated normalized query.
    bits: Vec<u64>,
    /// Number of u64 words in the bit representation.
    num_words: usize,
}

impl RaBitQIndex {
    /// Centers, normalizes, rotates, and extracts sign bits from a vector.
    ///
    /// Returns `None` when the centered vector has near-zero norm.
    fn prepare_query(&self, vector: &[f32]) -> Option<PreparedQuery> {
        let centered: Vec<f32> = vector
            .iter()
            .zip(self.centroid.iter())
            .map(|(&v, &c)| v - c)
            .collect();

        let norm_sq: f32 = centered.iter().map(|&x| x * x).sum();
        let norm = norm_sq.sqrt();

        if norm < f32::EPSILON {
            return None;
        }

        let normalized: Vec<f32> = centered.iter().map(|&x| x / norm).collect();
        let rotated = apply_rotation_flat(&self.rotation, &normalized, self.dimension);
        let bits = signs_to_bits(&rotated, self.dimension);
        let num_words = self.dimension.div_ceil(64);

        Some(PreparedQuery {
            norm_sq,
            norm,
            bits,
            num_words,
        })
    }

    /// Computes L2 distance from a prepared query to an encoded vector.
    fn distance_from_prepared(&self, pq: &PreparedQuery, encoded: &RaBitQVector) -> f32 {
        let ip_binary = xor_popcount_ip(&pq.bits, &encoded.bits, pq.num_words, self.dimension);

        let v_norm = encoded.correction.vector_norm;
        let estimated_ip = pq.norm * v_norm * ip_binary;
        let l2_sq = v_norm.mul_add(v_norm, pq.norm_sq) - 2.0 * estimated_ip;
        l2_sq.max(0.0).sqrt()
    }

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

        let Some(pq) = self.prepare_query(vector) else {
            let num_words = self.dimension.div_ceil(64);
            return Ok(RaBitQVector {
                bits: vec![0u64; num_words],
                correction: RaBitQCorrection {
                    vector_norm: 0.0,
                    quantization_ip: 1.0,
                },
            });
        };

        // Recompute the rotated normalized vector for correction factor calculation.
        // prepare_query already does this work but only returns bits. We need the
        // full rotated values for the quantization inner product computation.
        let centered: Vec<f32> = vector
            .iter()
            .zip(self.centroid.iter())
            .map(|(&v, &c)| v - c)
            .collect();
        let normalized: Vec<f32> = centered.iter().map(|&x| x / pq.norm).collect();
        let rotated = apply_rotation_flat(&self.rotation, &normalized, self.dimension);

        // Compute correction factors
        // The binary reconstruction maps each sign bit to +1/-1, scaled by 1/sqrt(D).
        // quantization_inner_product = <binary_reconstruction, rotated_normalized>
        #[allow(clippy::cast_precision_loss)]
        let scale = 1.0 / (self.dimension as f32).sqrt();
        let mut qip: f32 = 0.0;
        for (i, &rv) in rotated.iter().enumerate().take(self.dimension) {
            let word = i / 64;
            let bit = i % 64;
            let sign = if (pq.bits[word] >> bit) & 1 == 1 {
                1.0
            } else {
                -1.0
            };
            qip = (sign * scale).mul_add(rv, qip);
        }

        Ok(RaBitQVector {
            bits: pq.bits,
            correction: RaBitQCorrection {
                vector_norm: pq.norm,
                quantization_ip: qip,
            },
        })
    }

    /// Estimate the L2 distance between a raw query vector and an encoded vector.
    ///
    /// Uses XOR + popcount for fast Hamming-based inner product estimation,
    /// then applies affine correction with stored norms.
    #[must_use]
    pub fn distance(&self, query: &[f32], encoded: &RaBitQVector) -> f32 {
        let Some(pq) = self.prepare_query(query) else {
            // Query is at centroid; distance = norm of encoded vector
            return encoded.correction.vector_norm;
        };
        self.distance_from_prepared(&pq, encoded)
    }

    /// Batch distance: process query once, then iterate over encoded vectors.
    ///
    /// Amortizes query preprocessing (centering, normalization, rotation, sign extraction).
    #[must_use]
    pub fn batch_distance(&self, query: &[f32], encoded: &[RaBitQVector]) -> Vec<f32> {
        let Some(pq) = self.prepare_query(query) else {
            return encoded.iter().map(|e| e.correction.vector_norm).collect();
        };

        encoded
            .iter()
            .map(|ev| self.distance_from_prepared(&pq, ev))
            .collect()
    }
}

/// Training and persistence methods (require `persistence` feature for ndarray and rayon).
#[cfg(feature = "persistence")]
impl RaBitQIndex {
    /// Train a `RaBitQ` index from a set of vectors.
    ///
    /// Computes dataset centroid and generates a random orthogonal rotation
    /// matrix via modified Gram-Schmidt orthogonalization of a random matrix.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidQuantizerConfig` if:
    /// - `vectors` is empty
    /// - vectors have inconsistent dimensions
    /// - vector dimension is 0
    pub fn train(vectors: &[Vec<f32>], seed: u64) -> Result<Self, Error> {
        if vectors.is_empty() {
            return Err(Error::InvalidQuantizerConfig(
                "cannot train RaBitQ with empty dataset".into(),
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

        // Compute centroid (element-wise mean)
        let mut centroid = vec![0.0f32; dimension];
        for v in vectors {
            for (ci, &vi) in centroid.iter_mut().zip(v.iter()) {
                *ci += vi;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let inv_n = 1.0 / vectors.len() as f32;
        for x in &mut centroid {
            *x *= inv_n;
        }

        // Generate random orthogonal matrix via modified Gram-Schmidt
        let rotation = generate_orthogonal_matrix(dimension, seed);

        Ok(Self {
            rotation,
            centroid,
            dimension,
        })
    }

    /// Save `RaBitQ` index to `<dir>/rabitq.idx` using postcard with atomic write.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if serialization or file I/O fails.
    pub fn save(&self, dir: &std::path::Path) -> Result<(), Error> {
        let data = postcard::to_allocvec(self).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize RaBitQ index: {e}"),
            ))
        })?;
        let tmp_path = dir.join("rabitq.idx.tmp");
        let final_path = dir.join("rabitq.idx");
        std::fs::write(&tmp_path, &data)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Load `RaBitQ` index from `<dir>/rabitq.idx`. Returns `None` if file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if deserialization or file I/O fails.
    pub fn load(dir: &std::path::Path) -> Result<Option<Self>, Error> {
        let path = dir.join("rabitq.idx");
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read(&path)?;
        let index: Self = postcard::from_bytes(&data).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to deserialize RaBitQ index: {e}"),
            ))
        })?;
        Ok(Some(index))
    }
}

/// Generate a random orthogonal matrix using modified Gram-Schmidt.
///
/// Creates a D x D random matrix from a seeded RNG, then orthogonalizes it
/// using modified Gram-Schmidt (numerically stable for D <= 2048).
/// Returns the matrix flattened in row-major order.
///
/// # Complexity
///
/// Time complexity: O(d³) for Modified Gram-Schmidt on a d×d matrix.
/// For d=1024 this is ~10⁹ f64 operations. Only called during training.
///
/// # Numerical stability
///
/// MGS produces near-orthogonal matrices for D up to ~1024 at f32 precision.
/// For D > 1024, accumulated rounding errors can make the result noticeably
/// non-orthogonal (‖Rᵀ R − I‖_F may exceed 1e-3). If higher-dimensional
/// rotations are required, consider Householder QR or double-precision MGS.
#[cfg(feature = "persistence")]
fn generate_orthogonal_matrix(dim: usize, seed: u64) -> Vec<f32> {
    use rand::{Rng, SeedableRng};

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    // Generate random D x D matrix (column-major for easier Gram-Schmidt)
    // columns[j][i] = element at row i, column j
    let mut columns: Vec<Vec<f32>> = (0..dim)
        .map(|_| (0..dim).map(|_| rng.gen::<f32>() * 2.0 - 1.0).collect())
        .collect();

    // Modified Gram-Schmidt orthogonalization
    for j in 0..dim {
        // Normalize column j
        let norm: f32 = columns[j].iter().map(|&x| x * x).sum::<f32>().sqrt();
        if norm > f32::EPSILON {
            for x in &mut columns[j] {
                *x /= norm;
            }
        }

        // Subtract projection of remaining columns onto column j
        for k in (j + 1)..dim {
            let dot: f32 = columns[j]
                .iter()
                .zip(columns[k].iter())
                .map(|(&a, &b)| a * b)
                .sum();
            let proj: Vec<f32> = columns[j].iter().map(|&x| dot * x).collect();
            for (ck, p) in columns[k].iter_mut().zip(proj.iter()) {
                *ck -= p;
            }
        }
    }

    // Convert column-major to row-major flattened format
    let mut rotation = vec![0.0f32; dim * dim];
    for i in 0..dim {
        for j in 0..dim {
            rotation[i * dim + j] = columns[j][i];
        }
    }

    rotation
}

#[cfg(test)]
#[path = "rabitq_tests.rs"]
mod tests;
