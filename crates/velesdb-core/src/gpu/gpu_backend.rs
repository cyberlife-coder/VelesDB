//! GPU backend implementation using wgpu.
//!
//! Provides batch distance calculations on GPU for large datasets.

use std::sync::OnceLock;
use wgpu::util::DeviceExt;

/// Global GPU availability check (cached).
static GPU_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// GPU accelerator for batch vector operations.
///
/// # Example
///
/// ```ignore
/// use velesdb_core::gpu::GpuAccelerator;
///
/// if let Some(gpu) = GpuAccelerator::new() {
///     let results = gpu.batch_cosine_similarity(&vectors, &query);
/// }
/// ```
pub struct GpuAccelerator {
    device: wgpu::Device,
    queue: wgpu::Queue,
    cosine_pipeline: wgpu::ComputePipeline,
}

impl GpuAccelerator {
    /// Creates a new GPU accelerator if GPU is available.
    ///
    /// Returns `None` if no compatible GPU is found.
    #[must_use]
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("VelesDB GPU"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;

        // Create compute shader for cosine similarity
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cosine Similarity Shader"),
            source: wgpu::ShaderSource::Wgsl(COSINE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cosine Bind Group Layout"),
            entries: &[
                // Query vector
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Vectors buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Results buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Params (dimension, num_vectors)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cosine Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let cosine_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Cosine Similarity Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("batch_cosine"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Some(Self {
            device,
            queue,
            cosine_pipeline,
        })
    }

    /// Checks if GPU acceleration is available (cached).
    #[must_use]
    pub fn is_available() -> bool {
        *GPU_AVAILABLE.get_or_init(|| Self::new().is_some())
    }

    /// Computes batch cosine similarities between a query and multiple vectors.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Flat array of vectors (`num_vectors` * `dimension`)
    /// * `query` - Query vector
    /// * `dimension` - Vector dimension
    ///
    /// # Returns
    ///
    /// Vector of cosine similarities, one per input vector.
    #[must_use]
    pub fn batch_cosine_similarity(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> Vec<f32> {
        let num_vectors = vectors.len() / dimension;
        if num_vectors == 0 {
            return Vec::new();
        }

        // Create buffers
        let query_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Query Buffer"),
                contents: bytemuck::cast_slice(query),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let vectors_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Vectors Buffer"),
                contents: bytemuck::cast_slice(vectors),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let results_size = (num_vectors * std::mem::size_of::<f32>()) as u64;
        let results_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Results Buffer"),
            size: results_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Buffer"),
            size: results_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Params: [dimension, num_vectors]
        #[allow(clippy::cast_possible_truncation)]
        let params = [dimension as u32, num_vectors as u32];
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Params Buffer"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Create bind group
        let bind_group_layout = self.cosine_pipeline.get_bind_group_layout(0);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cosine Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: query_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: vectors_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: results_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        // Dispatch compute
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Cosine Encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Cosine Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.cosine_pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);

            #[allow(clippy::cast_possible_truncation)]
            let workgroups = num_vectors.div_ceil(256) as u32;
            compute_pass.dispatch_workgroups(workgroups, 1, 1);
        }

        // Copy results to staging buffer
        encoder.copy_buffer_to_buffer(&results_buffer, 0, &staging_buffer, 0, results_size);
        self.queue.submit(std::iter::once(encoder.finish()));

        // Read back results
        let buffer_slice = staging_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);

        if rx.recv().ok().and_then(Result::ok).is_none() {
            return vec![0.0; num_vectors];
        }

        let data = buffer_slice.get_mapped_range();
        let results: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging_buffer.unmap();

        results
    }
}

/// WGSL compute shader for batch cosine similarity.
const COSINE_SHADER: &str = r"
struct Params {
    dimension: u32,
    num_vectors: u32,
}

@group(0) @binding(0) var<storage, read> query: array<f32>;
@group(0) @binding(1) var<storage, read> vectors: array<f32>;
@group(0) @binding(2) var<storage, read_write> results: array<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(256)
fn batch_cosine(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.num_vectors) {
        return;
    }
    
    let dim = params.dimension;
    let offset = idx * dim;
    
    var dot: f32 = 0.0;
    var norm_q: f32 = 0.0;
    var norm_v: f32 = 0.0;
    
    for (var i: u32 = 0u; i < dim; i = i + 1u) {
        let q = query[i];
        let v = vectors[offset + i];
        dot = dot + q * v;
        norm_q = norm_q + q * q;
        norm_v = norm_v + v * v;
    }
    
    let denom = sqrt(norm_q) * sqrt(norm_v);
    if (denom > 0.0) {
        results[idx] = dot / denom;
    } else {
        results[idx] = 0.0;
    }
}
";

// =============================================================================
// Tests (TDD)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_available_check() {
        // Should not panic
        let _ = GpuAccelerator::is_available();
    }

    #[test]
    fn test_gpu_accelerator_creation() {
        // May return None if no GPU available (CI environment)
        let gpu = GpuAccelerator::new();
        if gpu.is_some() {
            println!("GPU available for testing");
        } else {
            println!("No GPU available, skipping GPU tests");
        }
    }

    #[test]
    fn test_batch_cosine_empty_input() {
        if let Some(gpu) = GpuAccelerator::new() {
            let results = gpu.batch_cosine_similarity(&[], &[1.0, 0.0, 0.0], 3);
            assert!(results.is_empty());
        }
    }

    #[test]
    fn test_batch_cosine_identical_vectors() {
        if let Some(gpu) = GpuAccelerator::new() {
            // Query and vector are identical -> similarity should be 1.0
            let query = vec![1.0, 0.0, 0.0];
            let vectors = vec![1.0, 0.0, 0.0]; // One vector

            let results = gpu.batch_cosine_similarity(&vectors, &query, 3);

            assert_eq!(results.len(), 1);
            assert!(
                (results[0] - 1.0).abs() < 0.01,
                "Expected ~1.0, got {}",
                results[0]
            );
        }
    }

    #[test]
    fn test_batch_cosine_orthogonal_vectors() {
        if let Some(gpu) = GpuAccelerator::new() {
            let query = vec![1.0, 0.0, 0.0];
            let vectors = vec![0.0, 1.0, 0.0]; // Orthogonal

            let results = gpu.batch_cosine_similarity(&vectors, &query, 3);

            assert_eq!(results.len(), 1);
            assert!(results[0].abs() < 0.01, "Expected ~0.0, got {}", results[0]);
        }
    }

    #[test]
    fn test_batch_cosine_multiple_vectors() {
        if let Some(gpu) = GpuAccelerator::new() {
            let query = vec![1.0, 0.0, 0.0];
            // 3 vectors of dimension 3
            let vectors = vec![
                1.0, 0.0, 0.0, // Identical -> 1.0
                0.0, 1.0, 0.0, // Orthogonal -> 0.0
                -1.0, 0.0, 0.0, // Opposite -> -1.0
            ];

            let results = gpu.batch_cosine_similarity(&vectors, &query, 3);

            assert_eq!(results.len(), 3);
            assert!((results[0] - 1.0).abs() < 0.01, "Expected ~1.0");
            assert!(results[1].abs() < 0.01, "Expected ~0.0");
            assert!((results[2] + 1.0).abs() < 0.01, "Expected ~-1.0");
        }
    }
}
