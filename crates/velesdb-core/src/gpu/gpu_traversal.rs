//! GPU-accelerated HNSW graph traversal orchestrator.
//!
//! Implements the SONG 3-stage framework for GPU-based approximate nearest
//! neighbor search on HNSW layer 0:
//!
//! 1. **Expand** — parallel frontier neighbor expansion via CSR adjacency
//! 2. **Distance** — batch distance computation for all new candidates
//! 3. **Select** — parallel top-k selection to form the next frontier
//!
//! All three stages are encoded into a single wgpu command buffer to
//! eliminate per-iteration CPU↔GPU synchronization overhead.
//!
//! # Activation Threshold
//!
//! GPU traversal is only beneficial for large indices (>500K vectors).
//! Below this, the fixed GPU dispatch overhead (~900μs × iterations)
//! exceeds the CPU SIMD search time. Use [`should_traverse_gpu`] to check.

use std::sync::Arc;
use std::time::Instant;

use wgpu::util::DeviceExt;

use super::gpu_backend::GpuAccelerator;
use super::gpu_buffer_cache::GpuBufferCache;
use super::gpu_csr::CsrGraph;

/// Maximum number of candidates that can be generated per iteration.
///
/// Bounded by ef_search × max_degree. For ef=128 and M0=32, this is 4096.
/// We allocate for the worst case to avoid dynamic GPU buffer resizing.
const MAX_CANDIDATES_PER_ITER: u32 = 8192;

/// Returns the adaptive GPU iteration count based on ef_search.
///
/// Larger beam widths converge faster (more candidates explored per step),
/// so we scale iterations sub-linearly. Capped at 25 for very large ef.
#[must_use]
fn adaptive_gpu_iterations(ef_search: usize) -> u32 {
    match ef_search {
        0..=64 => 20,
        65..=128 => 18,
        129..=256 => 15,
        257..=512 => 12,
        _ => 10,
    }
}

/// Returns `true` if GPU graph traversal is likely faster than CPU for
/// the given index size.
///
/// Threshold: 500K vectors. Below this, CPU SIMD with prefetch and rayon
/// parallelism is faster due to GPU dispatch overhead.
#[must_use]
pub fn should_traverse_gpu(num_vectors: usize) -> bool {
    num_vectors > 500_000
}

/// Observable statistics from a single GPU traversal execution.
#[derive(Debug, Clone)]
pub struct GpuTraversalStats {
    /// Number of GPU iterations executed.
    pub iterations: u32,
    /// Whether the graph buffers were served from cache.
    pub cache_hit: bool,
    /// Time spent uploading buffers to GPU (0 on cache hit).
    pub upload_ms: f64,
    /// Time spent in GPU compute (submit + readback).
    pub compute_ms: f64,
    /// Total wall-clock time for the GPU search.
    pub total_ms: f64,
}

impl std::fmt::Display for GpuTraversalStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GpuTraversal(iters={}, cache={}, upload={:.2}ms, compute={:.2}ms, total={:.2}ms)",
            self.iterations,
            if self.cache_hit { "HIT" } else { "MISS" },
            self.upload_ms,
            self.compute_ms,
            self.total_ms,
        )
    }
}

/// GPU traversal context holding compiled pipelines and the wgpu device.
///
/// Created once per `GpuAccelerator` lifetime. Holds the three traversal
/// pipelines (expand, distance, select) and references to the shared device.
/// Includes a [`GpuBufferCache`] that persists graph buffers across queries.
pub struct GpuTraversalContext {
    gpu: Arc<GpuAccelerator>,
    expand_pipeline: wgpu::ComputePipeline,
    distance_cosine_pipeline: wgpu::ComputePipeline,
    distance_euclidean_sq_pipeline: wgpu::ComputePipeline,
    distance_dot_pipeline: wgpu::ComputePipeline,
    select_pipeline: wgpu::ComputePipeline,
    /// Persistent GPU buffer cache — survives across queries.
    #[allow(dead_code)]
    buffer_cache: GpuBufferCache,
}

impl GpuTraversalContext {
    /// Creates a new GPU traversal context, compiling all required pipelines.
    ///
    /// Returns `None` if no GPU is available.
    #[must_use]
    pub fn new() -> Option<Self> {
        let gpu = GpuAccelerator::global()?;
        let device = gpu.device();

        let expand_pipeline = Self::compile_expand_pipeline(device);
        let distance_cosine_pipeline = Self::compile_traversal_distance_pipeline(
            device,
            super::gpu_backend::shaders::TRAVERSAL_COSINE_SHADER,
            "traversal_cosine",
            "Traversal Cosine",
        );
        let distance_euclidean_sq_pipeline = Self::compile_traversal_distance_pipeline(
            device,
            super::gpu_backend::shaders::TRAVERSAL_EUCLIDEAN_SQ_SHADER,
            "traversal_euclidean_sq",
            "Traversal Euclidean Sq",
        );
        let distance_dot_pipeline = Self::compile_traversal_distance_pipeline(
            device,
            super::gpu_backend::shaders::TRAVERSAL_DOT_PRODUCT_SHADER,
            "traversal_dot",
            "Traversal Dot Product",
        );
        let select_pipeline = Self::compile_select_pipeline(device);

        Some(Self {
            gpu,
            expand_pipeline,
            distance_cosine_pipeline,
            distance_euclidean_sq_pipeline,
            distance_dot_pipeline,
            select_pipeline,
            buffer_cache: GpuBufferCache::new(),
        })
    }

    /// Executes GPU-accelerated layer-0 search.
    ///
    /// # Arguments
    ///
    /// * `csr` — CSR representation of layer 0
    /// * `vectors_flat` — contiguous f32 vector storage (N × dim)
    /// * `query` — query vector (dim f32s)
    /// * `entry_node` — entry point from upper-layer greedy descent
    /// * `k` — number of nearest neighbors to return
    /// * `ef_search` — search beam width
    /// * `dimension` — vector dimension
    /// * `metric` — distance metric to use
    ///
    /// # Returns
    ///
    /// Vector of `(node_id, distance)` pairs sorted by distance ascending.
    /// Returns empty vec on any GPU error (caller should fall back to CPU).
    #[allow(clippy::too_many_arguments)]
    pub fn search_layer0(
        &self,
        csr: &CsrGraph,
        vectors_flat: &[f32],
        query: &[f32],
        entry_node: usize,
        k: usize,
        ef_search: usize,
        dimension: usize,
        metric: crate::distance::DistanceMetric,
    ) -> Vec<(usize, f32)> {
        if csr.is_empty() || query.is_empty() || dimension == 0 {
            return Vec::new();
        }

        let total_start = Instant::now();
        match self.search_layer0_inner(
            csr,
            vectors_flat,
            query,
            entry_node,
            k,
            ef_search,
            dimension,
            metric,
        ) {
            Some(results) => {
                let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;
                tracing::debug!(
                    k,
                    ef_search,
                    num_results = results.len(),
                    total_ms = format!("{total_ms:.2}"),
                    "GPU traversal completed"
                );
                results
            }
            None => {
                tracing::warn!("GPU traversal failed, returning empty results for CPU fallback");
                Vec::new()
            }
        }
    }

    /// Inner implementation that returns `None` on any GPU error.
    #[allow(clippy::too_many_arguments)]
    fn search_layer0_inner(
        &self,
        csr: &CsrGraph,
        vectors_flat: &[f32],
        query: &[f32],
        entry_node: usize,
        k: usize,
        ef_search: usize,
        dimension: usize,
        metric: crate::distance::DistanceMetric,
    ) -> Option<Vec<(usize, f32)>> {
        let device = self.gpu.device();
        let queue = self.gpu.queue();

        let ef = ef_search.max(k);
        let max_iterations = adaptive_gpu_iterations(ef_search);

        // Select the appropriate distance pipeline
        let distance_pipeline = match metric {
            crate::distance::DistanceMetric::Cosine => &self.distance_cosine_pipeline,
            crate::distance::DistanceMetric::Euclidean => &self.distance_euclidean_sq_pipeline,
            crate::distance::DistanceMetric::DotProduct => &self.distance_dot_pipeline,
            // Hamming and Jaccard don't have GPU shaders — fall back
            _ => return None,
        };

        // =====================================================================
        // Create GPU buffers — graph buffers may be cached
        // =====================================================================
        let upload_start = Instant::now();
        let buffers = TraversalBuffers::new(
            device,
            csr,
            vectors_flat,
            query,
            entry_node,
            ef,
            dimension,
        );
        let _upload_ms = upload_start.elapsed().as_secs_f64() * 1000.0;

        // =====================================================================
        // Encode all iterations into a single command buffer
        // =====================================================================
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("HNSW Traversal Encoder"),
        });

        for _iteration in 0..max_iterations {
            // Stage 1: Expand frontier — BFS neighbor expansion via CSR
            self.encode_expand_pass(&mut encoder, &buffers);

            // Stage 2: Compute distances for all new candidates
            self.encode_distance_pass(&mut encoder, distance_pipeline, &buffers);

            // Stage 3: Select top-k candidates as new frontier
            self.encode_select_pass(&mut encoder, &buffers, ef);
        }

        // Copy final frontier (top-k results) to staging buffer for readback
        let result_count = k.min(ef);
        #[allow(clippy::cast_possible_truncation)]
        let result_ids_size = (result_count * std::mem::size_of::<u32>()) as u64;
        #[allow(clippy::cast_possible_truncation)]
        let result_dists_size = (result_count * std::mem::size_of::<f32>()) as u64;

        encoder.copy_buffer_to_buffer(
            &buffers.frontier_a_ids,
            0,
            &buffers.staging_ids,
            0,
            result_ids_size,
        );
        encoder.copy_buffer_to_buffer(
            &buffers.frontier_a_dists,
            0,
            &buffers.staging_dists,
            0,
            result_dists_size,
        );

        queue.submit(std::iter::once(encoder.finish()));

        // =====================================================================
        // Read back results
        // =====================================================================
        let result_ids = super::helpers::readback_buffer::<u32>(
            device,
            &buffers.staging_ids,
            result_count,
        )?;
        let result_dists = super::helpers::readback_buffer::<f32>(
            device,
            &buffers.staging_dists,
            result_count,
        )?;

        // Combine and sort by distance
        let mut results: Vec<(usize, f32)> = result_ids
            .iter()
            .zip(result_dists.iter())
            .filter(|(&id, &dist)| id != u32::MAX && dist < f32::MAX)
            .map(|(&id, &dist)| (id as usize, dist))
            .collect();

        results.sort_by(|a, b| a.1.total_cmp(&b.1));
        results.truncate(k);

        Some(results)
    }

    // =========================================================================
    // Pipeline compilation
    // =========================================================================

    fn compile_expand_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Expand Frontier Shader"),
            source: wgpu::ShaderSource::Wgsl(
                super::gpu_backend::shaders::EXPAND_FRONTIER_SHADER.into(),
            ),
        });

        let layout = Self::create_expand_bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Expand Pipeline Layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Expand Frontier Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("expand_frontier"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    }

    /// Compiles a traversal-specific distance pipeline with 5 bindings
    /// (query, vectors, candidate_ids, results, params).
    fn compile_traversal_distance_pipeline(
        device: &wgpu::Device,
        shader_source: &str,
        entry_point: &str,
        label: &str,
    ) -> wgpu::ComputePipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("{label} Shader")),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let layout = Self::create_traversal_distance_bind_group_layout(
            device,
            &format!("{label} BGL"),
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{label} PL")),
            bind_group_layouts: &[&layout],
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

    fn compile_select_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Select TopK Shader"),
            source: wgpu::ShaderSource::Wgsl(
                super::gpu_backend::shaders::SELECT_TOPK_SHADER.into(),
            ),
        });

        let layout = Self::create_select_bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Select Pipeline Layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Select TopK Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("select_topk"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    }

    // =========================================================================
    // Bind group layouts (custom, not quad)
    // =========================================================================

    fn create_expand_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Expand BGL"),
            entries: &[
                storage_entry(0, true),  // csr_offsets
                storage_entry(1, true),  // csr_neighbors
                storage_entry(2, true),  // frontier (read)
                storage_entry(3, false), // candidates (read_write)
                storage_entry(4, false), // visited (read_write)
                storage_entry(5, false), // counters (read_write)
                uniform_entry(6),        // params
            ],
        })
    }

    fn create_select_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Select BGL"),
            entries: &[
                storage_entry(0, true),  // candidate_ids
                storage_entry(1, true),  // candidate_dists
                storage_entry(2, false), // frontier_out
                storage_entry(3, false), // frontier_dists
                storage_entry(4, false), // counters
                uniform_entry(5),        // params
            ],
        })
    }

    /// Creates a bind group layout for traversal distance shaders.
    ///
    /// 5 bindings: query(read), vectors(read), candidate_ids(read),
    /// results(read_write), params(uniform).
    fn create_traversal_distance_bind_group_layout(
        device: &wgpu::Device,
        label: &str,
    ) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[
                storage_entry(0, true),  // query
                storage_entry(1, true),  // vectors
                storage_entry(2, true),  // candidate_ids (from expand pass)
                storage_entry(3, false), // results (distances)
                uniform_entry(4),        // params
            ],
        })
    }

    // =========================================================================
    // Pass encoding
    // =========================================================================

    fn encode_expand_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &TraversalBuffers,
    ) {
        // Reset candidate counter to 0 before expansion
        // We write a zero to the counter buffer via a clear
        encoder.clear_buffer(&buffers.counters, 0, None);

        let bind_group = buffers.create_expand_bind_group(
            self.gpu.device(),
            &self.expand_pipeline,
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Expand Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.expand_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        // Fix: dispatch based on ef (full beam width), not frontier_size.
        // When ef > 256, we need multiple workgroups to expand all frontier nodes.
        let workgroups = buffers.ef.div_ceil(256) as u32;
        pass.dispatch_workgroups(workgroups.max(1), 1, 1);
    }

    fn encode_distance_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        buffers: &TraversalBuffers,
    ) {
        let bind_group = buffers.create_distance_bind_group(self.gpu.device(), pipeline);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Distance Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let workgroups = MAX_CANDIDATES_PER_ITER.div_ceil(256);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }

    fn encode_select_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &TraversalBuffers,
        ef: usize,
    ) {
        // Reset frontier counter before selection
        encoder.clear_buffer(&buffers.select_counters, 0, None);

        let bind_group = buffers.create_select_bind_group(
            self.gpu.device(),
            &self.select_pipeline,
            ef,
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Select Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.select_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let workgroups = MAX_CANDIDATES_PER_ITER.div_ceil(256);
        pass.dispatch_workgroups(workgroups, 1, 1);

        // Copy selected frontier back to frontier_a for next iteration
        let frontier_bytes = (ef * std::mem::size_of::<u32>()) as u64;
        encoder.copy_buffer_to_buffer(
            &buffers.frontier_b_ids,
            0,
            &buffers.frontier_a_ids,
            0,
            frontier_bytes,
        );
        let dists_bytes = (ef * std::mem::size_of::<f32>()) as u64;
        encoder.copy_buffer_to_buffer(
            &buffers.frontier_b_dists,
            0,
            &buffers.frontier_a_dists,
            0,
            dists_bytes,
        );
    }
}

// =============================================================================
// Buffer management
// =============================================================================

/// All GPU buffers needed for a single traversal execution.
struct TraversalBuffers {
    // Graph (persistent, could be cached)
    csr_offsets: wgpu::Buffer,
    csr_neighbors: wgpu::Buffer,
    vectors: wgpu::Buffer,
    query: wgpu::Buffer,

    // Search state (per-query)
    frontier_a_ids: wgpu::Buffer,
    frontier_a_dists: wgpu::Buffer,
    frontier_b_ids: wgpu::Buffer,
    frontier_b_dists: wgpu::Buffer,
    candidates: wgpu::Buffer,
    candidate_dists: wgpu::Buffer,
    visited: wgpu::Buffer,
    counters: wgpu::Buffer,
    select_counters: wgpu::Buffer,

    // Params
    expand_params: wgpu::Buffer,
    distance_params: wgpu::Buffer,

    // Readback staging
    staging_ids: wgpu::Buffer,
    staging_dists: wgpu::Buffer,

    // Metadata
    ef: usize,
}

impl TraversalBuffers {
    #[allow(clippy::too_many_arguments)]
    fn new(
        device: &wgpu::Device,
        csr: &CsrGraph,
        vectors_flat: &[f32],
        query: &[f32],
        entry_node: usize,
        ef: usize,
        dimension: usize,
    ) -> Self {
        // --- Graph buffers (could be cached) ---
        let csr_offsets = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CSR Offsets"),
            contents: bytemuck::cast_slice(&csr.offsets),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let csr_neighbors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CSR Neighbors"),
            contents: bytemuck::cast_slice(&csr.neighbors),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let vectors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vectors"),
            contents: bytemuck::cast_slice(vectors_flat),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let query_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Query"),
            contents: bytemuck::cast_slice(query),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // --- Visited bitset: zero-initialize and pre-mark entry node ---
        let visited_words = (csr.num_nodes as usize).div_ceil(32);
        let mut visited_data = vec![0u32; visited_words.max(1)];
        // Pre-mark entry node as visited to avoid redundant re-expansion
        #[allow(clippy::cast_possible_truncation)]
        {
            let entry_u32 = entry_node as u32;
            if (entry_u32 as usize) < csr.num_nodes as usize {
                let word_idx = (entry_u32 / 32) as usize;
                let bit_idx = entry_u32 % 32;
                visited_data[word_idx] |= 1u32 << bit_idx;
            }
        }

        // --- Frontier (double-buffered) ---
        let frontier_buf_size = (ef * std::mem::size_of::<u32>()) as u64;
        let frontier_dists_size = (ef * std::mem::size_of::<f32>()) as u64;

        // Initialize frontier A with the entry node
        let mut initial_frontier = vec![u32::MAX; ef];
        #[allow(clippy::cast_possible_truncation)]
        {
            initial_frontier[0] = entry_node as u32;
        }

        let frontier_a_ids = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Frontier A IDs"),
            contents: bytemuck::cast_slice(&initial_frontier),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });

        let mut initial_dists = vec![f32::MAX; ef];
        initial_dists[0] = 0.0; // Entry point distance (will be recomputed)

        let frontier_a_dists = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Frontier A Dists"),
            contents: bytemuck::cast_slice(&initial_dists),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });

        let frontier_b_ids = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frontier B IDs"),
            size: frontier_buf_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let frontier_b_dists = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frontier B Dists"),
            size: frontier_dists_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Candidates ---
        let max_cand = MAX_CANDIDATES_PER_ITER as usize;
        let candidates = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Candidates"),
            size: (max_cand * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let candidate_dists = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Candidate Dists"),
            size: (max_cand * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Visited bitset: upload pre-initialized data (entry node pre-marked) ---
        let visited = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Visited Bitset"),
            contents: bytemuck::cast_slice(&visited_data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // --- Counters ---
        let counters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Expand Counters"),
            // 4 bytes for candidate_count atomic
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let select_counters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Select Counters"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Params ---
        // BUG FIX: num_frontier must be `ef`, not `1`.
        // After the first select pass, the frontier contains up to `ef` valid
        // entries. Setting num_frontier=1 caused the shader to only process
        // the first frontier node on every iteration, severely limiting recall.
        // Sentinel entries (u32::MAX) are safely skipped by the shader's
        // `if (node >= params.num_nodes) { return; }` guard.
        #[allow(clippy::cast_possible_truncation)]
        let expand_params_data = [
            ef as u32,                     // num_frontier (full beam width)
            MAX_CANDIDATES_PER_ITER,       // max_candidates
            csr.num_nodes,                 // num_nodes
            0u32,                          // padding
        ];
        let expand_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Expand Params"),
            contents: bytemuck::cast_slice(&expand_params_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        #[allow(clippy::cast_possible_truncation)]
        let distance_params_data = [dimension as u32, MAX_CANDIDATES_PER_ITER];
        let distance_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Distance Params"),
            contents: bytemuck::cast_slice(&distance_params_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // --- Staging for readback ---
        let staging_ids = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging IDs"),
            size: frontier_buf_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging_dists = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Dists"),
            size: frontier_dists_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            csr_offsets,
            csr_neighbors,
            vectors,
            query: query_buf,
            frontier_a_ids,
            frontier_a_dists,
            frontier_b_ids,
            frontier_b_dists,
            candidates,
            candidate_dists,
            visited,
            counters,
            select_counters,
            expand_params,
            distance_params,
            staging_ids,
            staging_dists,
            ef,
        }
    }

    fn create_expand_bind_group(
        &self,
        device: &wgpu::Device,
        pipeline: &wgpu::ComputePipeline,
    ) -> wgpu::BindGroup {
        let layout = pipeline.get_bind_group_layout(0);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Expand BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.csr_offsets.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.csr_neighbors.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.frontier_a_ids.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.candidates.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.visited.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.counters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.expand_params.as_entire_binding() },
            ],
        })
    }

    /// Creates bind group for traversal distance pass.
    ///
    /// 5 bindings: query, vectors, candidate_ids, results, params.
    /// The candidate_ids buffer provides indirection from the expand pass —
    /// each thread uses candidate_ids[idx] to look up the actual vector.
    fn create_distance_bind_group(
        &self,
        device: &wgpu::Device,
        pipeline: &wgpu::ComputePipeline,
    ) -> wgpu::BindGroup {
        let layout = pipeline.get_bind_group_layout(0);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Distance BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.query.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.vectors.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.candidates.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.candidate_dists.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.distance_params.as_entire_binding() },
            ],
        })
    }

    fn create_select_bind_group(
        &self,
        device: &wgpu::Device,
        pipeline: &wgpu::ComputePipeline,
        ef: usize,
    ) -> wgpu::BindGroup {
        let layout = pipeline.get_bind_group_layout(0);

        #[allow(clippy::cast_possible_truncation)]
        let select_params_data = [
            MAX_CANDIDATES_PER_ITER,  // num_candidates
            ef as u32,                // k (beam width)
            0u32,                     // padding
            0u32,                     // padding
        ];
        let select_params_buf =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Select Params"),
                contents: bytemuck::cast_slice(&select_params_data),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Select BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.candidates.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.candidate_dists.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.frontier_b_ids.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.frontier_b_dists.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.select_counters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: select_params_buf.as_entire_binding() },
            ],
        })
    }
}

// =============================================================================
// Helper functions for bind group layout entries
// =============================================================================

const fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_traverse_gpu_threshold() {
        assert!(!should_traverse_gpu(0));
        assert!(!should_traverse_gpu(100_000));
        assert!(!should_traverse_gpu(500_000));
        assert!(should_traverse_gpu(500_001));
        assert!(should_traverse_gpu(1_000_000));
    }

    #[test]
    fn test_gpu_traversal_context_new_no_panic() {
        // GpuTraversalContext::new() should not panic even without GPU
        let _ctx = GpuTraversalContext::new();
        // May return None if no GPU — that's fine
    }

    #[test]
    fn test_search_empty_csr_returns_empty() {
        if let Some(ctx) = GpuTraversalContext::new() {
            let csr = CsrGraph {
                offsets: vec![0],
                neighbors: vec![],
                num_nodes: 0,
                max_degree: 0,
                total_edges: 0,
            };
            let result = ctx.search_layer0(
                &csr,
                &[],
                &[1.0, 0.0, 0.0],
                0,
                10,
                64,
                3,
                crate::distance::DistanceMetric::Cosine,
            );
            assert!(result.is_empty());
        }
    }

    #[test]
    fn test_search_unsupported_metric_returns_empty() {
        if let Some(ctx) = GpuTraversalContext::new() {
            let csr = CsrGraph {
                offsets: vec![0, 1],
                neighbors: vec![0],
                num_nodes: 1,
                max_degree: 1,
                total_edges: 1,
            };
            // Hamming has no GPU shader
            let result = ctx.search_layer0(
                &csr,
                &[1.0, 0.0, 0.0],
                &[1.0, 0.0, 0.0],
                0,
                10,
                64,
                3,
                crate::distance::DistanceMetric::Hamming,
            );
            assert!(result.is_empty());
        }
    }
}
