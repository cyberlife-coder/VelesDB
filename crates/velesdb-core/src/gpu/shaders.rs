//! WGSL compute shaders for GPU-accelerated vector operations.
//!
//! Each shader operates on a flat array of vectors and a single query vector,
//! computing distances/similarities in parallel across workgroups of 256 threads.

/// WGSL compute shader for batch cosine similarity.
pub(super) const COSINE_SHADER: &str = r"
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

/// WGSL compute shader for batch Euclidean distance (ready for GPU pipeline).
#[allow(dead_code)]
pub(super) const EUCLIDEAN_SHADER: &str = r"
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

/// WGSL compute shader for batch dot product (P2-GPU-2).
#[allow(dead_code)]
pub(super) const DOT_PRODUCT_SHADER: &str = r"
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
