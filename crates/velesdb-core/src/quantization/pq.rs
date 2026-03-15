//! Product Quantization (PQ) for aggressive lossy vector compression.
//!
//! PQ splits vectors into multiple subspaces and quantizes each subspace
//! independently with its own codebook (k-means centroids).
//!
//! K-means training is in [`super::pq_kmeans`], OPQ rotation in [`super::pq_opq`].

use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use super::pq_kmeans::{kmeans_train, l2_squared, nearest_centroid};

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

/// Validate common training parameters shared by [`ProductQuantizer::train`] and
/// [`super::pq_opq::train_opq`].
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
pub(super) fn validate_train_params(
    vectors: &[Vec<f32>],
    num_subspaces: usize,
    num_centroids: usize,
) -> Result<(usize, usize), Error> {
    validate_basic_params(vectors, num_subspaces, num_centroids)?;

    let dimension = vectors[0].len();
    validate_dimension(vectors, dimension, num_subspaces, num_centroids)?;

    let subspace_dim = dimension / num_subspaces;
    Ok((dimension, subspace_dim))
}

/// Validates non-empty dataset and non-zero subspace/centroid counts.
fn validate_basic_params(
    vectors: &[Vec<f32>],
    num_subspaces: usize,
    num_centroids: usize,
) -> Result<(), Error> {
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
    Ok(())
}

/// Validates dimension uniformity, divisibility, and centroid count bounds.
fn validate_dimension(
    vectors: &[Vec<f32>],
    dimension: usize,
    num_subspaces: usize,
    num_centroids: usize,
) -> Result<(), Error> {
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
    Ok(())
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

        let centroids =
            train_subspace_centroids(vectors, num_subspaces, subspace_dim, num_centroids);

        // Post-training: degenerate centroid detection.
        // This O(k^2) check is only run in debug builds.
        #[cfg(debug_assertions)]
        check_degenerate_centroids(&centroids);

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

/// Train centroids for a single subspace via k-means.
fn train_single_subspace(
    vectors: &[Vec<f32>],
    subspace: usize,
    subspace_dim: usize,
    num_centroids: usize,
    #[cfg(feature = "gpu")] gpu_ctx: Option<&crate::gpu::PqGpuContext>,
) -> Vec<Vec<f32>> {
    let start = subspace * subspace_dim;
    let end = start + subspace_dim;
    let sub_vectors: Vec<Vec<f32>> = vectors.iter().map(|v| v[start..end].to_vec()).collect();
    #[allow(clippy::cast_possible_truncation)]
    let seed = 42u64.wrapping_add(subspace as u64);
    kmeans_train(
        &sub_vectors,
        num_centroids,
        50,
        seed,
        #[cfg(feature = "gpu")]
        gpu_ctx,
    )
}

/// Train centroids for all subspaces, using rayon when persistence is enabled.
fn train_subspace_centroids(
    vectors: &[Vec<f32>],
    num_subspaces: usize,
    subspace_dim: usize,
    num_centroids: usize,
) -> Vec<Vec<Vec<f32>>> {
    #[cfg(feature = "gpu")]
    let gpu_ctx = crate::gpu::PqGpuContext::new();

    #[cfg(feature = "persistence")]
    {
        use rayon::prelude::*;
        (0..num_subspaces)
            .into_par_iter()
            .map(|s| {
                train_single_subspace(
                    vectors,
                    s,
                    subspace_dim,
                    num_centroids,
                    #[cfg(feature = "gpu")]
                    gpu_ctx.as_ref(),
                )
            })
            .collect()
    }
    #[cfg(not(feature = "persistence"))]
    {
        (0..num_subspaces)
            .map(|s| {
                train_single_subspace(
                    vectors,
                    s,
                    subspace_dim,
                    num_centroids,
                    #[cfg(feature = "gpu")]
                    gpu_ctx.as_ref(),
                )
            })
            .collect()
    }
}

/// Debug-only check for degenerate (nearly duplicate) centroids after training.
#[cfg(debug_assertions)]
fn check_degenerate_centroids(centroids: &[Vec<Vec<f32>>]) {
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
/// Applies OPQ rotation to the query when the quantizer has a rotation matrix,
/// matching the space in which the codebook centroids were trained.
///
/// This is a crate-internal function. Inputs are expected to be valid by
/// construction: `query_vector.len() == quantizer.codebook.dimension` and
/// `pq_vector.codes.len() == quantizer.codebook.num_subspaces`. These invariants
/// are enforced at insert/train time and asserted only in debug builds.
#[must_use]
#[allow(dead_code)]
pub(crate) fn distance_pq_l2(
    query_vector: &[f32],
    pq_vector: &PQVector,
    quantizer: &ProductQuantizer,
) -> f32 {
    let codebook = &quantizer.codebook;
    debug_assert_eq!(query_vector.len(), codebook.dimension);
    debug_assert_eq!(pq_vector.codes.len(), codebook.num_subspaces);

    // apply_rotation returns Cow::Borrowed when rotation is None (no allocation),
    // Cow::Owned with the rotated vector otherwise. Centroids are in rotated space when
    // OPQ is enabled, so the query must be rotated to the same space before computing ADC.
    let query = quantizer.apply_rotation(query_vector);

    let mut lookup_tables = Vec::with_capacity(codebook.num_subspaces);
    for subspace in 0..codebook.num_subspaces {
        let start = subspace * codebook.subspace_dim;
        let end = start + codebook.subspace_dim;
        let q_sub = &query[start..end];

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

#[cfg(test)]
#[path = "pq_tests.rs"]
mod tests;
