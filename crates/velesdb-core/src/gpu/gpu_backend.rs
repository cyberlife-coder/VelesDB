//! GPU-accelerated batch distance calculations via wgpu (WebGPU).
//!
//! Provides batch distance calculations on GPU for large datasets.
//! WGSL shader sources are in `shaders.rs`.

mod shaders;

use std::sync::{Arc, OnceLock};

use wgpu::util::DeviceExt;

/// Lazily-initialized singleton GPU accelerator.
///
/// `None` means GPU probe was attempted and failed (no compatible adapter).
///
/// The probe is **one-shot**: `OnceLock` guarantees the initialization closure
/// runs exactly once. If no GPU is found on that first probe, subsequent calls
/// to [`GpuAccelerator::global()`] return `None` forever. A process restart is
/// required if a GPU becomes available after the initial probe (e.g. hot-plug
/// or driver recovery).
static GPU_INSTANCE: OnceLock<Option<Arc<GpuAccelerator>>> = OnceLock::new();

/// GPU accelerator for batch vector operations.
///
/// # Example
///
/// ```ignore
/// use velesdb_core::gpu::GpuAccelerator;
///
/// if let Some(gpu) = GpuAccelerator::global() {
///     let results = gpu.batch_cosine_similarity(&vectors, &query, dimension)?;
/// }
/// ```
pub struct GpuAccelerator {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    cosine_pipeline: wgpu::ComputePipeline,
    euclidean_pipeline: wgpu::ComputePipeline,
    dot_product_pipeline: wgpu::ComputePipeline,
    kmeans_pipeline: wgpu::ComputePipeline,
}

impl GpuAccelerator {
    /// Returns a shared singleton GPU accelerator, initializing on first call.
    ///
    /// Probes the GPU exactly once. Subsequent calls return the cached `Arc`
    /// (or `None` if no compatible GPU was found on the first probe).
    #[must_use]
    pub fn global() -> Option<Arc<Self>> {
        GPU_INSTANCE
            .get_or_init(|| Self::new().map(Arc::new))
            .clone()
    }

    /// Creates a new GPU accelerator if GPU is available.
    ///
    /// Returns `None` if no compatible GPU is found.
    #[must_use]
    pub(crate) fn new() -> Option<Self> {
        let (device, queue) = Self::init_device()?;

        let cosine_pipeline = Self::compile_pipeline(
            &device,
            shaders::COSINE_SHADER,
            "batch_cosine",
            "Cosine Similarity",
        );
        let euclidean_pipeline = Self::compile_pipeline(
            &device,
            shaders::EUCLIDEAN_SHADER,
            "batch_euclidean",
            "Euclidean Distance",
        );
        let dot_product_pipeline = Self::compile_pipeline(
            &device,
            shaders::DOT_PRODUCT_SHADER,
            "batch_dot",
            "Dot Product",
        );
        let kmeans_pipeline = Self::compile_pipeline(
            &device,
            shaders::PQ_KMEANS_ASSIGN_SHADER,
            "kmeans_assign",
            "PQ K-means Assignment",
        );

        Some(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            cosine_pipeline,
            euclidean_pipeline,
            dot_product_pipeline,
            kmeans_pipeline,
        })
    }

    /// Probes the system for a compatible GPU and returns a `(Device, Queue)` pair.
    ///
    /// Returns `None` if no adapter is found or device creation fails.
    ///
    /// Delegates to a background thread so `pollster::block_on` never panics
    /// when called from within an async runtime (e.g. tokio in velesdb-server).
    /// [`super::pq_gpu::PqGpuContext::new`] delegates to [`Self::global`], which
    /// calls this method once via [`OnceLock`].
    fn init_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        std::thread::spawn(Self::init_device_sync)
            .join()
            .ok()
            .flatten()
    }

    /// Synchronous device initialization -- must NOT be called from inside an
    /// async context (use [`init_device`] instead).
    fn init_device_sync() -> Option<(wgpu::Device, wgpu::Queue)> {
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

        pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("VelesDB GPU"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()
    }

    /// Compiles a WGSL compute shader into a [`wgpu::ComputePipeline`].
    ///
    /// Uses the shared quad bind-group layout from [`super::helpers`]:
    /// binding 0 = storage(read), binding 1 = storage(read),
    /// binding 2 = `storage(read_write)`, binding 3 = uniform.
    ///
    /// All four shaders (cosine, euclidean, `dot_product`, kmeans) share this
    /// structural layout. The uniform buffer's internal data layout (e.g.,
    /// 2-field params for distance vs 4-field for kmeans) is interpreted
    /// by the shader code, not constrained by the bind group layout.
    fn compile_pipeline(
        device: &wgpu::Device,
        shader_source: &str,
        entry_point: &str,
        label: &str,
    ) -> wgpu::ComputePipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("{label} Shader")),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout = super::helpers::create_quad_bind_group_layout(
            device,
            &format!("{label} Bind Group Layout"),
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{label} Pipeline Layout")),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(&format!("{label} Pipeline")),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some(entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
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
    ///
    /// Delegates to [`Self::global()`], so the first call initializes the
    /// singleton and subsequent calls reuse the cached probe result.
    #[must_use]
    pub fn is_available() -> bool {
        Self::global().is_some()
    }

    /// Returns a reference to the underlying wgpu device.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Returns a reference to the underlying wgpu queue.
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Returns a reference to the PQ k-means assignment pipeline.
    #[must_use]
    pub fn kmeans_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.kmeans_pipeline
    }

    /// Computes batch cosine similarities between a query and multiple vectors.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if `dimension` or `num_vectors` exceeds `u32::MAX`,
    /// or if the GPU map-async operation fails.
    pub fn batch_cosine_similarity(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> crate::error::Result<Vec<f32>> {
        self.dispatch_batch_distance(&self.cosine_pipeline, vectors, query, dimension)
    }

    // RF-DEDUP: Shared GPU dispatch eliminates duplication across cosine/euclidean/dot batch methods.
    /// Dispatches a batch distance computation on the GPU using the given pipeline.
    ///
    /// All three distance metrics (cosine, euclidean, dot product) share the same
    /// buffer layout and dispatch pattern; only the compiled pipeline differs.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if `dimension` or `num_vectors` exceeds `u32::MAX`,
    /// or if the GPU map-async operation fails.
    fn dispatch_batch_distance(
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

        Self::validate_gpu_params(dimension, num_vectors)?;

        let (results_buffer, staging_buffer, bind_group, results_size) =
            self.create_distance_buffers(pipeline, vectors, query, dimension, num_vectors);

        Self::encode_and_submit(
            &self.device,
            &self.queue,
            pipeline,
            &bind_group,
            &results_buffer,
            &staging_buffer,
            results_size,
            num_vectors,
        );

        // Read back results using shared helper
        super::helpers::readback_buffer::<f32>(&self.device, &staging_buffer, num_vectors)
            .ok_or_else(|| {
                crate::error::Error::GpuError("GPU map-async operation failed".to_string())
            })
    }

    /// Validates that `dimension` and `num_vectors` fit in `u32` for GPU shader params.
    fn validate_gpu_params(dimension: usize, num_vectors: usize) -> crate::error::Result<()> {
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
        Ok(())
    }

    /// Creates GPU buffers and bind group for a batch distance dispatch.
    ///
    /// Returns `(results_buffer, staging_buffer, bind_group, results_size)`.
    fn create_distance_buffers(
        &self,
        pipeline: &wgpu::ComputePipeline,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
        num_vectors: usize,
    ) -> (wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup, u64) {
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

        // Reason: num_vectors * 4 bytes always fits in u64 (validated by u32 check above)
        #[allow(clippy::cast_possible_truncation)]
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

        // Reason: dimension and num_vectors validated to fit in u32 by validate_gpu_params
        #[allow(clippy::cast_possible_truncation)]
        let params = [dimension as u32, num_vectors as u32];
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Params Buffer"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Distance Bind Group"),
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

        (results_buffer, staging_buffer, bind_group, results_size)
    }

    /// Encodes the compute pass and submits it to the GPU queue.
    // Reason: GPU encode needs device, queue, pipeline, bind_group, and 3 buffer
    // refs — bundling into a struct would add lifetime complexity for a private fn.
    #[allow(clippy::too_many_arguments)]
    fn encode_and_submit(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::ComputePipeline,
        bind_group: &wgpu::BindGroup,
        results_buffer: &wgpu::Buffer,
        staging_buffer: &wgpu::Buffer,
        results_size: u64,
        num_vectors: usize,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Distance Encoder"),
        });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Distance Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(pipeline);
            compute_pass.set_bind_group(0, bind_group, &[]);

            // Reason: num_vectors validated to fit in u32; div_ceil(256) only reduces the value.
            #[allow(clippy::cast_possible_truncation)]
            let workgroups = num_vectors.div_ceil(256) as u32;
            compute_pass.dispatch_workgroups(workgroups, 1, 1);
        }

        encoder.copy_buffer_to_buffer(results_buffer, 0, staging_buffer, 0, results_size);
        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Computes batch Euclidean distances between a query and multiple vectors.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if `dimension` or `num_vectors` exceeds `u32::MAX`,
    /// or if the GPU map-async operation fails.
    pub fn batch_euclidean_distance(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> crate::error::Result<Vec<f32>> {
        self.dispatch_batch_distance(&self.euclidean_pipeline, vectors, query, dimension)
    }

    /// Computes batch dot products between a query and multiple vectors.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if `dimension` or `num_vectors` exceeds `u32::MAX`,
    /// or if the GPU map-async operation fails.
    pub fn batch_dot_product(
        &self,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> crate::error::Result<Vec<f32>> {
        self.dispatch_batch_distance(&self.dot_product_pipeline, vectors, query, dimension)
    }

    /// Returns `true` if GPU reranking is likely faster than sequential SIMD.
    ///
    /// Benchmarks show wgpu has ~900 us of fixed overhead per dispatch (buffer
    /// upload + compute pass + poll + readback). SIMD with prefetch remains
    /// faster until the payload exceeds ~1 MB of float data (262,144 f32s).
    /// The threshold `rerank_k * dimension > 262_144` corresponds to roughly
    /// 100K vectors at dim=3 or 170 vectors at dim=1536.
    #[must_use]
    pub fn should_rerank_gpu(rerank_k: usize, dimension: usize) -> bool {
        rerank_k.saturating_mul(dimension) > 262_144
    }

    /// Computes batch distances using the appropriate GPU pipeline for the given metric.
    ///
    /// Returns `Option<Result<Vec<f32>>>` to communicate two distinct failure modes:
    /// - `None` — the metric has no GPU shader (Hamming, Jaccard). Caller should
    ///   fall back to CPU.
    /// - `Some(Err(...))` — the GPU dispatch failed (buffer overflow, map-async
    ///   error). Caller should fall back to CPU.
    /// - `Some(Ok(scores))` — successful GPU computation.
    ///
    /// # Errors
    ///
    /// Returns `Error::GpuError` if `dimension` or `num_vectors` exceeds `u32::MAX`,
    /// or if the GPU map-async operation fails.
    #[must_use]
    pub fn batch_distance_for_metric(
        &self,
        metric: crate::distance::DistanceMetric,
        vectors: &[f32],
        query: &[f32],
        dimension: usize,
    ) -> Option<crate::error::Result<Vec<f32>>> {
        match metric {
            crate::distance::DistanceMetric::Cosine => {
                Some(self.batch_cosine_similarity(vectors, query, dimension))
            }
            crate::distance::DistanceMetric::Euclidean => {
                Some(self.batch_euclidean_distance(vectors, query, dimension))
            }
            crate::distance::DistanceMetric::DotProduct => {
                Some(self.batch_dot_product(vectors, query, dimension))
            }
            // Hamming and Jaccard have no GPU shader pipeline.
            _ => None,
        }
    }
}
