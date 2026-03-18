//! GPU-accelerated k-means assignment for PQ training.
//!
//! Provides optional GPU acceleration for the k-means assignment step during
//! PQ codebook training. Falls back silently to CPU when GPU is unavailable
//! or the dataset is too small to benefit from GPU dispatch.
//!
//! ## Usage
//!
//! Create a [`PqGpuContext`] once before entering the k-means loop, then pass
//! a reference to each [`gpu_kmeans_assign`] call. This avoids paying the
//! device-init overhead (~100–500 ms) on every iteration.

use std::sync::Arc;
use wgpu::util::DeviceExt;

/// WGSL compute shader for PQ k-means assignment.
///
/// For each vector, finds the nearest centroid by L2 distance.
const PQ_KMEANS_ASSIGN_SHADER: &str = r"
struct Params {
    num_vectors: u32,
    num_centroids: u32,
    subspace_dim: u32,
    _padding: u32,
}

@group(0) @binding(0) var<storage, read> vectors: array<f32>;
@group(0) @binding(1) var<storage, read> centroids: array<f32>;
@group(0) @binding(2) var<storage, read_write> assignments: array<u32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(256)
fn kmeans_assign(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.num_vectors) { return; }

    let sd = params.subspace_dim;
    let k = params.num_centroids;
    let vec_offset = idx * sd;

    var best_dist: f32 = 3.4028235e+38;
    var best_idx: u32 = 0u;

    for (var c: u32 = 0u; c < k; c = c + 1u) {
        let cent_offset = c * sd;
        var dist: f32 = 0.0;
        for (var d: u32 = 0u; d < sd; d = d + 1u) {
            let diff = vectors[vec_offset + d] - centroids[cent_offset + d];
            dist = dist + diff * diff;
        }
        if (dist < best_dist) {
            best_dist = dist;
            best_idx = c;
        }
    }
    assignments[idx] = best_idx;
}
";

/// Reusable GPU context for PQ k-means assignment.
///
/// Creating a [`PqGpuContext`] initializes the wgpu adapter, device, queue,
/// and compiled compute pipeline once. Pass a reference to each
/// [`gpu_kmeans_assign`] call to amortize the ~100–500 ms initialization cost
/// over all k-means iterations.
///
/// Returns `None` if no suitable GPU adapter is available.
pub struct PqGpuContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: Arc<wgpu::ComputePipeline>,
    bind_group_layout: Arc<wgpu::BindGroupLayout>,
}

impl PqGpuContext {
    /// Initialize the GPU context: adapter, device, queue, and compiled pipeline.
    ///
    /// Returns `None` if:
    /// - No suitable GPU adapter is found.
    /// - Device creation fails.
    ///
    /// # Notes
    ///
    /// The 10 M FLOP threshold in [`should_use_gpu`] is tuned for discrete GPUs.
    /// Integrated GPUs may benefit from a higher threshold (e.g. 50 M FLOPs)
    /// due to shared memory bandwidth constraints.
    #[must_use]
    pub fn new() -> Option<Self> {
        // Dispatch to a background thread so `pollster::block_on` never panics
        // when called from within an async runtime (e.g. tokio in velesdb-server).
        std::thread::spawn(Self::new_sync).join().ok().flatten()
    }

    /// Synchronous initialization — must NOT be called from inside an async context.
    #[allow(clippy::too_many_lines)]
    fn new_sync() -> Option<Self> {
        let backends = wgpu::Backends::all();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("VelesDB PQ K-means"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;

        // Compile the shader and pipeline once.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PQ K-means Assignment Shader"),
            source: wgpu::ShaderSource::Wgsl(PQ_KMEANS_ASSIGN_SHADER.into()),
        });

        let bind_group_layout =
            super::helpers::create_quad_bind_group_layout(&device, "PQ K-means Bind Group Layout");

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PQ K-means Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("PQ K-means Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("kmeans_assign"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Some(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            pipeline: Arc::new(pipeline),
            bind_group_layout: Arc::new(bind_group_layout),
        })
    }
}

/// Check if GPU dispatch is worthwhile based on FLOP threshold.
///
/// GPU dispatch overhead (device init, buffer copies, kernel launch) is only
/// amortized when the computation exceeds ~10 M FLOPs. This threshold is
/// calibrated for discrete GPUs; integrated GPUs may require a higher value
/// due to shared memory bandwidth constraints.
#[must_use]
pub fn should_use_gpu(n: usize, k: usize, subspace_dim: usize) -> bool {
    n * k * subspace_dim > 10_000_000
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

    let device = &ctx.device;
    let queue = &ctx.queue;

    // Create per-call buffers.
    let vectors_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Vectors Buffer"),
        contents: bytemuck::cast_slice(&flat_vectors),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let centroids_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Centroids Buffer"),
        contents: bytemuck::cast_slice(&flat_centroids),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let assignments_size = (n * std::mem::size_of::<u32>()) as u64;
    let assignments_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Assignments Buffer"),
        size: assignments_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Staging Buffer"),
        size: assignments_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Params: [num_vectors, num_centroids, subspace_dim, padding]
    // SAFETY: n, k, and subspace_dim are validated to be non-zero above.
    // In practice these are bounded by training set size (thousands) and
    // centroid count (<=65535), well within u32 range.
    #[allow(clippy::cast_possible_truncation)]
    let params = [n as u32, k as u32, subspace_dim as u32, 0_u32];
    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Params Buffer"),
        contents: bytemuck::cast_slice(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    // Create bind group using the reused pipeline layout.
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("PQ K-means Bind Group"),
        layout: &ctx.bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: vectors_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: centroids_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: assignments_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: params_buffer.as_entire_binding(),
            },
        ],
    });

    // Dispatch.
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("PQ K-means Encoder"),
    });

    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("PQ K-means Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&ctx.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);

        // SAFETY: n is bounded by training set size. div_ceil(256) reduces
        // the value further. Even 4B vectors / 256 = 16M workgroups, fitting in u32.
        #[allow(clippy::cast_possible_truncation)]
        let workgroups = n.div_ceil(256) as u32;
        pass.dispatch_workgroups(workgroups, 1, 1);
    }

    encoder.copy_buffer_to_buffer(&assignments_buffer, 0, &staging_buffer, 0, assignments_size);
    queue.submit(std::iter::once(encoder.finish()));

    // Read back results using shared helper.
    let assignments_u32 = super::helpers::readback_buffer::<u32>(device, &staging_buffer, n)?;
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
        // PqGpuContext::new() either succeeds or returns None — must not panic.
        // This validates the thread-spawn + pollster approach works regardless
        // of whether we are in an async runtime.
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
        // sub_vectors[0] has dim=3 but subspace_dim=4 → must return None
        if let Some(ctx) = PqGpuContext::new() {
            let sub_vectors = vec![vec![1.0, 0.0, 0.0]]; // dim=3
            let centroids = vec![vec![1.0, 0.0, 0.0, 0.0]]; // dim=4
            assert!(
                gpu_kmeans_assign(&ctx, &sub_vectors, &centroids, 4).is_none(),
                "mismatched sub_vector dim must return None"
            );
        }
    }
}
