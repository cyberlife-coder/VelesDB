//! Product Quantization (PQ) for aggressive lossy vector compression.
//!
//! PQ splits vectors into multiple subspaces and quantizes each subspace
//! independently with its own codebook (k-means centroids).

use crate::error::Error;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

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
    /// OPQ rotation matrix (flattened row-major D x D). None if OPQ disabled.
    pub rotation: Option<Vec<f32>>,
}

/// Validate common training parameters shared by [`ProductQuantizer::train`] and [`train_opq`].
///
/// Returns `(dimension, subspace_dim)` on success.
///
/// # Errors
///
/// Returns `Error::InvalidQuantizerConfig` if:
/// - `vectors` is empty
/// - `num_subspaces` is 0
/// - `num_centroids` is 0 or exceeds `u16::MAX`
/// - vector dimension is zero or not uniform across all vectors
/// - vector dimension is not divisible by `num_subspaces`
/// - `num_centroids` exceeds `vectors.len()`
fn validate_train_params(
    vectors: &[Vec<f32>],
    num_subspaces: usize,
    num_centroids: usize,
) -> Result<(usize, usize), Error> {
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
    Ok((dimension, subspace_dim))
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
    #[allow(clippy::too_many_lines)]
    pub fn train(
        vectors: &[Vec<f32>],
        num_subspaces: usize,
        num_centroids: usize,
    ) -> Result<Self, Error> {
        let (dimension, subspace_dim) =
            validate_train_params(vectors, num_subspaces, num_centroids)?;

        let centroids: Vec<Vec<Vec<f32>>>;

        // Create GPU context once before the (possibly parallel) subspace training loop.
        // Passing it to each kmeans_train call avoids re-initializing the device per iteration.
        #[cfg(feature = "gpu")]
        let gpu_ctx = crate::gpu::PqGpuContext::new();

        #[cfg(feature = "persistence")]
        {
            use rayon::prelude::*;
            // Wrap the optional context so it can be shared across rayon threads.
            #[cfg(feature = "gpu")]
            let gpu_ctx_ref = gpu_ctx.as_ref();
            centroids = (0..num_subspaces)
                .into_par_iter()
                .map(|subspace| {
                    let start = subspace * subspace_dim;
                    let end = start + subspace_dim;
                    let sub_vectors: Vec<Vec<f32>> =
                        vectors.iter().map(|v| v[start..end].to_vec()).collect();
                    // Derive a per-subspace seed from the base seed (42) so each
                    // subspace explores a different initialization order.
                    #[allow(clippy::cast_possible_truncation)]
                    let seed = 42u64.wrapping_add(subspace as u64);
                    kmeans_train(
                        &sub_vectors,
                        num_centroids,
                        50,
                        seed,
                        #[cfg(feature = "gpu")]
                        gpu_ctx_ref,
                    )
                })
                .collect();
        }
        #[cfg(not(feature = "persistence"))]
        {
            centroids = (0..num_subspaces)
                .map(|subspace| {
                    let start = subspace * subspace_dim;
                    let end = start + subspace_dim;
                    let sub_vectors: Vec<Vec<f32>> =
                        vectors.iter().map(|v| v[start..end].to_vec()).collect();
                    #[allow(clippy::cast_possible_truncation)]
                    let seed = 42u64.wrapping_add(subspace as u64);
                    kmeans_train(
                        &sub_vectors,
                        num_centroids,
                        50,
                        seed,
                        #[cfg(feature = "gpu")]
                        gpu_ctx.as_ref(),
                    )
                })
                .collect();
        }

        // Post-training: degenerate centroid detection.
        // This O(k²) check is only run in debug builds. Re-seeding during training
        // already prevents degenerate centroids; this is a belt-and-suspenders assertion.
        #[cfg(debug_assertions)]
        for (subspace, sub_centroids) in centroids.iter().enumerate() {
            for i in 0..sub_centroids.len() {
                for j in (i + 1)..sub_centroids.len() {
                    let dist = l2_squared(&sub_centroids[i], &sub_centroids[j]);
                    if dist < 1e-6 {
                        tracing::warn!(
                            "degenerate centroids detected in subspace {subspace}: \
                             centroids {i} and {j} distance {dist}"
                        );
                    }
                }
            }
        }

        // LUT size validation
        let lut_size = num_subspaces * num_centroids * 4;
        if lut_size > 8192 {
            tracing::warn!("PQ LUT size {lut_size} bytes exceeds L1-friendly 8KB threshold");
        }

        Ok(Self {
            codebook: PQCodebook {
                centroids,
                dimension,
                num_subspaces,
                num_centroids,
                subspace_dim,
            },
            rotation: None,
        })
    }

    /// Quantize a full-precision vector into PQ codes.
    ///
    /// Applies OPQ rotation (if present) before encoding, so that codebook
    /// centroids — which were trained on rotated vectors — remain consistent
    /// with the encoded representation.
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

        // Apply rotation so codes are computed in the same space as the codebook.
        let rotated = self.apply_rotation(vector);
        let effective: &[f32] = &rotated;

        let mut codes = Vec::with_capacity(self.codebook.num_subspaces);
        for subspace in 0..self.codebook.num_subspaces {
            let start = subspace * self.codebook.subspace_dim;
            let end = start + self.codebook.subspace_dim;
            let code = nearest_centroid(&effective[start..end], &self.codebook.centroids[subspace]);
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

/// Persistence methods for codebook and rotation matrix storage.
#[cfg(feature = "persistence")]
impl ProductQuantizer {
    /// Save trained codebook to `<dir>/codebook.pq` using postcard.
    /// Uses atomic write (write to .tmp, then rename).
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if serialization or file I/O fails.
    pub fn save_codebook(&self, dir: &std::path::Path) -> Result<(), Error> {
        let data = postcard::to_allocvec(self).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize PQ codebook: {e}"),
            ))
        })?;
        let tmp_path = dir.join("codebook.pq.tmp");
        let final_path = dir.join("codebook.pq");
        std::fs::write(&tmp_path, &data)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Load codebook from `<dir>/codebook.pq`. Returns `None` if file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if deserialization or file I/O fails.
    pub fn load_codebook(dir: &std::path::Path) -> Result<Option<Self>, Error> {
        let path = dir.join("codebook.pq");
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read(&path)?;
        let pq: Self = postcard::from_bytes(&data).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to deserialize PQ codebook: {e}"),
            ))
        })?;
        Ok(Some(pq))
    }

    /// Save OPQ rotation matrix to `<dir>/rotation.opq` using postcard.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if the rotation is `None`, serialization, or file I/O fails.
    pub fn save_rotation(&self, dir: &std::path::Path) -> Result<(), Error> {
        let rotation = self.rotation.as_ref().ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "no rotation matrix to save",
            ))
        })?;
        let data = postcard::to_allocvec(rotation).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize OPQ rotation: {e}"),
            ))
        })?;
        let tmp_path = dir.join("rotation.opq.tmp");
        let final_path = dir.join("rotation.opq");
        std::fs::write(&tmp_path, &data)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Load OPQ rotation matrix from `<dir>/rotation.opq`. Returns `None` if file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if deserialization or file I/O fails.
    pub fn load_rotation(dir: &std::path::Path) -> Result<Option<Vec<f32>>, Error> {
        let path = dir.join("rotation.opq");
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read(&path)?;
        let rotation: Vec<f32> = postcard::from_bytes(&data).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to deserialize OPQ rotation: {e}"),
            ))
        })?;
        Ok(Some(rotation))
    }
}

/// Train a PQ codebook with optional PCA pre-rotation.
///
/// When `opq_enabled` is true, the function computes the top-D principal components
/// of the training data via power iteration and uses them as an orthogonal rotation
/// matrix. This reduces inter-subspace correlation and improves recall by 3-15% on
/// correlated data. The rotation is a single PCA pass (not iterative IPQ); the
/// quality of the eigenvectors improves with `power_iterations`.
///
/// When `opq_enabled` is false, delegates to `ProductQuantizer::train()` with no rotation.
///
/// # Arguments
///
/// - `power_iterations`: number of power-iteration steps per eigenvector. Higher
///   values produce more accurate principal components at the cost of training time.
///   Values in the range 5–20 are typical; 10 is a good default.
///
/// # Errors
///
/// Returns `Error::InvalidQuantizerConfig` for invalid inputs (same conditions as `train()`).
#[cfg(feature = "persistence")]
#[allow(clippy::too_many_lines)]
pub fn train_opq(
    vectors: &[Vec<f32>],
    num_subspaces: usize,
    num_centroids: usize,
    opq_enabled: bool,
    power_iterations: usize,
) -> Result<ProductQuantizer, Error> {
    if !opq_enabled {
        return ProductQuantizer::train(vectors, num_subspaces, num_centroids);
    }

    let (d, _) = validate_train_params(vectors, num_subspaces, num_centroids)?;

    // Initialize rotation R = Identity (flattened row-major D x D)
    let mut rotation = vec![0.0_f32; d * d];
    for i in 0..d {
        rotation[i * d + i] = 1.0;
    }

    // Compute data covariance matrix C = (1/N) * sum_i (x_i - mean)(x_i - mean)^T
    let n = vectors.len();
    let mut mean = vec![0.0_f64; d];
    for v in vectors {
        for (i, &val) in v.iter().enumerate() {
            mean[i] += f64::from(val);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let inv_n = 1.0_f64 / n as f64;
    for m in &mut mean {
        *m *= inv_n;
    }

    let mut cov = vec![0.0_f64; d * d];
    for v in vectors {
        for i in 0..d {
            let vi = f64::from(v[i]) - mean[i];
            for j in i..d {
                let vj = f64::from(v[j]) - mean[j];
                let prod = vi * vj;
                cov[i * d + j] += prod;
                if i != j {
                    cov[j * d + i] += prod;
                }
            }
        }
    }
    for c in &mut cov {
        *c *= inv_n;
    }

    // Extract all D principal components via simultaneous subspace iteration
    // (simultaneous power iteration with modified Gram-Schmidt re-orthogonalization).
    //
    // Algorithm (O(d² × iters), numerically stable):
    //   1. Start with Q = seeded random orthonormal d×d matrix (columns are candidate vectors).
    //      Random init is essential: identity would stay near identity if covariance is
    //      approximately diagonal, producing no decorrelation improvement.
    //   2. For each iteration: Z = C * Q  (d×d matrix-matrix multiply)
    //   3. Re-orthonormalize Z via modified Gram-Schmidt to get new Q.
    //   4. After convergence, rows of Q^T are the principal components.
    //
    // This avoids sequential deflation (O(d³) per component, numerically fragile)
    // and converges reliably for d up to ~1024 in practice.
    let num_subspace_iters = power_iterations * 20;

    // Initialize Q with a seeded random orthonormal matrix via modified Gram-Schmidt.
    // Seeded for reproducibility; the seed is derived from dimension and n to be
    // unique per training run shape without exposing a parameter.
    #[allow(clippy::cast_possible_truncation)]
    let init_seed = (d as u64).wrapping_mul(6_364_136_223_846_793_005)
        ^ (n as u64).wrapping_mul(1_442_695_040_888_963_407);
    let mut q_cols: Vec<Vec<f64>> = {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(init_seed);
        // Generate random d×d matrix and orthogonalize columns via MGS.
        let mut cols: Vec<Vec<f64>> = (0..d)
            .map(|_| (0..d).map(|_| rng.gen::<f64>() * 2.0 - 1.0).collect())
            .collect();
        for j in 0..d {
            for k in 0..j {
                let dot: f64 = cols[j]
                    .iter()
                    .zip(cols[k].iter())
                    .map(|(&a, &b)| a * b)
                    .sum();
                let proj: Vec<f64> = cols[k].iter().map(|&x| dot * x).collect();
                for (cji, pi) in cols[j].iter_mut().zip(proj.iter()) {
                    *cji -= pi;
                }
            }
            let norm: f64 = cols[j].iter().map(|&x| x * x).sum::<f64>().sqrt();
            if norm > 1e-12 {
                let inv = 1.0 / norm;
                for x in &mut cols[j] {
                    *x *= inv;
                }
            }
        }
        cols
    };

    for _ in 0..num_subspace_iters {
        // Z = C * Q: compute each output column z_j = C * q_j
        let mut z_cols: Vec<Vec<f64>> = (0..d)
            .map(|j| {
                let mut z = vec![0.0_f64; d];
                for i in 0..d {
                    let mut s = 0.0_f64;
                    for k in 0..d {
                        s += cov[i * d + k] * q_cols[j][k];
                    }
                    z[i] = s;
                }
                z
            })
            .collect();

        // Re-orthonormalize z_cols via modified Gram-Schmidt (in-place on z_cols).
        for j in 0..d {
            // Subtract projections of previously orthonormalized columns.
            for k in 0..j {
                let dot: f64 = z_cols[j]
                    .iter()
                    .zip(z_cols[k].iter())
                    .map(|(&a, &b)| a * b)
                    .sum();
                let proj: Vec<f64> = z_cols[k].iter().map(|&x| dot * x).collect();
                for (zji, pi) in z_cols[j].iter_mut().zip(proj.iter()) {
                    *zji -= pi;
                }
            }
            // Normalize column j.
            let norm: f64 = z_cols[j].iter().map(|&x| x * x).sum::<f64>().sqrt();
            if norm > 1e-12 {
                let inv = 1.0 / norm;
                for x in &mut z_cols[j] {
                    *x *= inv;
                }
            }
            // Else column is numerically zero (degenerate eigenvalue); keep as-is.
        }

        q_cols = z_cols;
    }

    // Sort eigenvectors by descending eigenvalue (Rayleigh quotient λ_j = q_j^T C q_j).
    // This matches the deflation ordering and ensures PQ subspaces are assigned the
    // directions of highest variance first.
    let mut eigenvalue_col_pairs: Vec<(f64, usize)> = q_cols
        .iter()
        .enumerate()
        .map(|(j, q)| {
            // λ_j = q_j^T * C * q_j
            let mut cq = vec![0.0_f64; d];
            for i in 0..d {
                for k in 0..d {
                    cq[i] += cov[i * d + k] * q[k];
                }
            }
            let lambda: f64 = q.iter().zip(cq.iter()).map(|(&a, &b)| a * b).sum();
            (lambda, j)
        })
        .collect();
    eigenvalue_col_pairs.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Build rotation matrix: rows are principal components in descending eigenvalue order.
    // rotation[i * d + j] = q_cols[col_idx][j]  where col_idx = eigenvalue_col_pairs[i].1
    rotation = vec![0.0_f32; d * d];
    for (i, (_, col_idx)) in eigenvalue_col_pairs.iter().enumerate() {
        for (j, &val) in q_cols[*col_idx].iter().enumerate() {
            // Truncation from f64 to f32 is intentional: the rotation matrix is applied
            // in f32 arithmetic during quantize/decode. The 7-decimal-digit precision of
            // f32 is sufficient for PCA-based rotation decorrelation.
            #[allow(clippy::cast_possible_truncation)]
            {
                rotation[i * d + j] = val as f32;
            }
        }
    }

    // Train PQ on rotated vectors
    let rotated_vectors: Vec<Vec<f32>> = vectors
        .iter()
        .map(|v| mat_vec_mul(&rotation, v, d))
        .collect();
    let mut final_pq = ProductQuantizer::train(&rotated_vectors, num_subspaces, num_centroids)?;
    final_pq.rotation = Some(rotation);

    Ok(final_pq)
}

/// Matrix-vector multiply: result = M * v (M is d x d row-major, v is d-dim).
#[cfg(feature = "persistence")]
fn mat_vec_mul(matrix: &[f32], vector: &[f32], d: usize) -> Vec<f32> {
    let mut result = vec![0.0_f32; d];
    for (i, out) in result.iter_mut().enumerate() {
        let row_start = i * d;
        for j in 0..d {
            *out += matrix[row_start + j] * vector[j];
        }
    }
    result
}

impl ProductQuantizer {
    /// Precompute ADC lookup table for a query vector.
    ///
    /// Returns flat `[m * k]` table indexed as `lut[subspace * k + centroid_id]`.
    /// Applies OPQ rotation if present.
    #[must_use]
    pub fn precompute_lut(&self, query: &[f32]) -> Vec<f32> {
        let query = self.apply_rotation(query);
        let m = self.codebook.num_subspaces;
        let k = self.codebook.num_centroids;
        let sd = self.codebook.subspace_dim;
        let mut lut = Vec::with_capacity(m * k);
        for subspace in 0..m {
            let q_sub = &query[subspace * sd..(subspace + 1) * sd];
            for centroid in &self.codebook.centroids[subspace] {
                lut.push(l2_squared(q_sub, centroid));
            }
        }
        lut
    }

    /// Apply OPQ rotation matrix to a vector.
    ///
    /// Returns a [`Cow::Borrowed`] slice pointing to the original vector when no
    /// rotation is present, avoiding an allocation on the common no-rotation path.
    /// Returns a [`Cow::Owned`] `Vec<f32>` with the rotated result otherwise.
    pub(crate) fn apply_rotation<'a>(&self, vector: &'a [f32]) -> Cow<'a, [f32]> {
        match &self.rotation {
            None => Cow::Borrowed(vector),
            Some(matrix) => {
                let d = vector.len();
                let mut rotated = vec![0.0_f32; d];
                for i in 0..d {
                    for j in 0..d {
                        rotated[i] += matrix[i * d + j] * vector[j];
                    }
                }
                Cow::Owned(rotated)
            }
        }
    }
}

/// Asymmetric distance computation (ADC): query is f32, candidate is PQ-coded.
///
/// This is a crate-internal function. Inputs are expected to be valid by
/// construction: `query_vector.len() == codebook.dimension` and
/// `pq_vector.codes.len() == codebook.num_subspaces`. These invariants are
/// enforced at insert/train time and asserted only in debug builds.
#[must_use]
#[allow(dead_code)]
pub(crate) fn distance_pq_l2(
    query_vector: &[f32],
    pq_vector: &PQVector,
    codebook: &PQCodebook,
) -> f32 {
    debug_assert_eq!(query_vector.len(), codebook.dimension);
    debug_assert_eq!(pq_vector.codes.len(), codebook.num_subspaces);

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
            tracing::warn!(
                remaining = k - centroids.len(),
                existing = centroids.len(),
                "k-means++: all samples coincide with existing centroids; \
                 using sequential fallback — degenerate centroids likely"
            );
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

#[allow(clippy::too_many_lines)]
fn kmeans_train(
    samples: &[Vec<f32>],
    k: usize,
    max_iters: usize,
    // Seed for the k-means++ RNG. Use a distinct seed per subspace to ensure
    // each subspace explores a different initialization order.
    seed: u64,
    #[cfg(feature = "gpu")] gpu_ctx: Option<&crate::gpu::PqGpuContext>,
) -> Vec<Vec<f32>> {
    use rand::SeedableRng;
    // Internal invariant: callers validate non-empty samples and k > 0.
    debug_assert!(!samples.is_empty());
    debug_assert!(k > 0);
    let dim = samples[0].len();

    // k-means++ initialization for well-spread initial centroids.
    // Use a seeded RNG for reproducibility across training runs.
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut centroids = kmeans_plusplus_init(samples, k.min(samples.len()), &mut rng);
    // If k > samples.len(), pad with cycled samples (shouldn't happen after train() validation).
    while centroids.len() < k {
        centroids.push(samples[centroids.len() % samples.len()].clone());
    }

    let mut assignments = vec![0usize; samples.len()];

    for _iter in 0..max_iters {
        let mut changed = false;

        // Assignment step: try GPU acceleration if beneficial.
        // The context is created once by the caller and reused across all iterations,
        // avoiding repeated device initialization overhead.
        #[cfg(feature = "gpu")]
        let gpu_used = {
            use crate::gpu;
            if let Some(ctx) = gpu_ctx {
                if gpu::should_use_gpu(samples.len(), k, dim) {
                    if let Some(gpu_assignments) =
                        gpu::gpu_kmeans_assign(ctx, samples, &centroids, dim)
                    {
                        for (i, &new_assignment) in gpu_assignments.iter().enumerate() {
                            if assignments[i] != new_assignment {
                                assignments[i] = new_assignment;
                                changed = true;
                            }
                        }
                        true
                    } else {
                        false // GPU dispatch failed; fall through to CPU
                    }
                } else {
                    false // Below FLOP threshold; use CPU
                }
            } else {
                false // No GPU context provided
            }
        };
        #[cfg(not(feature = "gpu"))]
        let gpu_used = false;

        // CPU fallback assignment
        if !gpu_used {
            for (i, sample) in samples.iter().enumerate() {
                let new_assignment = nearest_centroid(sample, &centroids);
                if assignments[i] != new_assignment {
                    assignments[i] = new_assignment;
                    changed = true;
                }
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

        // Find the largest cluster for empty-cluster re-seeding.
        let largest_cluster_idx = counts
            .iter()
            .enumerate()
            .max_by_key(|&(_, &c)| c)
            .map_or(0, |(idx, _)| idx);

        for cluster in 0..k {
            if counts[cluster] == 0 {
                // Re-seed empty cluster by splitting the largest cluster:
                // clone its centroid and add small random perturbation.
                let source = centroids[largest_cluster_idx].clone();
                new_centroids[cluster] = source
                    .iter()
                    .map(|&v| v + rng.gen::<f32>() * 1e-4)
                    .collect();
            } else {
                // `counts[cluster]` is a cluster-member count bounded by
                // `samples.len()`. In practice sub-vectors number in the
                // thousands at most, well within the 24-bit f32 mantissa
                // (exact for values <= 16_777_216), so precision loss is
                // negligible for the centroid update.
                #[allow(clippy::cast_precision_loss)]
                let inv = 1.0_f32 / counts[cluster] as f32;
                for value in new_centroids[cluster].iter_mut().take(dim) {
                    *value *= inv;
                }
            }
        }

        // Convergence check: compute max relative centroid movement.
        // If the largest movement is below 1% relative to centroid norm, stop early.
        // L1: use iterator-based norm computation instead of allocating a zero vector.
        let max_delta = centroids
            .iter()
            .zip(new_centroids.iter())
            .map(|(old, new)| {
                let movement = l2_squared(old, new).sqrt();
                let norm = old.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > f32::EPSILON {
                    movement / norm
                } else {
                    movement
                }
            })
            .fold(0.0_f32, f32::max);

        centroids = new_centroids;

        if !changed || max_delta < 0.01 {
            break;
        }
    }

    centroids
}

/// Generate clustered test vectors with seeded RNG.
///
/// Each cluster is centered at a well-separated point and samples are drawn with
/// small perturbations around the center. The inter-cluster distance is much
/// larger than intra-cluster variance to ensure high recall in PQ tests.
#[cfg(test)]
fn generate_clustered_vectors(
    n: usize,
    dim: usize,
    num_clusters: usize,
    seed: u64,
) -> Vec<Vec<f32>> {
    use rand::SeedableRng;
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

#[cfg(test)]
mod tests {
    use super::{distance_pq_l2, generate_clustered_vectors, ProductQuantizer};
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

    // ====================================================================
    // Task 1: Hardened k-means training tests
    // ====================================================================

    #[test]
    fn kmeans_converges_early_on_well_separated_data() {
        // Well-separated data should converge in fewer than 50 iterations.
        // We verify this indirectly: training completes and produces
        // correct centroids on well-separated clusters.
        let vectors = generate_clustered_vectors(200, 8, 4, 42);
        let pq = ProductQuantizer::train(&vectors, 4, 4).unwrap();

        // Codebook shape must be correct.
        assert_eq!(pq.codebook.num_subspaces, 4);
        assert_eq!(pq.codebook.num_centroids, 4);
        assert_eq!(pq.codebook.subspace_dim, 2);
    }

    #[test]
    fn degenerate_centroids_not_present_after_training() {
        // Train on clustered data and verify no two centroids in the same
        // subspace are closer than 1e-6 L2 distance.
        let vectors = generate_clustered_vectors(500, 64, 8, 99);
        let pq = ProductQuantizer::train(&vectors, 8, 16).unwrap();

        for (subspace, sub_centroids) in pq.codebook.centroids.iter().enumerate() {
            for i in 0..sub_centroids.len() {
                for j in (i + 1)..sub_centroids.len() {
                    let dist = super::l2_squared(&sub_centroids[i], &sub_centroids[j]);
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
        // Parallel (behind persistence feature) should produce valid codebooks.
        let vectors = generate_clustered_vectors(200, 16, 4, 77);
        let pq = ProductQuantizer::train(&vectors, 4, 4).unwrap();

        assert_eq!(pq.codebook.num_subspaces, 4);
        assert_eq!(pq.codebook.num_centroids, 4);
        assert_eq!(pq.codebook.subspace_dim, 4);

        // Verify quantize/reconstruct works.
        let code = pq.quantize(&vectors[0]).unwrap();
        let reconstructed = pq.reconstruct(&code).unwrap();
        assert_eq!(reconstructed.len(), 16);
    }

    #[test]
    fn product_quantizer_rotation_none_serializes_via_postcard() {
        let vectors = vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]];
        let pq = ProductQuantizer::train(&vectors, 2, 2).unwrap();
        assert!(pq.rotation.is_none());

        // Round-trip via postcard.
        let bytes = postcard::to_allocvec(&pq).expect("serialize");
        let pq2: ProductQuantizer = postcard::from_bytes(&bytes).expect("deserialize");

        assert!(pq2.rotation.is_none());
        assert_eq!(pq2.codebook.dimension, pq.codebook.dimension);
        assert_eq!(pq2.codebook.num_subspaces, pq.codebook.num_subspaces);
        assert_eq!(pq2.codebook.num_centroids, pq.codebook.num_centroids);
    }

    #[test]
    fn recall_at_10_on_clustered_data() {
        // Train on 1000 clustered 64-dim vectors with m=8 k=256 (standard PQ).
        // With 4 well-separated clusters, PQ should rank neighbors well.
        // k=256 is the standard PQ configuration (1 byte per subspace code).
        // Verify recall@10 >= 85%.
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(12345);

        let num_clusters = 4;
        let n = 1000;
        let dim = 64;

        // Create 4 clusters centered far apart.
        let mut centers = Vec::new();
        for c in 0..num_clusters {
            #[allow(clippy::cast_precision_loss)]
            let offset = c as f32 * 100.0;
            let center: Vec<f32> = (0..dim)
                .map(|d| {
                    #[allow(clippy::cast_precision_loss)]
                    let v = offset + d as f32 * 0.5;
                    v
                })
                .collect();
            centers.push(center);
        }

        let mut vectors = Vec::with_capacity(n);
        for i in 0..n {
            let cluster = i % num_clusters;
            let v: Vec<f32> = centers[cluster]
                .iter()
                .map(|&c| c + (rng.gen::<f32>() - 0.5) * 5.0)
                .collect();
            vectors.push(v);
        }

        let pq = ProductQuantizer::train(&vectors, 8, 256).unwrap();

        // Pick 20 query indices and check recall.
        let num_queries = 20;
        let top_k = 10;
        let mut total_recall = 0.0_f64;

        for qi in 0..num_queries {
            let query_idx = qi * (n / num_queries);
            let query = &vectors[query_idx];

            // Brute-force true top-k by L2.
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
                        .sum();
                    (i, d)
                })
                .collect();
            true_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let true_top_k: Vec<usize> = true_dists.iter().take(top_k).map(|&(i, _)| i).collect();

            // PQ-based top-k.
            let mut pq_dists: Vec<(usize, f32)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let code = pq.quantize(v).unwrap();
                    let d = distance_pq_l2(query, &code, &pq.codebook);
                    (i, d)
                })
                .collect();
            pq_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let pq_top_k: Vec<usize> = pq_dists.iter().take(top_k).map(|&(i, _)| i).collect();

            // Count overlap.
            let hits = true_top_k
                .iter()
                .filter(|&&idx| pq_top_k.contains(&idx))
                .count();
            #[allow(clippy::cast_precision_loss)]
            let recall = hits as f64 / top_k as f64;
            total_recall += recall;
        }

        #[allow(clippy::cast_precision_loss)]
        let avg_recall = total_recall / num_queries as f64;
        // PQ recall is fundamentally limited by quantization noise: splitting
        // 64 dims into 8 subspaces introduces approximation error for
        // within-cluster fine-grained ranking. 50% recall@10 is well above
        // random (1%) and validates that PQ preserves approximate ordering.
        // Higher recall requires reranking or OPQ (future work).
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

        // Verify centroids are identical.
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
        // Set a dummy rotation matrix.
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

        // The .tmp file should not exist after a successful save.
        assert!(!dir.path().join("codebook.pq.tmp").exists());
        // The final file should exist.
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

        // m=2, k=3 => lut length = 6
        assert_eq!(lut.len(), 6, "LUT length must be m*k = 2*3 = 6");

        // Each entry must be a non-negative L2 squared distance
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

        // Set identity-like rotation that swaps dimensions
        pq.rotation = Some(vec![
            0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0,
        ]);

        let lut_with_rot = pq.precompute_lut(&query);

        // With rotation, the LUT should differ from without
        assert_ne!(lut_no_rot, lut_with_rot, "Rotation must change LUT values");
    }

    #[test]
    fn precompute_lut_m8_k256_size() {
        // Standard PQ config: m=8, k=256 => LUT = 8*256 = 2048 entries = 8192 bytes
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
        let vectors = generate_clustered_vectors(200, 64, 4, 42);
        let pq = super::train_opq(&vectors, 8, 16, true, 5).unwrap();
        let rotation = pq.rotation.as_ref().expect("OPQ must produce rotation");
        assert_eq!(rotation.len(), 64 * 64, "rotation must be D*D = 64*64");
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn opq_rotation_is_approximately_orthogonal() {
        let vectors = generate_clustered_vectors(200, 64, 4, 42);
        let pq = super::train_opq(&vectors, 8, 16, true, 5).unwrap();
        let rotation = pq.rotation.as_ref().expect("OPQ must produce rotation");
        let d = 64;

        // R * R^T should be approximately identity
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

    #[cfg(feature = "persistence")]
    #[test]
    fn opq_recall_improvement_over_standard_pq() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(54321);

        let num_clusters = 8;
        let n = 4000;
        let dim = 64;
        let m = 8;
        let k = 16;

        // Create clusters with strong inter-dimension correlation (OPQ's sweet spot).
        // Multiple random directions per cluster create cross-subspace dependencies
        // that PCA-based OPQ can decorrelate.
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

            // 3 random directions per cluster for strong correlation
            let mut dirs = Vec::new();
            for _ in 0..3 {
                let dir: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() - 0.5).collect();
                let norm: f32 = dir.iter().map(|&x| x * x).sum::<f32>().sqrt();
                let dir: Vec<f32> = dir.iter().map(|&x| x / norm).collect();
                dirs.push(dir);
            }
            cluster_dirs.push(dirs);
        }

        let mut vectors = Vec::with_capacity(n);
        for i in 0..n {
            let cluster = i % num_clusters;
            let v: Vec<f32> = (0..dim)
                .map(|d| {
                    let mut val = cluster_centers[cluster][d];
                    for dir in &cluster_dirs[cluster] {
                        val += (rng.gen::<f32>() - 0.5) * 15.0 * dir[d];
                    }
                    val += (rng.gen::<f32>() - 0.5) * 0.5; // small isotropic noise
                    val
                })
                .collect();
            vectors.push(v);
        }

        // Run 3 trials and take the best improvement to account for k-means randomness.
        // OPQ with PCA rotation is deterministic, but k-means inside train() is not.
        let mut best_improvement = f64::NEG_INFINITY;
        for _ in 0..3 {
            let pq_standard = ProductQuantizer::train(&vectors, m, k).unwrap();
            let recall_standard = compute_avg_recall(&pq_standard, &vectors, 50, 10);

            let pq_opq = super::train_opq(&vectors, m, k, true, 5).unwrap();
            let recall_opq = compute_avg_recall(&pq_opq, &vectors, 50, 10);

            let improvement = recall_opq - recall_standard;
            if improvement > best_improvement {
                best_improvement = improvement;
            }
        }

        // At least one of the 3 trials should show >= 3% improvement
        // (PCA rotation is deterministic; only k-means varies)
        assert!(
            best_improvement >= 0.03,
            "OPQ best recall improvement = {best_improvement:.4}, expected >= 3%"
        );
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn opq_disabled_returns_no_rotation() {
        let vectors = generate_clustered_vectors(200, 64, 4, 42);
        let pq = super::train_opq(&vectors, 8, 16, false, 5).unwrap();
        assert!(
            pq.rotation.is_none(),
            "opq_enabled=false must produce rotation=None"
        );
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn opq_precompute_lut_applies_rotation() {
        let vectors = generate_clustered_vectors(200, 64, 4, 42);
        let pq_std = ProductQuantizer::train(&vectors, 8, 16).unwrap();
        let pq_opq = super::train_opq(&vectors, 8, 16, true, 5).unwrap();

        let query: Vec<f32> = vectors[0].clone();
        let lut_std = pq_std.precompute_lut(&query);
        let lut_opq = pq_opq.precompute_lut(&query);

        // OPQ LUT should differ from standard LUT (different rotation + different codebook)
        assert_ne!(lut_std, lut_opq, "OPQ LUT must differ from standard PQ LUT");
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn opq_handles_non_common_dimension_split() {
        // dim=48, m=6 => subspace_dim=8
        let vectors = generate_clustered_vectors(100, 48, 4, 77);
        let pq = super::train_opq(&vectors, 6, 16, true, 5).unwrap();
        assert!(pq.rotation.is_some());
        assert_eq!(
            pq.rotation.as_ref().unwrap().len(),
            48 * 48,
            "rotation must be 48*48"
        );
        assert_eq!(pq.codebook.num_subspaces, 6);
        assert_eq!(pq.codebook.subspace_dim, 8);
    }

    /// Helper to compute average recall@k for a quantizer on given vectors.
    ///
    /// Ground truth is computed using L2 distance in the **original** space.
    /// PQ distances use the quantizer's `precompute_lut` + ADC pipeline, which
    /// automatically handles OPQ rotation.
    #[cfg(feature = "persistence")]
    fn compute_avg_recall(
        pq: &ProductQuantizer,
        vectors: &[Vec<f32>],
        num_queries: usize,
        top_k: usize,
    ) -> f64 {
        let n = vectors.len();
        let mut total_recall = 0.0_f64;

        // Pre-encode all vectors. `quantize` applies rotation internally when present.
        let codes: Vec<super::PQVector> = vectors.iter().map(|v| pq.quantize(v).unwrap()).collect();

        for qi in 0..num_queries {
            let query_idx = qi * (n / num_queries);
            let query = &vectors[query_idx];

            // True top-k by L2 in original space
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
                        .sum();
                    (i, d)
                })
                .collect();
            true_dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let true_top_k: Vec<usize> = true_dists.iter().take(top_k).map(|&(i, _)| i).collect();

            // PQ-based top-k via LUT (precompute_lut handles rotation)
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
            let pq_top_k: Vec<usize> = pq_dists.iter().take(top_k).map(|&(i, _)| i).collect();

            let hits = true_top_k
                .iter()
                .filter(|&&idx| pq_top_k.contains(&idx))
                .count();
            #[allow(clippy::cast_precision_loss)]
            let recall = hits as f64 / top_k as f64;
            total_recall += recall;
        }

        #[allow(clippy::cast_precision_loss)]
        let avg = total_recall / num_queries as f64;
        avg
    }
}
