use super::{effective_chunk_policy, MIN_CHUNK_BYTES};
use crate::context::estimator::HeuristicEstimator;
use crate::context::model::CompilePolicy;

#[test]
fn test_effective_chunk_policy_floors_chunk_size_under_a_tiny_budget() {
    // A budget of one usable token must NOT drive the chunk ceiling down
    // toward a byte, which would explode a large fragment into one heap
    // String per byte (a caller-controlled memory-amplification DoS).
    let policy = CompilePolicy::default();
    let effective = effective_chunk_policy(&policy, 1, &HeuristicEstimator);
    assert!(
        effective.max_chunk_bytes >= MIN_CHUNK_BYTES,
        "tiny budget drove chunk size to {} bytes, below the {MIN_CHUNK_BYTES}-byte floor",
        effective.max_chunk_bytes
    );
}

#[test]
fn test_effective_chunk_policy_floors_a_caller_supplied_tiny_chunk_size() {
    // A caller cannot bypass the floor by setting a tiny max_chunk_bytes
    // in the request policy — the same amplification vector otherwise.
    let mut policy = CompilePolicy::default();
    policy.chunk.max_chunk_bytes = 1;
    let effective = effective_chunk_policy(&policy, 1_000, &HeuristicEstimator);
    assert!(
        effective.max_chunk_bytes >= MIN_CHUNK_BYTES,
        "caller max_chunk_bytes=1 bypassed the {MIN_CHUNK_BYTES}-byte floor, got {}",
        effective.max_chunk_bytes
    );
}
