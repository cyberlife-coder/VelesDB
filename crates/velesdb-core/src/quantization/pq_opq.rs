//! Optimized Product Quantization (OPQ) via PCA pre-rotation.
//!
//! When enabled, OPQ computes principal components of the training data via
//! simultaneous subspace iteration, producing an orthogonal rotation matrix
//! that reduces inter-subspace correlation and improves recall by 3-15%.

use crate::error::Error;

use super::pq::{validate_train_params, ProductQuantizer};

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
///   Values in the range 5-20 are typical; 10 is a good default.
///
/// # Errors
///
/// Returns `Error::InvalidQuantizerConfig` for invalid inputs (same conditions as `train()`).
#[cfg(feature = "persistence")]
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

    let cov = compute_covariance_matrix(vectors, d);
    let rotation = build_rotation_matrix(&cov, d, vectors.len(), power_iterations);

    // Train PQ on rotated vectors
    let rotated_vectors: Vec<Vec<f32>> = vectors
        .iter()
        .map(|v| mat_vec_mul(&rotation, v, d))
        .collect();
    let mut final_pq = ProductQuantizer::train(&rotated_vectors, num_subspaces, num_centroids)?;
    final_pq.rotation = Some(rotation);

    Ok(final_pq)
}

/// Compute the column-wise mean of the dataset.
#[cfg(feature = "persistence")]
fn compute_column_mean(vectors: &[Vec<f32>], d: usize) -> Vec<f64> {
    let mut mean = vec![0.0_f64; d];
    for v in vectors {
        for (i, &val) in v.iter().enumerate() {
            mean[i] += f64::from(val);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let inv_n = 1.0_f64 / vectors.len() as f64;
    for m in &mut mean {
        *m *= inv_n;
    }
    mean
}

/// Compute the covariance matrix of the dataset (flattened row-major d x d, f64).
#[cfg(feature = "persistence")]
fn compute_covariance_matrix(vectors: &[Vec<f32>], d: usize) -> Vec<f64> {
    let mean = compute_column_mean(vectors, d);

    #[allow(clippy::cast_precision_loss)]
    let inv_n = 1.0_f64 / vectors.len() as f64;
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

    cov
}

/// Build the PCA rotation matrix via simultaneous subspace iteration.
///
/// Returns a flattened row-major D x D f32 matrix whose rows are principal
/// components sorted by descending eigenvalue.
#[cfg(feature = "persistence")]
fn build_rotation_matrix(cov: &[f64], d: usize, n: usize, power_iterations: usize) -> Vec<f32> {
    let num_subspace_iters = power_iterations * 20;
    let mut q_cols = init_random_orthonormal_matrix(d, n);

    for _ in 0..num_subspace_iters {
        let z_cols = multiply_cov_by_q(cov, &q_cols, d);
        q_cols = gram_schmidt_orthonormalize(z_cols, d);
    }

    sort_eigenvectors_into_rotation(cov, &q_cols, d)
}

/// Initialize a d x d random orthonormal matrix (columns) via modified Gram-Schmidt.
///
/// Seeded from dimension and sample count for reproducibility.
#[cfg(feature = "persistence")]
fn init_random_orthonormal_matrix(d: usize, n: usize) -> Vec<Vec<f64>> {
    use rand::{Rng, SeedableRng};

    #[allow(clippy::cast_possible_truncation)]
    let init_seed = (d as u64).wrapping_mul(6_364_136_223_846_793_005)
        ^ (n as u64).wrapping_mul(1_442_695_040_888_963_407);
    let mut rng = rand::rngs::StdRng::seed_from_u64(init_seed);

    let mut cols: Vec<Vec<f64>> = (0..d)
        .map(|_| (0..d).map(|_| rng.gen::<f64>() * 2.0 - 1.0).collect())
        .collect();

    orthonormalize_columns(&mut cols, d);
    cols
}

/// Orthonormalize columns in-place via modified Gram-Schmidt.
#[cfg(feature = "persistence")]
fn orthonormalize_columns(cols: &mut [Vec<f64>], d: usize) {
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
}

/// Compute Z = C * Q (matrix-matrix multiply, column-wise).
#[cfg(feature = "persistence")]
fn multiply_cov_by_q(cov: &[f64], q_cols: &[Vec<f64>], d: usize) -> Vec<Vec<f64>> {
    (0..d)
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
        .collect()
}

/// Re-orthonormalize columns via modified Gram-Schmidt.
#[cfg(feature = "persistence")]
fn gram_schmidt_orthonormalize(mut cols: Vec<Vec<f64>>, d: usize) -> Vec<Vec<f64>> {
    orthonormalize_columns(&mut cols, d);
    cols
}

/// Sort eigenvectors by descending Rayleigh quotient and build f32 rotation matrix.
#[cfg(feature = "persistence")]
fn sort_eigenvectors_into_rotation(cov: &[f64], q_cols: &[Vec<f64>], d: usize) -> Vec<f32> {
    let mut eigenvalue_col_pairs: Vec<(f64, usize)> = q_cols
        .iter()
        .enumerate()
        .map(|(j, q)| {
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

    let mut rotation = vec![0.0_f32; d * d];
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

    rotation
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
