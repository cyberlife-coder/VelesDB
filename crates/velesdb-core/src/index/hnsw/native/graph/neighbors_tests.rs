//! Tests for neighbour-list eviction.

use super::super::super::distance::{CachedSimdDistance, DistanceEngine};
use super::super::NativeHnsw;
use crate::distance::DistanceMetric;

/// DotProduct distances between positively correlated vectors are negative
/// (`-dot`), below the `0.0` the scan for the farthest entry used to start
/// from: a node farther than every entry still displaced one. It displaces
/// none now.
#[test]
fn a_dot_product_list_keeps_its_entries_against_a_farther_node() {
    let g: NativeHnsw<CachedSimdDistance> = NativeHnsw::new(
        CachedSimdDistance::new(DistanceMetric::DotProduct, 2),
        24,
        100,
        8,
    );
    let rows: [&[f32]; 4] = [&[1.0, 0.0], &[0.9, 0.1], &[0.8, 0.2], &[0.1, 0.9]];
    let placed = g.allocate_batch(&rows).expect("test: place");
    let [owner, closest, close, far] = [placed[0].0, placed[1].0, placed[2].0, placed[3].0];
    let guard = g.vectors.read();
    let vectors = guard.as_ref().expect("test: arena");
    let owner_vec = vectors.get(owner).expect("test: owner");
    let far_dist = g.distance.distance(owner_vec, rows[3]);
    assert!(
        far_dist < 0.0,
        "a DotProduct distance here is below 0.0: {far_dist}"
    );

    let mut list = vec![closest, close];
    g.evict_most_redundant(&mut list, owner_vec, far, far_dist, vectors, |_| false);
    drop(guard);
    assert_eq!(list, [closest, close]);
}
