//! GPU-accelerated batch distance calculations via wgpu (WebGPU).
//!
//! Provides batch distance calculations on GPU for large datasets.
//! WGSL shader sources are in `shaders.rs`.

mod shaders;

use std::sync::OnceLock;
use wgpu::util::DeviceExt;

// Import for CPU fallback paths
use crate::simd_native;

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
    // GPU initialization is inherently sequential: instance → adapter → device → queue →
    // shader → pipeline. Splitting this into sub-functions would create artificial
    // intermediate state without reducing cognitive complexity.
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Option<Self> {
        // Avoid probing GLES/EGL on headless Linux where some drivers may abort.
        let backends = Self::preferred_backends();
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
            source: wgpu::ShaderSource::Wgsl(shaders::COSINE_SHADER.into()),
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

    #[must_use]
    fn preferred_backends() -> wgpu::Backends {
        #[cfg(target_os = "linux")]
        {
            let has_display = std::env::var_os("DISPLAY").is_some()
                || std::env::var_os("WAYLAND_DISPLAY").is_some();
            if !has_display {
                return wgpu::Backends::VULKAN;
            }
        }

        wgpu::Backends::all()
    }

    /// Checks if GPU acceleration is available (cached).
    #[must_use]
    pub fn is_available() -> bool {
        *GPU_AVAILABLE.get_or_init(|| Self::new().is_some())
    }

    /// Computes batch cosine similarities between a query and multiple vectors.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if `dimension` or `num_vectors` exceeds `u32::MAX`,
    /// or if the GPU map-async operation fails.
    #[allow(clippy::too_many_lines)]
    pub fn batch_cosine_similarity(
        &self,
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
                "dimension {dimension} exceeds u32::MAX"
            )));
        }
        if u32::try_from(num_vectors).is_err() {
            return Err(crate::error::Error::GpuError(format!(
                "num_vectors {num_vectors} exceeds u32::MAX"
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

    /// Computes batch Euclidean distances between a query and multiple vectors.
    ///
    /// Currently uses CPU SIMD fallback; GPU pipeline ready via `EUCLIDEAN_SHADER`.
    #[must_use]
    pub fn batch_euclidean_distance(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> Vec<f32> {
        if dimension == 0 || vectors.is_empty() {
            return Vec::new();
        }
        let num_vectors = vectors.len() / dimension;
        if num_vectors == 0 {
            return Vec::new();
        }

        // CPU fallback using direct SIMD dispatch for optimal performance
        let mut results = Vec::with_capacity(num_vectors);
        for i in 0..num_vectors {
            let offset = i * dimension;
            let vec = &vectors[offset..offset + dimension];
            let dist = simd_native::euclidean_native(query, vec);
            results.push(dist);
        }
        results
    }

    /// Computes batch dot products between a query and multiple vectors.
    ///
    /// Currently uses CPU SIMD fallback; GPU pipeline ready via `DOT_PRODUCT_SHADER`.
    #[must_use]
    pub fn batch_dot_product(&self, vectors: &[f32], query: &[f32], dimension: usize) -> Vec<f32> {
        if dimension == 0 || vectors.is_empty() {
            return Vec::new();
        }
        let num_vectors = vectors.len() / dimension;
        if num_vectors == 0 {
            return Vec::new();
        }

        // CPU fallback using direct SIMD dispatch for optimal performance
        let mut results = Vec::with_capacity(num_vectors);
        for i in 0..num_vectors {
            let offset = i * dimension;
            let vec = &vectors[offset..offset + dimension];
            let dot = simd_native::dot_product_native(query, vec);
            results.push(dot);
        }
        results
    }
}
