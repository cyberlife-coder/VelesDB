use super::*;

#[test]
fn next_edge_candidate_steps_by_one_below_u64_max() {
    assert_eq!(next_edge_candidate(0), 1);
    assert_eq!(next_edge_candidate(41), 42);
    assert_eq!(next_edge_candidate(u64::MAX - 1), u64::MAX);
}

/// The overflow branch: saturating here pinned every probe on the taken
/// `u64::MAX`. Two calls must also differ, or a retry after a collision would
/// re-probe the same random ID.
#[test]
fn next_edge_candidate_leaves_u64_max_for_fresh_random_ids() {
    let first = next_edge_candidate(u64::MAX);
    let second = next_edge_candidate(u64::MAX);
    assert_ne!(first, u64::MAX);
    assert_ne!(second, u64::MAX);
    assert_ne!(first, second, "each overflow must draw a new random id");
}
