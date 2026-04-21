//! WGSL compute shaders for GPU-accelerated vector operations.
//!
//! Each shader operates on a flat array of vectors and a single query vector,
//! computing distances/similarities in parallel across workgroups of 256 threads.

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

/// WGSL compute shader for batch squared Euclidean distance (no sqrt).
///
/// Matches `CachedSimdDistance::distance()` for Euclidean metric, which
/// returns squared L2 (no sqrt) in the HNSW traversal hot loop. The sqrt
/// is deferred to `transform_score()` applied only to the final k results.
///
/// Uses the standard quad bind-group layout (same as cosine/euclidean/dot).
pub(crate) const BATCH_EUCLIDEAN_SQ_SHADER: &str = r"
struct Params {
    dimension: u32,
    num_vectors: u32,
}

@group(0) @binding(0) var<storage, read> query: array<f32>;
@group(0) @binding(1) var<storage, read> vectors: array<f32>;
@group(0) @binding(2) var<storage, read_write> results: array<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(256)
fn batch_euclidean_sq(@builtin(global_invocation_id) id: vec3<u32>) {
    let idx = id.x;
    if (idx >= params.num_vectors) { return; }

    let dim = params.dimension;
    let offset = idx * dim;
    var sum_sq: f32 = 0.0;

    for (var i: u32 = 0u; i < dim; i = i + 1u) {
        let diff = query[i] - vectors[offset + i];
        sum_sq = sum_sq + diff * diff;
    }

    results[idx] = sum_sq;
}
";

/// WGSL compute shader for parallel top-k selection (SONG Stage 3).
///
/// Reads candidate node IDs and their precomputed distances, selects
/// the best `k` candidates by distance, and outputs them as the new
/// frontier for the next iteration.
///
/// Uses a simple serial scan per workgroup — sufficient because the
/// candidate count per iteration is bounded by ef_search * max_degree
/// which is typically < 10K elements.
///
/// Bind group layout (5 bindings — uses custom layout):
/// - binding 0: `storage(read)` — candidate IDs
/// - binding 1: `storage(read)` — candidate distances
/// - binding 2: `storage(read_write)` — frontier output (top-k node IDs)
/// - binding 3: `storage(read_write)` — frontier distances output
/// - binding 4: `storage(read_write)` — counters (atomic: [0]=frontier_size)
/// - binding 5: `uniform` — params
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

// Workgroup-local arrays for sorting
var<workgroup> local_ids: array<u32, 256>;
var<workgroup> local_dists: array<f32, 256>;

@compute @workgroup_size(256)
fn select_topk(@builtin(local_invocation_id) lid: vec3<u32>,
               @builtin(workgroup_id) wid: vec3<u32>) {
    let global_idx = wid.x * 256u + lid.x;

    // Load candidates into workgroup-local memory
    if (global_idx < params.num_candidates) {
        local_ids[lid.x] = candidate_ids[global_idx];
        local_dists[lid.x] = candidate_dists[global_idx];
    } else {
        local_ids[lid.x] = 0xFFFFFFFFu;
        local_dists[lid.x] = 3.4028235e+38;
    }
    workgroupBarrier();

    // Bitonic sort within workgroup
    for (var size = 2u; size <= 256u; size = size * 2u) {
        for (var stride = size / 2u; stride > 0u; stride = stride / 2u) {
            let pos = lid.x;
            let partner = pos ^ stride;
            if (partner > pos && partner < 256u) {
                let ascending = ((pos & size) == 0u);
                if ((ascending && local_dists[pos] > local_dists[partner]) ||
                    (!ascending && local_dists[pos] < local_dists[partner])) {
                    // Swap
                    let tmp_id = local_ids[pos];
                    let tmp_d = local_dists[pos];
                    local_ids[pos] = local_ids[partner];
                    local_dists[pos] = local_dists[partner];
                    local_ids[partner] = tmp_id;
                    local_dists[partner] = tmp_d;
                }
            }
            workgroupBarrier();
        }
    }

    // Write the top-k results from this workgroup to global frontier
    if (lid.x < params.k && local_dists[lid.x] < 3.4028235e+38) {
        let slot = atomicAdd(&counters[0], 1u);
        if (slot < params.k) {
            frontier_out[slot] = local_ids[lid.x];
            frontier_dists[slot] = local_dists[lid.x];
        }
    }
}
";
