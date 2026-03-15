//! K-means training and initialization for Product Quantization.
//!
//! Contains k-means++ initialization, the main k-means loop with GPU fallback,
//! and shared distance utilities used across PQ modules.

use rand::Rng;

/// Squared L2 distance between two slices of equal length.
pub(crate) fn l2_squared(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

/// Return the index of the centroid closest to `vector` by L2 distance.
pub(crate) fn nearest_centroid(vector: &[f32], centroids: &[Vec<f32>]) -> usize {
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

/// K-means++ initialization: picks well-spread initial centroids.
///
/// Step 1: Choose the first centroid uniformly at random.
/// Step 2: For each subsequent centroid, pick a sample with probability
///         proportional to D(x)^2 (squared distance to nearest existing centroid).
pub(crate) fn kmeans_plusplus_init(
    samples: &[Vec<f32>],
    k: usize,
    rng: &mut impl Rng,
) -> Vec<Vec<f32>> {
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
        update_min_distances(&mut min_dists, samples, &centroids[iter - 1]);

        if let Some(chosen) = pick_weighted_sample(&min_dists, rng) {
            centroids.push(samples[chosen].clone());
        } else {
            // All remaining samples coincide with existing centroids.
            fill_remaining_centroids(&mut centroids, samples, k, n);
            break;
        }
    }

    centroids
}

/// Update `min_dists[i]` to be the minimum of current value and distance to
/// `last_centroid` for each sample.
fn update_min_distances(min_dists: &mut [f32], samples: &[Vec<f32>], last_centroid: &[f32]) {
    for (i, sample) in samples.iter().enumerate() {
        let dist = l2_squared(sample, last_centroid);
        if dist < min_dists[i] {
            min_dists[i] = dist;
        }
    }
}

/// Pick a sample index with probability proportional to `min_dists[i]`.
///
/// Returns `None` if the total weight is zero (all samples coincide with centroids).
fn pick_weighted_sample(min_dists: &[f32], rng: &mut impl Rng) -> Option<usize> {
    let total: f64 = min_dists.iter().map(|&d| f64::from(d)).sum();
    if total <= 0.0 {
        return None;
    }

    let threshold = rng.gen::<f64>() * total;
    let mut cumulative = 0.0_f64;
    for (i, &d) in min_dists.iter().enumerate() {
        cumulative += f64::from(d);
        if cumulative >= threshold {
            return Some(i);
        }
    }
    // Default to last if rounding issues
    Some(min_dists.len() - 1)
}

/// Fill remaining centroid slots by cycling through samples.
fn fill_remaining_centroids(
    centroids: &mut Vec<Vec<f32>>,
    samples: &[Vec<f32>],
    k: usize,
    n: usize,
) {
    tracing::warn!(
        remaining = k - centroids.len(),
        existing = centroids.len(),
        "k-means++: all samples coincide with existing centroids; \
         using sequential fallback — degenerate centroids likely"
    );
    for i in centroids.len()..k {
        centroids.push(samples[i % n].clone());
    }
}

/// Run k-means clustering on `samples`, returning `k` centroids.
///
/// Uses k-means++ initialization and supports optional GPU-accelerated
/// assignment when the `gpu` feature is enabled.
#[allow(clippy::too_many_lines)]
pub(crate) fn kmeans_train(
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
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut centroids = kmeans_plusplus_init(samples, k.min(samples.len()), &mut rng);
    // If k > samples.len(), pad with cycled samples (shouldn't happen after train() validation).
    while centroids.len() < k {
        centroids.push(samples[centroids.len() % samples.len()].clone());
    }

    let mut assignments = vec![0usize; samples.len()];

    for _iter in 0..max_iters {
        let changed = assign_samples(
            samples,
            &centroids,
            &mut assignments,
            dim,
            #[cfg(feature = "gpu")]
            gpu_ctx,
        );

        let new_centroids = update_centroids(samples, &assignments, &centroids, k, dim, &mut rng);

        let converged = has_converged(&centroids, &new_centroids);
        centroids = new_centroids;

        if !changed || converged {
            break;
        }
    }

    centroids
}

/// Assign each sample to its nearest centroid. Returns `true` if any assignment changed.
#[allow(unused_variables)]
fn assign_samples(
    samples: &[Vec<f32>],
    centroids: &[Vec<f32>],
    assignments: &mut [usize],
    dim: usize,
    #[cfg(feature = "gpu")] gpu_ctx: Option<&crate::gpu::PqGpuContext>,
) -> bool {
    let mut changed = false;

    // Try GPU acceleration if beneficial.
    #[cfg(feature = "gpu")]
    {
        let gpu_used = try_gpu_assign(samples, centroids, assignments, dim, gpu_ctx, &mut changed);
        if gpu_used {
            return changed;
        }
    }

    // CPU fallback assignment
    for (i, sample) in samples.iter().enumerate() {
        let new_assignment = nearest_centroid(sample, centroids);
        if assignments[i] != new_assignment {
            assignments[i] = new_assignment;
            changed = true;
        }
    }
    changed
}

/// Attempt GPU-accelerated assignment. Returns `true` if GPU was used.
#[cfg(feature = "gpu")]
fn try_gpu_assign(
    samples: &[Vec<f32>],
    centroids: &[Vec<f32>],
    assignments: &mut [usize],
    dim: usize,
    gpu_ctx: Option<&crate::gpu::PqGpuContext>,
    changed: &mut bool,
) -> bool {
    use crate::gpu;
    if let Some(ctx) = gpu_ctx {
        let k = centroids.len();
        if gpu::should_use_gpu(samples.len(), k, dim) {
            if let Some(gpu_assignments) = gpu::gpu_kmeans_assign(ctx, samples, centroids, dim) {
                for (i, &new_assignment) in gpu_assignments.iter().enumerate() {
                    if assignments[i] != new_assignment {
                        assignments[i] = new_assignment;
                        *changed = true;
                    }
                }
                return true;
            }
        }
    }
    false
}

/// Recompute centroids from current assignments, re-seeding empty clusters.
fn update_centroids(
    samples: &[Vec<f32>],
    assignments: &[usize],
    old_centroids: &[Vec<f32>],
    k: usize,
    dim: usize,
    rng: &mut impl Rng,
) -> Vec<Vec<f32>> {
    let mut new_centroids = vec![vec![0.0; dim]; k];
    let mut counts = vec![0usize; k];

    for (sample, &cluster) in samples.iter().zip(assignments.iter()) {
        counts[cluster] += 1;
        for (d, &val) in sample.iter().enumerate() {
            new_centroids[cluster][d] += val;
        }
    }

    let largest_cluster_idx = counts
        .iter()
        .enumerate()
        .max_by_key(|&(_, &c)| c)
        .map_or(0, |(idx, _)| idx);

    for cluster in 0..k {
        if counts[cluster] == 0 {
            // Re-seed empty cluster by splitting the largest cluster:
            // clone its centroid and add small random perturbation.
            let source = old_centroids[largest_cluster_idx].clone();
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

    new_centroids
}

/// Check if centroids have converged (max relative movement < 1%).
fn has_converged(old: &[Vec<f32>], new: &[Vec<f32>]) -> bool {
    let max_delta = old
        .iter()
        .zip(new.iter())
        .map(|(old_c, new_c)| {
            let movement = l2_squared(old_c, new_c).sqrt();
            let norm = old_c.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > f32::EPSILON {
                movement / norm
            } else {
                movement
            }
        })
        .fold(0.0_f32, f32::max);

    max_delta < 0.01
}
