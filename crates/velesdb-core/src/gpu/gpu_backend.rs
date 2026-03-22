//! GPU-accelerated batch distance calculations via wgpu (WebGPU).
//!
//! Provides batch distance calculations on GPU for large datasets.
//! WGSL shader sources are in `shaders.rs`.
//!
//! # Auto-calibration
//!
//! On first initialization ([`GpuAccelerator::new()`]), a micro-benchmark
//! measures the fixed GPU dispatch overhead and marginal per-float cost,
//! then computes the crossover point above which GPU is faster than CPU
//! SIMD. The result is stored in [`GpuCalibration`] and exposed via
//! [`GpuAccelerator::calibration()`]. If calibration fails (e.g. the
//! adapter does not support timing, or a dispatch errors out), the
//! conservative defaults from [`GpuCalibration::default()`] are used.

mod shaders;

use std::sync::{Arc, OnceLock};

use wgpu::util::DeviceExt;

/// Auto-calibrated GPU/SIMD crossover parameters.
///
/// Populated by a micro-benchmark during [`GpuAccelerator::new()`].
/// If calibration fails, defaults to conservative thresholds.
#[derive(Debug, Clone)]
pub struct GpuCalibration {
    /// Fixed GPU dispatch overhead in nanoseconds.
    overhead_ns: u64,
    /// Marginal GPU cost per float in nanoseconds.
    gpu_ns_per_float: f64,
    /// Marginal SIMD cost per float in nanoseconds.
    simd_ns_per_float: f64,
    /// Number of floats above which GPU is faster than SIMD.
    crossover_floats: usize,
}

/// Default calibration: conservative values that prefer SIMD unless the payload
/// is large (~1 MB of f32 data). These are used when GPU micro-benchmarking
/// fails (e.g. adapter does not support timing, or dispatch errors out).
impl Default for GpuCalibration {
    fn default() -> Self {
        Self {
            overhead_ns: 900_000,
            gpu_ns_per_float: 0.0,
            simd_ns_per_float: 0.0,
            crossover_floats: 262_144,
        }
    }
}

impl GpuCalibration {
    /// Returns the fixed GPU dispatch overhead in nanoseconds.
    #[must_use]
    pub fn overhead_ns(&self) -> u64 {
        self.overhead_ns
    }

    /// Returns the marginal GPU cost per float in nanoseconds.
    #[must_use]
    pub fn gpu_ns_per_float(&self) -> f64 {
        self.gpu_ns_per_float
    }

    /// Returns the marginal SIMD cost per float in nanoseconds.
    #[must_use]
    pub fn simd_ns_per_float(&self) -> f64 {
        self.simd_ns_per_float
    }

    /// Returns the crossover point in floats above which GPU is faster.
    #[must_use]
    pub fn crossover_floats(&self) -> usize {
        self.crossover_floats
    }
}

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
/// if let Some(gpu) = GpuAccelerator::new() {
///     let results = gpu.batch_cosine_similarity(&vectors, &query);
/// }
/// ```
pub struct GpuAccelerator {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    cosine_pipeline: wgpu::ComputePipeline,
    euclidean_pipeline: wgpu::ComputePipeline,
    dot_product_pipeline: wgpu::ComputePipeline,
    kmeans_pipeline: wgpu::ComputePipeline,
    calibration: GpuCalibration,
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
    pub fn new() -> Option<Self> {
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

        let mut accel = Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            cosine_pipeline,
            euclidean_pipeline,
            dot_product_pipeline,
            kmeans_pipeline,
            calibration: GpuCalibration::default(),
        };
        accel.calibration = accel.calibrate();
        Some(accel)
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
    /// Uses the shared quad bind-group layout from [`super::helpers`].
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

    /// Returns the auto-calibrated GPU/SIMD crossover parameters.
    #[must_use]
    pub fn calibration(&self) -> &GpuCalibration {
        &self.calibration
    }

    // =====================================================================
    // Calibration helpers
    // =====================================================================

    /// Runs a micro-benchmark to measure GPU dispatch overhead and per-float
    /// marginal cost, then computes the crossover point vs SIMD.
    ///
    /// Falls back to [`GpuCalibration::default()`] if any dispatch errors out.
    fn calibrate(&self) -> GpuCalibration {
        const DIM: usize = 128;
        const WARMUP_DISPATCHES: usize = 2;
        const LARGE_BATCH: usize = 1000;

        // Warmup: prime GPU caches / driver JIT
        for _ in 0..WARMUP_DISPATCHES {
            if self.measure_gpu_dispatch_ns(1, DIM).is_none() {
                tracing::info!("GPU calibration failed during warmup, using conservative defaults");
                return GpuCalibration::default();
            }
        }

        let overhead = self.measure_overhead_ns(DIM);
        let gpu_large = self.measure_gpu_large_ns(LARGE_BATCH, DIM);
        let simd_large = Self::measure_simd_baseline_ns(LARGE_BATCH, DIM);

        let cal = Self::compute_crossover(overhead, gpu_large, simd_large);
        tracing::info!(
            overhead_us = cal.overhead_ns / 1000,
            crossover_floats = cal.crossover_floats,
            "GPU auto-calibration complete"
        );
        cal
    }

    /// Measures the fixed GPU dispatch overhead by dispatching a single vector
    /// three times and taking the median.
    fn measure_overhead_ns(&self, dim: usize) -> Option<u64> {
        let mut samples = [0u64; 3];
        for s in &mut samples {
            *s = self.measure_gpu_dispatch_ns(1, dim)?;
        }
        Some(median_of_three(samples[0], samples[1], samples[2]))
    }

    /// Measures the total GPU time for a large batch and returns the median
    /// of three runs.
    fn measure_gpu_large_ns(&self, num_vectors: usize, dim: usize) -> Option<u64> {
        let mut samples = [0u64; 3];
        for s in &mut samples {
            *s = self.measure_gpu_dispatch_ns(num_vectors, dim)?;
        }
        Some(median_of_three(samples[0], samples[1], samples[2]))
    }

    /// Times a single `dispatch_batch_distance` call in nanoseconds.
    fn measure_gpu_dispatch_ns(&self, num_vectors: usize, dimension: usize) -> Option<u64> {
        let query = vec![0.01_f32; dimension];
        let vectors = vec![0.01_f32; num_vectors.saturating_mul(dimension)];

        let start = std::time::Instant::now();
        self.dispatch_batch_distance(&self.cosine_pipeline, &vectors, &query, dimension)
            .ok()?;
        Some(elapsed_nanos_u64(&start))
    }

    /// Measures the total SIMD time for `num_vectors` cosine similarity calls
    /// and returns the median of three runs.
    fn measure_simd_baseline_ns(num_vectors: usize, dimension: usize) -> u64 {
        let query = vec![0.01_f32; dimension];
        let vector = vec![0.01_f32; dimension];

        let mut samples = [0u64; 3];
        for s in &mut samples {
            let start = std::time::Instant::now();
            for _ in 0..num_vectors {
                std::hint::black_box(crate::simd_native::cosine_similarity_native(
                    &query, &vector,
                ));
            }
            *s = elapsed_nanos_u64(&start);
        }
        median_of_three(samples[0], samples[1], samples[2])
    }

    /// Computes the crossover calibration from raw measurements.
    ///
    /// Returns default calibration if measurements are missing or the GPU
    /// marginal cost exceeds SIMD marginal cost (GPU is always slower).
    /// Computes the float counts used in the large-batch calibration probe.
    const CALIBRATE_LARGE_BATCH: usize = 1000;
    /// Dimension used for calibration probes.
    const CALIBRATE_DIM: usize = 128;

    fn compute_crossover(
        overhead: Option<u64>,
        gpu_large: Option<u64>,
        simd_large: u64,
    ) -> GpuCalibration {
        let (Some(overhead_ns), Some(gpu_total)) = (overhead, gpu_large) else {
            return GpuCalibration::default();
        };

        let total_floats = Self::CALIBRATE_LARGE_BATCH.saturating_mul(Self::CALIBRATE_DIM);

        // Reason: total_floats (128_000) always fits comfortably in f64.
        #[allow(clippy::cast_precision_loss)]
        let total_floats_f64 = total_floats as f64;
        // Reason: gpu_total and overhead_ns are sub-second nanosecond counts; f64 is exact.
        #[allow(clippy::cast_precision_loss)]
        let gpu_ns_per_float = gpu_total.saturating_sub(overhead_ns) as f64 / total_floats_f64;
        // Reason: simd_large is a sub-second nanosecond count; f64 is exact.
        #[allow(clippy::cast_precision_loss)]
        let simd_ns_per_float = simd_large as f64 / total_floats_f64;

        // If GPU marginal cost >= SIMD marginal cost, GPU is never worth it.
        if gpu_ns_per_float >= simd_ns_per_float {
            return GpuCalibration {
                overhead_ns,
                gpu_ns_per_float,
                simd_ns_per_float,
                crossover_floats: usize::MAX,
            };
        }

        // crossover = overhead / (simd_per_float - gpu_per_float)
        let delta = simd_ns_per_float - gpu_ns_per_float;

        // Reason: overhead_ns is a sub-second nanosecond count; f64 is exact.
        #[allow(clippy::cast_precision_loss)]
        let crossover_f64 = overhead_ns as f64 / delta;

        let crossover = crossover_f64_to_usize(crossover_f64);

        GpuCalibration {
            overhead_ns,
            gpu_ns_per_float,
            simd_ns_per_float,
            crossover_floats: crossover.max(1),
        }
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
    // Reason: wgpu encode+submit needs device, queue, pipeline, bind_group,
    // two buffers, results_size, and num_vectors — all distinct concerns.
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
    /// The crossover threshold is auto-calibrated during [`GpuAccelerator::new()`]
    /// by micro-benchmarking actual GPU dispatch overhead vs SIMD throughput.
    /// Defaults to 262,144 floats (~1 MB) if calibration fails.
    #[must_use]
    pub fn should_rerank_gpu(&self, rerank_k: usize, dimension: usize) -> bool {
        rerank_k.saturating_mul(dimension) > self.calibration.crossover_floats
    }

    /// Returns `true` if GPU brute-force search is likely faster than rayon SIMD.
    ///
    /// Uses the same auto-calibrated crossover as [`Self::should_rerank_gpu`]:
    /// `num_vectors * dimension` must exceed the calibrated float threshold.
    #[must_use]
    pub fn should_brute_force_gpu(&self, num_vectors: usize, dimension: usize) -> bool {
        num_vectors.saturating_mul(dimension) > self.calibration.crossover_floats
    }

    /// Returns `true` if GPU PQ k-means assignment is likely faster than CPU.
    ///
    /// The workload size is `n * k * subspace_dim` floats. Uses the same
    /// auto-calibrated crossover threshold as the other GPU decision methods.
    #[must_use]
    pub fn should_use_gpu_pq(&self, n: usize, k: usize, subspace_dim: usize) -> bool {
        n.saturating_mul(k).saturating_mul(subspace_dim) > self.calibration.crossover_floats
    }

    /// Computes batch distances using the appropriate GPU pipeline for the given metric.
    ///
    /// Returns `None` for metrics without GPU support (Hamming, Jaccard).
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

/// Returns the median of three `u64` values.
fn median_of_three(a: u64, b: u64, c: u64) -> u64 {
    let mut arr = [a, b, c];
    arr.sort_unstable();
    arr[1]
}

/// Converts elapsed time to nanoseconds clamped to `u64` range.
///
/// `Instant::elapsed().as_nanos()` returns `u128`; this helper clamps it to
/// `u64::MAX` (584 years of nanoseconds) so callers never overflow.
fn elapsed_nanos_u64(start: &std::time::Instant) -> u64 {
    // Reason: value is clamped to u64::MAX before the cast, so truncation
    // cannot occur in practice.
    #[allow(clippy::cast_possible_truncation)]
    let ns = start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    ns
}

/// Converts a positive `f64` crossover value to `usize`, clamping at `usize::MAX`.
fn crossover_f64_to_usize(value: f64) -> usize {
    // Reason: value is always non-negative (overhead / positive delta);
    // clamped to usize::MAX before cast.
    #[allow(clippy::cast_precision_loss)]
    let max_f64 = usize::MAX as f64;

    // Reason: value is clamped before casting; sign_loss impossible (always >= 0).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    if value >= max_f64 {
        usize::MAX
    } else {
        value as usize
    }
}
