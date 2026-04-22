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

#![allow(clippy::similar_names)]
// Doc comments in this module mix WGSL binding names, shader variables,
// and HNSW-paper terminology that aren't Rust items; backticking every
// one would hurt readability without adding linkable references.
#![allow(clippy::doc_markdown)]
// The TraversalBuffers::new constructor intentionally inlines every GPU
// buffer allocation for a single query's lifecycle. Splitting would
// obscure the ordering relative to the WGSL bind group layout; tracked
// as a refactor for a follow-up PR.
#![allow(clippy::too_many_lines)]

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use super::gpu_backend::GpuAccelerator;
use super::gpu_csr::CsrGraph;
use super::gpu_traversal_buffers::{TraversalBuffers, MAX_CANDIDATES_PER_ITER};
use super::gpu_traversal_pipelines as pipelines;

/// Computes the distance between `query` and `entry_vec` in the same
/// "lower = better" space that the traversal shaders produce.
///
/// This matches the shader conventions exactly so that an entry-node
/// distance computed on CPU can be directly merged with GPU-computed
/// candidate distances during the top-k selection:
/// * Cosine → `1.0 - cosine_similarity`
/// * Euclidean → squared L2 (not `sqrt`-reduced), matching
///   `TRAVERSAL_EUCLIDEAN_SQ_SHADER`
/// * DotProduct → `-dot_product`
///
/// Returns `f32::MAX` for metrics that have no GPU shader (caller must
/// have already bailed out via `should_traverse_gpu` or metric dispatch).
#[must_use]
fn gpu_distance_cpu_fallback(
    query: &[f32],
    entry_vec: &[f32],
    metric: crate::distance::DistanceMetric,
) -> f32 {
    use crate::distance::DistanceMetric;
    debug_assert_eq!(query.len(), entry_vec.len());
    match metric {
        DistanceMetric::Cosine => {
            let (mut dot, mut na, mut nb) = (0.0_f32, 0.0_f32, 0.0_f32);
            for (x, y) in query.iter().zip(entry_vec.iter()) {
                dot += x * y;
                na += x * x;
                nb += y * y;
            }
            let denom = (na * nb).sqrt();
            if denom == 0.0 {
                1.0
            } else {
                1.0 - (dot / denom)
            }
        }
        DistanceMetric::Euclidean => query
            .iter()
            .zip(entry_vec.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>(),
        DistanceMetric::DotProduct => -query
            .iter()
            .zip(entry_vec.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>(),
        DistanceMetric::Hamming | DistanceMetric::Jaccard => f32::MAX,
    }
}

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

/// Returns `true` if GPU graph traversal is safe and likely faster than CPU
/// for the given index size and dimension.
///
/// Two gates are checked:
/// * **Performance gate** — `num_vectors > 500_000`. Below this, CPU SIMD with
///   prefetch and rayon parallelism is faster than GPU due to dispatch overhead.
/// * **Correctness gate** — `num_vectors * dimension <= u32::MAX`. The distance
///   shaders (`TRAVERSAL_EUCLIDEAN_SQ_SHADER`, `TRAVERSAL_COSINE_SHADER`,
///   `TRAVERSAL_DOT_PRODUCT_SHADER`) compute vector offsets as `node_id * dim`
///   in `u32`; a `10M × 768` index would overflow (~7.68B > 4.29B) and silently
///   return wrong distances. WGSL has no u64 scalar, so the caller must bail
///   out to CPU in this regime.
///
/// Returns `false` (fall back to CPU) when either gate is not satisfied.
#[must_use]
pub fn should_traverse_gpu(num_vectors: usize, dimension: usize) -> bool {
    if num_vectors <= 500_000 {
        return false;
    }
    // Correctness gate: keep `node_id * dim` within u32 range.
    num_vectors
        .checked_mul(dimension)
        .is_some_and(|prod| u32::try_from(prod).is_ok())
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
/// Created once via [`global()`](Self::global) and shared across all queries.
/// Holds the three traversal pipelines (expand, distance, select)
/// and references to the shared GPU device.
pub struct GpuTraversalContext {
    gpu: Arc<GpuAccelerator>,
    expand_pipeline: wgpu::ComputePipeline,
    distance_cosine_pipeline: wgpu::ComputePipeline,
    distance_euclidean_sq_pipeline: wgpu::ComputePipeline,
    distance_dot_pipeline: wgpu::ComputePipeline,
    select_pipeline: wgpu::ComputePipeline,
}

impl GpuTraversalContext {
    /// Returns the global singleton `GpuTraversalContext`, creating it on
    /// first call. Pipelines are compiled once and reused across all queries.
    ///
    /// Returns `None` if no GPU is available.
    #[must_use]
    pub fn global() -> Option<Arc<Self>> {
        static INSTANCE: OnceLock<Option<Arc<GpuTraversalContext>>> = OnceLock::new();
        INSTANCE
            .get_or_init(|| GpuTraversalContext::new().map(Arc::new))
            .clone()
    }

    /// Creates a new GPU traversal context, compiling all required pipelines.
    ///
    /// Prefer [`global()`](Self::global) for production use to avoid
    /// recompiling pipelines per query. Use `new()` only in benchmarks
    /// or tests that need isolated contexts.
    ///
    /// Returns `None` if no GPU is available.
    #[must_use]
    pub fn new() -> Option<Self> {
        let gpu = GpuAccelerator::global()?;
        let device = gpu.device();

        let expand_pipeline = pipelines::compile_expand_pipeline(device);
        let distance_cosine_pipeline = pipelines::compile_traversal_distance_pipeline(
            device,
            super::gpu_backend::shaders::TRAVERSAL_COSINE_SHADER,
            "traversal_cosine",
            "Traversal Cosine",
        );
        let distance_euclidean_sq_pipeline = pipelines::compile_traversal_distance_pipeline(
            device,
            super::gpu_backend::shaders::TRAVERSAL_EUCLIDEAN_SQ_SHADER,
            "traversal_euclidean_sq",
            "Traversal Euclidean Sq",
        );
        let distance_dot_pipeline = pipelines::compile_traversal_distance_pipeline(
            device,
            super::gpu_backend::shaders::TRAVERSAL_DOT_PRODUCT_SHADER,
            "traversal_dot",
            "Traversal Dot Product",
        );
        let select_pipeline = pipelines::compile_select_pipeline(device);

        Some(Self {
            gpu,
            expand_pipeline,
            distance_cosine_pipeline,
            distance_euclidean_sq_pipeline,
            distance_dot_pipeline,
            select_pipeline,
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
        if let Some(results) = self.search_layer0_inner(
            csr,
            vectors_flat,
            query,
            entry_node,
            k,
            ef_search,
            dimension,
            metric,
        ) {
            let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;
            tracing::debug!(
                k,
                ef_search,
                num_results = results.len(),
                total_ms = format!("{total_ms:.2}"),
                "GPU traversal completed"
            );
            results
        } else {
            tracing::warn!("GPU traversal failed, returning empty results for CPU fallback");
            Vec::new()
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

        // Compute the entry node's real query distance in the same space
        // the GPU shaders use. The SELECT shader seeds its output frontier
        // from `frontier_a`, and `frontier_a[0]` represents the entry node;
        // a sentinel/0.0 placeholder here would silently force the entry to
        // pollute every top-k with an artificial minimum.
        let entry_offset = entry_node.checked_mul(dimension).filter(|end| {
            end.checked_add(dimension)
                .is_some_and(|e| e <= vectors_flat.len())
        })?;
        let entry_vec = &vectors_flat[entry_offset..entry_offset + dimension];
        let entry_distance = gpu_distance_cpu_fallback(query, entry_vec, metric);

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
            entry_distance,
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
        let result_ids =
            super::helpers::readback_buffer::<u32>(device, &buffers.staging_ids, result_count)?;
        let result_dists =
            super::helpers::readback_buffer::<f32>(device, &buffers.staging_dists, result_count)?;

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
    // Pass encoding
    // =========================================================================

    fn encode_expand_pass(&self, encoder: &mut wgpu::CommandEncoder, buffers: &TraversalBuffers) {
        // Reset candidate counter to 0 before expansion
        encoder.clear_buffer(&buffers.counters, 0, None);

        // Fill candidates buffer with sentinels (u32::MAX = 0xFFFFFFFF) by
        // copying from `candidates_sentinel` (initialized once at buffer
        // creation to all u32::MAX). Without this, unused slots contain 0
        // — a valid node ID — causing the distance shader to compute
        // distances against node 0 for all ~8000 unused slots.
        // The sentinels are detected by the TRAVERSAL_*_SHADER's
        // `if (node_id == 0xFFFFFFFFu)` guard and assigned f32::MAX.
        //
        // Do NOT `clear_buffer(&candidates_sentinel, …)` here: `clear_buffer`
        // fills with 0x00, which would destroy the u32::MAX pattern in
        // the read-only source buffer and poison every subsequent copy.
        encoder.copy_buffer_to_buffer(
            &buffers.candidates_sentinel,
            0,
            &buffers.candidates,
            0,
            buffers.candidates_byte_size as u64,
        );

        let bind_group = buffers.create_expand_bind_group(self.gpu.device(), &self.expand_pipeline);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Expand Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.expand_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        // Fix: dispatch based on ef (full beam width), not frontier_size.
        // When ef > 256, we need multiple workgroups to expand all frontier nodes.
        #[allow(clippy::cast_possible_truncation)]
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

        // Initialize frontier_b with sentinels so slots not filled by
        // the select shader remain as "empty" (u32::MAX / f32::MAX).
        // This prevents stale frontier entries from previous iterations
        // contaminating the next expand pass.
        let frontier_bytes = (ef * std::mem::size_of::<u32>()) as u64;
        encoder.copy_buffer_to_buffer(
            &buffers.frontier_ids_sentinel,
            0,
            &buffers.frontier_b_ids,
            0,
            frontier_bytes,
        );
        encoder.copy_buffer_to_buffer(
            &buffers.frontier_dists_sentinel,
            0,
            &buffers.frontier_b_dists,
            0,
            frontier_bytes, // same size: ef × f32
        );

        let bind_group =
            buffers.create_select_bind_group(self.gpu.device(), &self.select_pipeline, ef);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Select Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.select_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        // SELECT_TOPK_SHADER is `@workgroup_size(1)` and does a serial
        // insertion-sort over all candidates. Dispatching >1 workgroup
        // would race on the frontier writes — see shader doc comment.
        pass.dispatch_workgroups(1, 1, 1);

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
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_traverse_gpu_threshold() {
        // Performance gate (num_vectors > 500_000) — dimension 128 is always safe.
        assert!(!should_traverse_gpu(0, 128));
        assert!(!should_traverse_gpu(100_000, 128));
        assert!(!should_traverse_gpu(500_000, 128));
        assert!(should_traverse_gpu(500_001, 128));
        assert!(should_traverse_gpu(1_000_000, 128));
    }

    #[test]
    fn test_should_traverse_gpu_u32_offset_correctness_gate() {
        // 10M * 768 = 7_680_000_000 > u32::MAX (4_294_967_295) — must bail out to CPU.
        assert!(!should_traverse_gpu(10_000_000, 768));
        // 5M * 768 = 3_840_000_000 < u32::MAX — safe.
        assert!(should_traverse_gpu(5_000_000, 768));
        // Boundary: exactly u32::MAX offsets — safe.
        assert!(should_traverse_gpu((u32::MAX as usize) / 128, 128));
        // Overflow even after checked_mul — must bail out.
        assert!(!should_traverse_gpu(usize::MAX / 2, 4));
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
