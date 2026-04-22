//! WGSL compute shaders for GPU-accelerated vector operations.
//!
//! Each shader operates on a flat array of vectors and a single query vector,
//! computing distances/similarities in parallel across workgroups of 256 threads.
//!
//! Doc comments in this file describe WGSL bindings, uniforms, buffer layouts
//! and shader-internal variables that deliberately aren't Rust items. Allowing
//! `clippy::doc_markdown` here avoids having to backtick every binding name
//! and WGSL identifier — they're not linkable Rust paths anyway.

#![allow(clippy::doc_markdown)]

/// WGSL compute shader for batch cosine similarity.
pub(crate) const COSINE_SHADER: &str = r"
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

/// WGSL compute shader for batch Euclidean distance.
pub(crate) const EUCLIDEAN_SHADER: &str = r"
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

/// WGSL compute shader for PQ k-means assignment.
///
/// For each vector, finds the nearest centroid by L2 distance.
pub(crate) const PQ_KMEANS_ASSIGN_SHADER: &str = r"
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

/// WGSL compute shader for batch dot product.
pub(crate) const DOT_PRODUCT_SHADER: &str = r"
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

// =============================================================================
// GPU HNSW Traversal Shaders (Issue #502)
// =============================================================================

/// WGSL compute shader for frontier neighbor expansion (SONG Stage 1).
///
/// Each thread processes one frontier node: reads its CSR neighbor list,
/// atomically tests-and-sets the visited bitset, and appends unvisited
/// neighbors to the candidates buffer.
///
/// Bind group layout (7 bindings — uses custom layout, NOT quad):
/// - binding 0: `storage(read)` — CSR offsets (N+1 u32s)
/// - binding 1: `storage(read)` — CSR neighbors (total_edges u32s)
/// - binding 2: `storage(read)` — frontier input (current frontier node IDs)
/// - binding 3: `storage(read_write)` — candidates output (new candidate node IDs)
/// - binding 4: `storage(read_write)` — visited bitset (atomic u32 words)
/// - binding 5: `storage(read_write)` — counters (atomic: [0]=candidate_count)
/// - binding 6: `uniform` — params
pub(crate) const EXPAND_FRONTIER_SHADER: &str = r"
struct ExpandParams {
    num_frontier: u32,
    max_candidates: u32,
    num_nodes: u32,
    _padding: u32,
}

@group(0) @binding(0) var<storage, read> csr_offsets: array<u32>;
@group(0) @binding(1) var<storage, read> csr_neighbors: array<u32>;
@group(0) @binding(2) var<storage, read> frontier: array<u32>;
@group(0) @binding(3) var<storage, read_write> candidates: array<u32>;
@group(0) @binding(4) var<storage, read_write> visited: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> counters: array<atomic<u32>>;
@group(0) @binding(6) var<uniform> params: ExpandParams;

@compute @workgroup_size(256)
fn expand_frontier(@builtin(global_invocation_id) id: vec3<u32>) {
    let thread_id = id.x;
    if (thread_id >= params.num_frontier) { return; }

    let node = frontier[thread_id];
    if (node >= params.num_nodes) { return; }

    let start = csr_offsets[node];
    let end = csr_offsets[node + 1u];

    for (var i = start; i < end; i = i + 1u) {
        let neighbor = csr_neighbors[i];
        if (neighbor >= params.num_nodes) { continue; }

        // Atomic visited check-and-set (maps to CPU BitVecVisited::insert)
        let word_idx = neighbor / 32u;
        let bit_idx = neighbor % 32u;
        let mask = 1u << bit_idx;

        let old = atomicOr(&visited[word_idx], mask);
        if ((old & mask) == 0u) {
            // Not previously visited — add to candidate list
            let slot = atomicAdd(&counters[0], 1u);
            if (slot < params.max_candidates) {
                candidates[slot] = neighbor;
            }
        }
    }
}
";

/// WGSL compute shader for traversal-specific batch squared Euclidean distance.
///
/// Unlike the generic `BATCH_EUCLIDEAN_SQ_SHADER`, this shader uses **candidate
/// ID indirection**: each thread reads `candidate_ids[idx]` to get the actual
/// node ID, then looks up `vectors[node_id * dim .. (node_id+1) * dim]`.
///
/// This is critical because the expand pass writes arbitrary node IDs (e.g.,
/// 42, 1337, 50000) into the candidates buffer — computing distance for
/// sequential indices 0..N would produce completely wrong results.
///
/// Output matches CPU `CachedSimdDistance::distance()`: squared L2 (lower = closer).
///
/// # u32 offset limit
///
/// The expression `node_id * dim` is computed in `u32` (WGSL has no u64
/// scalar). Callers must ensure `num_vectors * dim <= u32::MAX` — this is
/// enforced by [`crate::gpu::should_traverse_gpu`]. A `10M × 768` index
/// would overflow (~7.68B > 4.29B) and silently return wrong distances.
///
/// Bind group layout (5 bindings — uses custom traversal distance layout):
/// - binding 0: `storage(read)` — query vector
/// - binding 1: `storage(read)` — all vectors (N × dim flat array)
/// - binding 2: `storage(read)` — candidate_ids (node IDs from expand pass)
/// - binding 3: `storage(read_write)` — results (distances, one per candidate)
/// - binding 4: `uniform` — params (dimension, max_candidates)
pub(crate) const TRAVERSAL_EUCLIDEAN_SQ_SHADER: &str = r"
struct Params {
    dimension: u32,
    max_candidates: u32,
}

@group(0) @binding(0) var<storage, read> query: array<f32>;
@group(0) @binding(1) var<storage, read> vectors: array<f32>;
@group(0) @binding(2) var<storage, read> candidate_ids: array<u32>;
@group(0) @binding(3) var<storage, read_write> results: array<f32>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(256)
fn traversal_euclidean_sq(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.max_candidates) { return; }

    let node_id = candidate_ids[idx];
    // Sentinel check: unexpanded slots have u32::MAX
    if (node_id == 0xFFFFFFFFu) {
        results[idx] = 3.4028235e+38;
        return;
    }

    let dim = params.dimension;
    let offset = node_id * dim;
    var sum_sq: f32 = 0.0;

    for (var i: u32 = 0u; i < dim; i = i + 1u) {
        let diff = query[i] - vectors[offset + i];
        sum_sq = sum_sq + diff * diff;
    }

    // Squared L2: lower = closer (matches CPU CachedSimdDistance)
    results[idx] = sum_sq;
}
";

/// WGSL compute shader for traversal-specific batch cosine distance.
///
/// Uses candidate ID indirection (same as `TRAVERSAL_EUCLIDEAN_SQ_SHADER`).
/// Output: `1.0 - cosine_similarity` (lower = closer), matching CPU
/// `CachedSimdDistance::distance()` for Cosine metric.
///
/// Same u32 offset limit as `TRAVERSAL_EUCLIDEAN_SQ_SHADER`: callers must
/// ensure `num_vectors * dim <= u32::MAX` (gated by
/// [`crate::gpu::should_traverse_gpu`]).
pub(crate) const TRAVERSAL_COSINE_SHADER: &str = r"
struct Params {
    dimension: u32,
    max_candidates: u32,
}

@group(0) @binding(0) var<storage, read> query: array<f32>;
@group(0) @binding(1) var<storage, read> vectors: array<f32>;
@group(0) @binding(2) var<storage, read> candidate_ids: array<u32>;
@group(0) @binding(3) var<storage, read_write> results: array<f32>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(256)
fn traversal_cosine(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.max_candidates) { return; }

    let node_id = candidate_ids[idx];
    if (node_id == 0xFFFFFFFFu) {
        results[idx] = 3.4028235e+38;
        return;
    }

    let dim = params.dimension;
    let offset = node_id * dim;
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
        // 1 - similarity: lower = closer (matches CPU CachedSimdDistance)
        results[idx] = 1.0 - (dot / denom);
    } else {
        results[idx] = 1.0;
    }
}
";

/// WGSL compute shader for traversal-specific batch dot-product distance.
///
/// Uses candidate ID indirection (same pattern).
/// Output: `-dot_product` (lower = closer), matching CPU
/// `CachedSimdDistance::distance()` for DotProduct metric.
///
/// Same u32 offset limit as `TRAVERSAL_EUCLIDEAN_SQ_SHADER`: callers must
/// ensure `num_vectors * dim <= u32::MAX` (gated by
/// [`crate::gpu::should_traverse_gpu`]).
pub(crate) const TRAVERSAL_DOT_PRODUCT_SHADER: &str = r"
struct Params {
    dimension: u32,
    max_candidates: u32,
}

@group(0) @binding(0) var<storage, read> query: array<f32>;
@group(0) @binding(1) var<storage, read> vectors: array<f32>;
@group(0) @binding(2) var<storage, read> candidate_ids: array<u32>;
@group(0) @binding(3) var<storage, read_write> results: array<f32>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(256)
fn traversal_dot(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.max_candidates) { return; }

    let node_id = candidate_ids[idx];
    if (node_id == 0xFFFFFFFFu) {
        results[idx] = 3.4028235e+38;
        return;
    }

    let dim = params.dimension;
    let offset = node_id * dim;
    var dot: f32 = 0.0;

    for (var i: u32 = 0u; i < dim; i = i + 1u) {
        dot = dot + query[i] * vectors[offset + i];
    }

    // Negate: lower = closer (matches CPU CachedSimdDistance)
    results[idx] = -dot;
}
";

/// WGSL compute shader for global top-k selection (SONG Stage 3).
///
/// Bind group layout (8 bindings — uses custom layout):
/// - binding 0: `storage(read)` — candidate IDs
/// - binding 1: `storage(read)` — candidate distances
/// - binding 2: `storage(read_write)` — frontier output (top-k node IDs)
/// - binding 3: `storage(read_write)` — frontier distances output
/// - binding 4: `storage(read_write)` — counters (kept for layout compatibility)
/// - binding 5: `uniform` — params
/// - binding 6: `storage(read)` — previous frontier IDs (accumulator seed)
/// - binding 7: `storage(read)` — previous frontier distances
///
/// # Algorithm — serial insertion sort with frontier accumulation
///
/// Invoked once per HNSW traversal iteration. Dispatch is **a single
/// workgroup of a single thread** (host side uses `dispatch_workgroups(1,1,1)`).
///
/// # Why accumulate the previous frontier
///
/// HNSW beam search maintains a working set `W` across iterations:
/// `W_{n+1} = top-k(W_n ∪ new_candidates)`. If the shader only sorted
/// `new_candidates`, the best results from earlier iterations — including
/// 1-hop neighbors of the entry point — would be permanently discarded
/// when the frontier gets replaced, silently destroying recall.
///
/// The fix is to seed the output frontier with the previous frontier's
/// contents (bindings 6/7, copied from `frontier_a_*` by the host) and
/// then insertion-sort the new candidates into the seeded top-k. The
/// caller is responsible for initialising `frontier_a_dists[0]` with the
/// entry node's real query distance (not a sentinel placeholder), so the
/// very first iteration starts with a semantically correct top-k of size 1.
///
/// # Why single-threaded
///
/// A parallel per-workgroup bitonic sort followed by an `atomicAdd` race to
/// fill the global frontier — the original design — is **not correct**: the
/// workgroups that finish first win all `k` frontier slots regardless of
/// distance, so the global top-k is actually the per-workgroup-racing top-k.
/// This silently degrades recall.
///
/// A correct parallel top-k on GPU requires either a multi-pass reduction
/// (per-workgroup local sort → merge) or a shared min-heap with proper
/// serialization. Both are significant additions. For now we choose
/// correctness over speed: one thread does an insertion-sort scan over all
/// `num_candidates` (≤ 8192) keeping the sorted top-k in the frontier.
///
/// Cost: O(`num_candidates` × log `k` + `num_candidates` × `k` in the worst
/// case when every candidate is closer than the current best) ≈ 1.1M ops per
/// iteration at `num_candidates` = 8192 / `k` = 128. Acceptable for the
/// advertised 500K–5M-vector GPU range; a parallel reduction is a future
/// perf win (tracked separately).
pub(crate) const SELECT_TOPK_SHADER: &str = r"
struct SelectParams {
    num_candidates: u32,
    k: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<storage, read> candidate_ids: array<u32>;
@group(0) @binding(1) var<storage, read> candidate_dists: array<f32>;
@group(0) @binding(2) var<storage, read_write> frontier_out: array<u32>;
@group(0) @binding(3) var<storage, read_write> frontier_dists: array<f32>;
@group(0) @binding(4) var<storage, read_write> counters: array<atomic<u32>>;
@group(0) @binding(5) var<uniform> params: SelectParams;
@group(0) @binding(6) var<storage, read> frontier_in_ids: array<u32>;
@group(0) @binding(7) var<storage, read> frontier_in_dists: array<f32>;

@compute @workgroup_size(1)
fn select_topk() {
    let n = params.num_candidates;
    let k = params.k;

    // Seed the output frontier with the previous iteration's top-k so that
    // earlier-seen best results are preserved. Sentinel slots (id == u32::MAX
    // or dist == f32::MAX) survive the seed untouched; real candidates will
    // displace them during the scan. See shader doc for the invariant.
    for (var i: u32 = 0u; i < k; i = i + 1u) {
        frontier_out[i] = frontier_in_ids[i];
        frontier_dists[i] = frontier_in_dists[i];
    }

    // Insertion sort scan: maintain a sorted ascending top-k in the frontier.
    for (var i: u32 = 0u; i < n; i = i + 1u) {
        let id = candidate_ids[i];
        if (id == 0xFFFFFFFFu) { continue; }
        let d = candidate_dists[i];

        // Worse than the current k-th smallest — nothing to do.
        if (d >= frontier_dists[k - 1u]) { continue; }

        // Skip if this node is already in the seeded frontier (happens when
        // a previous iteration's top-k node is re-discovered as a candidate).
        // The visited bitset normally prevents this for nodes seen through
        // expand, but the entry node in the first iteration lives only in
        // frontier_a and could re-appear as a neighbour later.
        var duplicate = false;
        for (var j: u32 = 0u; j < k; j = j + 1u) {
            if (frontier_out[j] == id) {
                duplicate = true;
                break;
            }
        }
        if (duplicate) { continue; }

        // Binary search for insertion position in the sorted frontier.
        var lo: u32 = 0u;
        var hi: u32 = k;
        while (lo < hi) {
            let mid = (lo + hi) / 2u;
            if (frontier_dists[mid] > d) { hi = mid; } else { lo = mid + 1u; }
        }
        let pos = lo;

        // Shift right [pos, k-1) → [pos+1, k) to make room.
        var j: u32 = k - 1u;
        while (j > pos) {
            frontier_out[j] = frontier_out[j - 1u];
            frontier_dists[j] = frontier_dists[j - 1u];
            j = j - 1u;
        }
        frontier_out[pos] = id;
        frontier_dists[pos] = d;
    }

    // Counters unused in this implementation but kept in the layout for
    // backward compatibility with the existing bind group.
    atomicStore(&counters[0], k);
}
";
