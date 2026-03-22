//! GPU-accelerated k-means assignment for PQ training.
//!
//! Provides optional GPU acceleration for the k-means assignment step during
//! PQ codebook training. Falls back silently to CPU when GPU is unavailable
//! or the dataset is too small to benefit from GPU dispatch.
//!
//! ## Usage
//!
//! Create a [`PqGpuContext`] once before entering the k-means loop, then pass
//! a reference to each [`gpu_kmeans_assign`] call. The underlying GPU device,
//! queue, and compiled pipeline are shared with the [`super::gpu_backend::GpuAccelerator`]
//! singleton, so no duplicate GPU resources are allocated.

use std::sync::Arc;
use wgpu::util::DeviceExt;

use super::gpu_backend::GpuAccelerator;

/// Reusable GPU context for PQ k-means assignment.
///
/// Wraps a shared [`GpuAccelerator`] singleton that owns the wgpu device,
/// queue, and all compiled compute pipelines (including the k-means pipeline).
/// Pass a reference to each [`gpu_kmeans_assign`] call to amortize the
/// ~100-500 ms initialization cost over all k-means iterations.
///
/// Returns `None` from [`PqGpuContext::new`] if no suitable GPU adapter is
/// available.
pub struct PqGpuContext {
    gpu: Arc<GpuAccelerator>,
}

impl PqGpuContext {
    /// Obtain a PQ GPU context backed by the global [`GpuAccelerator`] singleton.
    ///
    /// Returns `None` if no suitable GPU adapter is found (identical to
    /// [`GpuAccelerator::is_available`] returning `false`).
    ///
    /// # Notes
    ///
    /// The 10 M FLOP threshold in [`should_use_gpu`] is tuned for discrete GPUs.
    /// Integrated GPUs may benefit from a higher threshold (e.g. 50 M FLOPs)
    /// due to shared memory bandwidth constraints.
    #[must_use]
    pub fn new() -> Option<Self> {
        let gpu = GpuAccelerator::global()?;
        Some(Self { gpu })
    }
}

/// Check if GPU dispatch is worthwhile based on FLOP threshold.
///
/// Returns `true` when `n * k * subspace_dim > 10_000_000` (~10 M FLOPs).
/// This is a pure arithmetic check — it does **not** verify GPU availability.
/// Callers must separately check that a GPU context exists before dispatching.
///
/// Uses `saturating_mul` to prevent overflow on 32-bit targets.
///
/// The 10 M FLOP threshold is calibrated for discrete GPUs; integrated
/// GPUs may require a higher value due to shared memory bandwidth.
#[must_use]
pub fn should_use_gpu(n: usize, k: usize, subspace_dim: usize) -> bool {
    n.saturating_mul(k).saturating_mul(subspace_dim) > 10_000_000
}

/// Compute k-means assignments on GPU for one subspace.
///
/// For each vector, finds the nearest centroid index using L2 distance.
/// Returns assignment indices (one per vector), or `None` if GPU dispatch fails.
///
/// Requires a pre-initialized [`PqGpuContext`] to avoid re-paying device init
/// overhead on every call.
///
/// Falls back to `None` if:
/// - Input dimensions are inconsistent (`sub_vectors[i].len() != subspace_dim`).
/// - Buffer creation fails.
/// - Compute dispatch fails.
/// - Result readback fails.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn gpu_kmeans_assign(
    ctx: &PqGpuContext,
    sub_vectors: &[Vec<f32>],
    centroids: &[Vec<f32>],
    subspace_dim: usize,
) -> Option<Vec<usize>> {
    if sub_vectors.is_empty() || centroids.is_empty() || subspace_dim == 0 {
        return None;
    }

    // Validate that all inputs have the expected dimension.
    if sub_vectors.iter().any(|v| v.len() != subspace_dim)
        || centroids.iter().any(|c| c.len() != subspace_dim)
    {
        return None;
    }

    let n = sub_vectors.len();
    let k = centroids.len();

    // Flatten vectors and centroids into contiguous buffers.
    let flat_vectors = super::helpers::flatten_vecs(sub_vectors, subspace_dim);
    let flat_centroids = super::helpers::flatten_vecs(centroids, subspace_dim);

    let device = ctx.gpu.device();
    let queue = ctx.gpu.queue();
    let pipeline = ctx.gpu.kmeans_pipeline();

    let buffers = create_kmeans_buffers(device, &flat_vectors, &flat_centroids, n, k, subspace_dim);

    // Obtain bind group layout from the compiled pipeline (same pattern as
    // batch_cosine_similarity in gpu_backend.rs).
    let bind_group_layout = pipeline.get_bind_group_layout(0);
    let bind_group = create_kmeans_bind_group(device, &bind_group_layout, &buffers);

    dispatch_and_readback(device, queue, pipeline, &bind_group, &buffers, n)
}

/// GPU buffers needed for a single k-means assignment dispatch.
struct KmeansBuffers {
    vectors: wgpu::Buffer,
    centroids: wgpu::Buffer,
    assignments: wgpu::Buffer,
    staging: wgpu::Buffer,
    params: wgpu::Buffer,
    assignments_size: u64,
}

/// Creates all GPU buffers for a k-means assignment dispatch.
fn create_kmeans_buffers(
    device: &wgpu::Device,
    flat_vectors: &[f32],
    flat_centroids: &[f32],
    n: usize,
    k: usize,
    subspace_dim: usize,
) -> KmeansBuffers {
    let vectors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Vectors Buffer"),
        contents: bytemuck::cast_slice(flat_vectors),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let centroids = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Centroids Buffer"),
        contents: bytemuck::cast_slice(flat_centroids),
        usage: wgpu::BufferUsages::STORAGE,
    });

    // Reason: n is bounded by training set size (thousands of vectors).
    // n * 4 bytes is well within u64::MAX even for billions of vectors.
    #[allow(clippy::cast_possible_truncation)]
    let assignments_size = (n * std::mem::size_of::<u32>()) as u64;
    let assignments = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Assignments Buffer"),
        size: assignments_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Staging Buffer"),
        size: assignments_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Params: [num_vectors, num_centroids, subspace_dim, padding]
    // Reason: n, k, and subspace_dim are bounded by training set size (thousands)
    // and centroid count (<=65535), well within u32 range.
    #[allow(clippy::cast_possible_truncation)]
    let params_data = [n as u32, k as u32, subspace_dim as u32, 0_u32];
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Params Buffer"),
        contents: bytemuck::cast_slice(&params_data),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    KmeansBuffers {
        vectors,
        centroids,
        assignments,
        staging,
        params,
        assignments_size,
    }
}

/// Creates the bind group wiring buffers to shader bindings.
fn create_kmeans_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &KmeansBuffers,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("PQ K-means Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.vectors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.centroids.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.assignments.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.params.as_entire_binding(),
            },
        ],
    })
}

/// Dispatches the compute pipeline and reads back results.
fn dispatch_and_readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    bind_group: &wgpu::BindGroup,
    buffers: &KmeansBuffers,
    n: usize,
) -> Option<Vec<usize>> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("PQ K-means Encoder"),
    });

    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("PQ K-means Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);

        // Reason: n is bounded by training set size. div_ceil(256) reduces
        // the value further. Even 4B vectors / 256 = 16M workgroups, fitting in u32.
        #[allow(clippy::cast_possible_truncation)]
        let workgroups = n.div_ceil(256) as u32;
        pass.dispatch_workgroups(workgroups, 1, 1);
    }

    encoder.copy_buffer_to_buffer(
        &buffers.assignments,
        0,
        &buffers.staging,
        0,
        buffers.assignments_size,
    );
    queue.submit(std::iter::once(encoder.finish()));

    // Read back results using shared helper.
    let assignments_u32 = super::helpers::readback_buffer::<u32>(device, &buffers.staging, n)?;
    // Reason: u32 → usize is lossless on all platforms where wgpu runs
    // (32-bit and 64-bit). `From<u32>` is not implemented for `usize` in
    // libstd, so we use the infallible `as` cast.
    #[allow(clippy::cast_lossless)]
    let assignments: Vec<usize> = assignments_u32.iter().map(|&a| a as usize).collect();

    Some(assignments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_use_gpu_threshold() {
        // Below threshold: 100 * 16 * 8 = 12800 < 10M
        assert!(!should_use_gpu(100, 16, 8));

        // Above threshold: 10000 * 256 * 8 = 20_480_000 > 10M
        assert!(should_use_gpu(10000, 256, 8));

        // Exactly at threshold: should not trigger (strictly greater)
        assert!(!should_use_gpu(10_000_000 / (256 * 8), 256, 8));
    }

    #[test]
    fn test_gpu_context_new_does_not_panic() {
        // PqGpuContext::new() either succeeds or returns None -- must not panic.
        // This validates the singleton delegation works regardless of whether
        // we are in an async runtime.
        let _ctx = PqGpuContext::new();
        // No assertion: GPU may not be available in CI. Absence of panic is the test.
    }

    #[test]
    fn test_gpu_kmeans_assign_matches_cpu() {
        // Small dataset: 10 vectors, 3 centroids, dim=4
        let sub_vectors = vec![
            vec![1.0, 0.0, 0.0, 0.0],
            vec![0.9, 0.1, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0],
            vec![0.1, 0.9, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 0.0],
            vec![0.0, 0.0, 0.9, 0.1],
            vec![1.0, 1.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.0, 1.0],
            vec![0.5, 0.5, 0.0, 0.0],
            vec![0.0, 0.0, 0.5, 0.5],
        ];
        let centroids = vec![
            vec![1.0, 0.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 0.0],
        ];

        // CPU assignments (nearest centroid by L2)
        let cpu_assignments: Vec<usize> = sub_vectors
            .iter()
            .map(|v| {
                centroids
                    .iter()
                    .enumerate()
                    .map(|(idx, c)| {
                        let dist: f32 =
                            v.iter().zip(c.iter()).map(|(a, b)| (a - b) * (a - b)).sum();
                        (idx, dist)
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .map(|(idx, _)| idx)
                    .unwrap()
            })
            .collect();

        // GPU assignments (may return None if no GPU available or context init fails)
        if let Some(ctx) = PqGpuContext::new() {
            if let Some(gpu_assignments) = gpu_kmeans_assign(&ctx, &sub_vectors, &centroids, 4) {
                assert_eq!(
                    gpu_assignments.len(),
                    sub_vectors.len(),
                    "GPU must return one assignment per vector"
                );
                assert_eq!(
                    gpu_assignments, cpu_assignments,
                    "GPU assignments must match CPU"
                );
            }
        }
        // If GPU is not available, test passes silently (fallback behavior)
    }

    #[test]
    fn test_gpu_kmeans_assign_empty_input() {
        if let Some(ctx) = PqGpuContext::new() {
            assert!(gpu_kmeans_assign(&ctx, &[], &[vec![1.0]], 1).is_none());
            assert!(gpu_kmeans_assign(&ctx, &[vec![1.0]], &[], 1).is_none());
            assert!(gpu_kmeans_assign(&ctx, &[vec![1.0]], &[vec![1.0]], 0).is_none());
        }
    }

    #[test]
    fn test_gpu_kmeans_assign_dimension_mismatch_returns_none() {
        // sub_vectors[0] has dim=3 but subspace_dim=4 -> must return None
        if let Some(ctx) = PqGpuContext::new() {
            let sub_vectors = vec![vec![1.0, 0.0, 0.0]]; // dim=3
            let centroids = vec![vec![1.0, 0.0, 0.0, 0.0]]; // dim=4
            assert!(
                gpu_kmeans_assign(&ctx, &sub_vectors, &centroids, 4).is_none(),
                "mismatched sub_vector dim must return None"
            );
        }
    }

    /// Regression guard: `PqGpuContext::new()` must return `Some` if and only if
    /// `GpuAccelerator::is_available()` returns `true`. After consolidation
    /// (Step 0.16), both go through the same singleton -- this test ensures
    /// they stay in sync.
    #[test]
    fn test_pq_context_shares_global_device() {
        let gpu_available = GpuAccelerator::is_available();
        let pq_ctx = PqGpuContext::new();

        assert_eq!(
            pq_ctx.is_some(),
            gpu_available,
            "PqGpuContext availability must match GpuAccelerator::is_available()"
        );
    }
}
