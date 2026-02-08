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
    euclidean_pipeline: wgpu::ComputePipeline,
    dot_product_pipeline: wgpu::ComputePipeline,
}

impl GpuAccelerator {
    /// Creates a new GPU accelerator if GPU is available.
    ///
    /// Returns `None` if no compatible GPU is found.
    #[must_use]
    #[allow(clippy::too_many_lines)] // Reason: GPU init requires sequential pipeline setup for 3 metrics
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

        // Create Euclidean distance pipeline (same bind group layout)
        let euclidean_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Euclidean Distance Shader"),
            source: wgpu::ShaderSource::Wgsl(EUCLIDEAN_SHADER.into()),
        });
        let euclidean_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Euclidean Distance Pipeline"),
            layout: Some(&pipeline_layout),
            module: &euclidean_shader,
            entry_point: Some("batch_euclidean"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Create Dot Product pipeline (same bind group layout)
        let dot_product_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dot Product Shader"),
            source: wgpu::ShaderSource::Wgsl(DOT_PRODUCT_SHADER.into()),
        });
        let dot_product_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Dot Product Pipeline"),
                layout: Some(&pipeline_layout),
                module: &dot_product_shader,
                entry_point: Some("batch_dot"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        Some(Self {
            device,
            queue,
            cosine_pipeline,
            euclidean_pipeline,
            dot_product_pipeline,
        })
    }

    /// Checks if GPU acceleration is available (cached).
    #[must_use]
    pub fn is_available() -> bool {
        *GPU_AVAILABLE.get_or_init(|| Self::new().is_some())
    }

    /// Computes batch cosine similarities on GPU.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if parameters exceed GPU limits or async operation fails.
    pub fn batch_cosine_similarity(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> crate::error::Result<Vec<f32>> {
        self.dispatch_gpu_batch(&self.cosine_pipeline, vectors, query, dimension)
    }

    /// Dispatches a GPU batch computation using the given pipeline.
    ///
    /// Shared implementation for cosine, euclidean, and dot product metrics.
    /// All pipelines share the same bind group layout: query, vectors, results, params.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if parameters exceed GPU limits or async operation fails.
    #[allow(clippy::too_many_lines)] // Reason: GPU batch operations require sequential setup steps
    fn dispatch_gpu_batch(
        &self,
        pipeline: &wgpu::ComputePipeline,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> crate::error::Result<Vec<f32>> {
        if dimension == 0 || vectors.is_empty() {
            return Ok(Vec::new());
        }
        let num_vectors = vectors.len() / dimension;
        if num_vectors == 0 {
            return Ok(Vec::new());
        }

        // Validate GPU shader parameter constraints
        if u32::try_from(dimension).is_err() {
            return Err(crate::error::Error::GpuError(format!(
                "dimension {} exceeds u32::MAX",
                dimension
            )));
        }
        if u32::try_from(num_vectors).is_err() {
            return Err(crate::error::Error::GpuError(format!(
                "num_vectors {} exceeds u32::MAX",
                num_vectors
            )));
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
        // SAFETY: dimension and num_vectors validated above to fit in u32
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
        let bind_group_layout = pipeline.get_bind_group_layout(0);
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
            compute_pass.set_pipeline(pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);

            // SAFETY: num_vectors is bounded by GPU buffer limits. div_ceil(256) reduces
            // the value further. Even 4B vectors / 256 = 16M workgroups, fitting in u32.
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
            return Err(crate::error::Error::GpuError(
                "GPU map-async operation failed".to_string(),
            ));
        }

        let data = buffer_slice.get_mapped_range();
        let results: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging_buffer.unmap();

        Ok(results)
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

impl GpuAccelerator {
    /// Computes batch Euclidean distances on GPU.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if parameters exceed GPU limits or async operation fails.
    pub fn batch_euclidean_distance(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> crate::error::Result<Vec<f32>> {
        self.dispatch_gpu_batch(&self.euclidean_pipeline, vectors, query, dimension)
    }

    /// Computes batch dot products on GPU.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if parameters exceed GPU limits or async operation fails.
    pub fn batch_dot_product(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> crate::error::Result<Vec<f32>> {
        self.dispatch_gpu_batch(&self.dot_product_pipeline, vectors, query, dimension)
    }
}

/// WGSL compute shader for batch euclidean distance.
const EUCLIDEAN_SHADER: &str = r"
struct Params {
    dimension: u32,
    num_vectors: u32,
}

@group(0) @binding(0) var<storage, read> query: array<f32>;
@group(0) @binding(1) var<storage, read> vectors: array<f32>;
@group(0) @binding(2) var<storage, read_write> results: array<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(256)
fn batch_euclidean(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.num_vectors) {
        return;
    }
    
    let dim = params.dimension;
    let offset = idx * dim;
    
    var sum_sq: f32 = 0.0;
    
    for (var i: u32 = 0u; i < dim; i = i + 1u) {
        let diff = query[i] - vectors[offset + i];
        sum_sq = sum_sq + diff * diff;
    }
    
    results[idx] = sqrt(sum_sq);
}
";

/// WGSL compute shader for batch dot product.
const DOT_PRODUCT_SHADER: &str = r"
struct Params {
    dimension: u32,
    num_vectors: u32,
}

@group(0) @binding(0) var<storage, read> query: array<f32>;
@group(0) @binding(1) var<storage, read> vectors: array<f32>;
@group(0) @binding(2) var<storage, read_write> results: array<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(256)
fn batch_dot(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.num_vectors) {
        return;
    }
    
    let dim = params.dimension;
    let offset = idx * dim;
    
    var dot: f32 = 0.0;
    
    for (var i: u32 = 0u; i < dim; i = i + 1u) {
        dot = dot + query[i] * vectors[offset + i];
    }
    
    results[idx] = dot;
}
";
