//! The shape of a multi-vector fusion's input, shared by every engine that
//! fuses (`Collection::multi_query_search`, the WASM `NEAR_FUSED` executor):
//! how many query vectors one fusion takes, and how deep each per-vector
//! ranking (a *branch*) goes before the branches are fused.

/// The most query vectors one fusion accepts.
pub const MAX_FUSED_VECTORS: usize = 10;

/// Refuses a fusion of `count` query vectors that is not between one and
/// [`MAX_FUSED_VECTORS`].
///
/// # Errors
///
/// [`Error::Config`](crate::Error::Config) (`VELES-009`) for no vector, or
/// more than [`MAX_FUSED_VECTORS`].
pub fn validate_fused_vector_count(count: usize) -> crate::error::Result<()> {
    if count == 0 {
        return Err(crate::Error::Config(
            "multi_query_search requires at least one vector".into(),
        ));
    }
    if count > MAX_FUSED_VECTORS {
        return Err(crate::Error::Config(format!(
            "multi_query_search supports at most {MAX_FUSED_VECTORS} vectors, got {count}"
        )));
    }
    Ok(())
}

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

/// How many of each branch's best points enter a hybrid dense + sparse
/// fusion when `limit` fused results are wanted: twice the limit, and at
/// least ten more than it.
///
/// Both the dense and the sparse branch can independently miss the globally
/// optimal result; fetching more candidates from each compensates for the
/// blind spots of the other. Doubling is the conservative end, and a filter
/// does not deepen it: each branch admits only the rows the filter matches
/// (core's sparse branch, `Collection::ranked_rows`), so this depth works at
/// the fusion level, not as a filter's headroom. Every engine
/// running the hybrid search reads its depth here — core's query pipeline
/// and the WASM executor alike.
#[must_use]
pub fn dense_sparse_candidate_k(limit: usize) -> usize {
    limit.saturating_mul(2).max(limit.saturating_add(10))
}

#[cfg(test)]
#[path = "branches_tests.rs"]
mod tests;
