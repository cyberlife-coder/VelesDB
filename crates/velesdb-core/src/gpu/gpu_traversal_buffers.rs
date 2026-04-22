//! GPU buffer layout and bind-group construction for HNSW traversal.
//!
//! Extracted from `gpu_traversal.rs` to keep the orchestrator file under
//! the project's 500-line-per-file NLOC ceiling. This module owns nothing
//! conceptual beyond buffer lifetime management:
//!
//! * [`TraversalBuffers::new`] allocates every buffer a single query needs
//!   (graph, vectors, frontiers, candidates, params, readback, sentinels).
//! * `create_*_bind_group` helpers wire those buffers to the corresponding
//!   pipelines' WGSL layouts (`expand`, `distance`, `select`).
//!
//! The constant [`MAX_CANDIDATES_PER_ITER`] lives here because it is
//! primarily a buffer-sizing decision; the orchestrator re-exports it via
//! `super::MAX_CANDIDATES_PER_ITER` for dispatch computations.

// Doc comments in this module mix WGSL binding names and HNSW-paper
// terminology that aren't Rust items; backticking each one would hurt
// readability without adding linkable references.
#![allow(clippy::doc_markdown)]
// `TraversalBuffers::new` intentionally inlines every GPU buffer allocation
// in the WGSL-binding order so the code matches the shader layouts 1:1.
// Splitting by "kind of buffer" would reduce the line count but break
// that correspondence.
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]

use wgpu::util::DeviceExt;

use super::gpu_csr::CsrGraph;

/// Maximum number of candidates that can be generated per iteration.
///
/// Bounded by ef_search × max_degree. For ef=128 and M0=32, this is 4096.
/// We allocate for the worst case to avoid dynamic GPU buffer resizing.
pub(super) const MAX_CANDIDATES_PER_ITER: u32 = 8192;

/// All GPU buffers needed for a single traversal execution.
pub(super) struct TraversalBuffers {
    // Graph (persistent, could be cached)
    pub(super) csr_offsets: wgpu::Buffer,
    pub(super) csr_neighbors: wgpu::Buffer,
    pub(super) vectors: wgpu::Buffer,
    pub(super) query: wgpu::Buffer,

    // Search state (per-query)
    pub(super) frontier_a_ids: wgpu::Buffer,
    pub(super) frontier_a_dists: wgpu::Buffer,
    pub(super) frontier_b_ids: wgpu::Buffer,
    pub(super) frontier_b_dists: wgpu::Buffer,
    pub(super) candidates: wgpu::Buffer,
    pub(super) candidate_dists: wgpu::Buffer,
    pub(super) visited: wgpu::Buffer,
    pub(super) counters: wgpu::Buffer,
    pub(super) select_counters: wgpu::Buffer,

    // Params
    pub(super) expand_params: wgpu::Buffer,
    pub(super) distance_params: wgpu::Buffer,

    // Readback staging
    pub(super) staging_ids: wgpu::Buffer,
    pub(super) staging_dists: wgpu::Buffer,
    // Sentinel buffer for clearing candidates to u32::MAX each iteration
    pub(super) candidates_sentinel: wgpu::Buffer,
    pub(super) candidates_byte_size: usize,
    // Sentinel buffers for frontier_b initialization
    pub(super) frontier_ids_sentinel: wgpu::Buffer,
    pub(super) frontier_dists_sentinel: wgpu::Buffer,

    // Metadata
    pub(super) ef: usize,
}

impl TraversalBuffers {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        device: &wgpu::Device,
        csr: &CsrGraph,
        vectors_flat: &[f32],
        query: &[f32],
        entry_node: usize,
        entry_distance: f32,
        ef: usize,
        dimension: usize,
    ) -> Self {
        // Defense in depth: the traversal distance shaders compute
        // `node_id * dim` in u32 (WGSL has no u64 scalar). The public
        // entry point `should_traverse_gpu` already gates this, but assert
        // it here too so any future caller that bypasses the helper fails
        // loudly in debug builds instead of silently returning wrong results.
        debug_assert!(
            (csr.num_nodes as usize)
                .checked_mul(dimension)
                .is_some_and(|p| u32::try_from(p).is_ok()),
            "GPU traversal requires num_nodes * dimension <= u32::MAX \
             (got {} * {}); use should_traverse_gpu() to gate the caller",
            csr.num_nodes,
            dimension,
        );

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

        // Seed frontier A with the entry node at its *real* query distance
        // (computed by the caller in the same space the GPU distance shaders
        // produce). Remaining slots stay sentinels — the SELECT shader treats
        // `f32::MAX` as "nothing here, displace me with any real candidate".
        let mut initial_dists = vec![f32::MAX; ef];
        initial_dists[0] = entry_distance;

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
        let candidates_byte_size = max_cand * std::mem::size_of::<u32>();
        let candidates = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Candidates"),
            size: candidates_byte_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Sentinel buffer: pre-filled with u32::MAX (0xFFFFFFFF).
        // Copied into candidates buffer before each expand pass to ensure
        // unused slots are sentinels, not node 0 (WebGPU zero-init default).
        let sentinel_data = vec![u32::MAX; max_cand];
        let candidates_sentinel = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Candidates Sentinel"),
            contents: bytemuck::cast_slice(&sentinel_data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
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
            ef as u32,               // num_frontier (full beam width)
            MAX_CANDIDATES_PER_ITER, // max_candidates
            csr.num_nodes,           // num_nodes
            0u32,                    // padding
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

        // --- Frontier sentinel buffers ---
        // Pre-filled with sentinels and used to re-initialize frontier_b
        // before each select pass (prevents stale entries from prior iterations).
        let frontier_ids_sentinel_data = vec![u32::MAX; ef];
        let frontier_ids_sentinel = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Frontier IDs Sentinel"),
            contents: bytemuck::cast_slice(&frontier_ids_sentinel_data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let frontier_dists_sentinel_data = vec![f32::MAX; ef];
        let frontier_dists_sentinel =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Frontier Dists Sentinel"),
                contents: bytemuck::cast_slice(&frontier_dists_sentinel_data),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
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
            candidates_sentinel,
            candidates_byte_size,
            frontier_ids_sentinel,
            frontier_dists_sentinel,
            ef,
        }
    }

    pub(super) fn create_expand_bind_group(
        &self,
        device: &wgpu::Device,
        pipeline: &wgpu::ComputePipeline,
    ) -> wgpu::BindGroup {
        let layout = pipeline.get_bind_group_layout(0);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Expand BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.csr_offsets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.csr_neighbors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.frontier_a_ids.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.candidates.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.visited.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.counters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.expand_params.as_entire_binding(),
                },
            ],
        })
    }

    /// Creates bind group for traversal distance pass.
    ///
    /// 5 bindings: query, vectors, candidate_ids, results, params.
    /// The candidate_ids buffer provides indirection from the expand pass —
    /// each thread uses candidate_ids[idx] to look up the actual vector.
    pub(super) fn create_distance_bind_group(
        &self,
        device: &wgpu::Device,
        pipeline: &wgpu::ComputePipeline,
    ) -> wgpu::BindGroup {
        let layout = pipeline.get_bind_group_layout(0);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Distance BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.query.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.vectors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.candidates.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.candidate_dists.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.distance_params.as_entire_binding(),
                },
            ],
        })
    }

    pub(super) fn create_select_bind_group(
        &self,
        device: &wgpu::Device,
        pipeline: &wgpu::ComputePipeline,
        ef: usize,
    ) -> wgpu::BindGroup {
        let layout = pipeline.get_bind_group_layout(0);

        #[allow(clippy::cast_possible_truncation)]
        let select_params_data = [
            MAX_CANDIDATES_PER_ITER, // num_candidates
            ef as u32,               // k (beam width)
            0u32,                    // padding
            0u32,                    // padding
        ];
        let select_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Select Params"),
            contents: bytemuck::cast_slice(&select_params_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Select BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.candidates.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.candidate_dists.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.frontier_b_ids.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.frontier_b_dists.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.select_counters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: select_params_buf.as_entire_binding(),
                },
                // Accumulator seed: previous iteration's frontier. See the
                // SELECT_TOPK_SHADER doc comment for the HNSW invariant this
                // preserves. `frontier_a_*` is the input side of the
                // ping-pong pair; `frontier_b_*` bound at 2/3 is the output.
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.frontier_a_ids.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.frontier_a_dists.as_entire_binding(),
                },
            ],
        })
    }
}
