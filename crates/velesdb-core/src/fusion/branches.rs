//! The shape of a multi-vector fusion's input, shared by every engine that
//! fuses (`Collection::multi_query_search`, the WASM `NEAR_FUSED` executor):
//! how many query vectors one fusion takes, and how deep each per-vector
//! ranking (a *branch*) goes before the branches are fused.

/// The most query vectors one fusion accepts.
pub const MAX_FUSED_VECTORS: usize = 10;

/// How many of a branch's best points enter the fusion when `top_k` fused
/// results are wanted: a branch is overfetched, so a point ranked just past
/// `top_k` by one query vector but near the top by another still counts.
#[must_use]
pub fn branch_depth(top_k: usize) -> usize {
    match top_k {
        0..=10 => top_k * 20,
        11..=50 => top_k * 10,
        51..=100 => top_k * 5,
        _ => top_k.saturating_mul(2),
    }
}
