//! Tests for the layer growth of `insert` (#2306).

use std::sync::atomic::Ordering;

use super::super::NativeHnsw;
use crate::distance::DistanceMetric;
use crate::index::hnsw::native::distance::CpuDistance;

fn graph(max_elements: usize) -> NativeHnsw<CpuDistance> {
    NativeHnsw::new(
        CpuDistance::new(DistanceMetric::Euclidean),
        16,
        100,
        max_elements,
    )
}

/// The fewest slots any layer holds: what every node id the fast path lets
/// through must stay below.
fn shortest_layer(hnsw: &NativeHnsw<CpuDistance>) -> usize {
    hnsw.layers
        .read()
        .iter()
        .map(|layer| layer.neighbors.len())
        .min()
        .unwrap_or(0)
}

/// Once the index has outgrown its capacity, the slow path grows it far
/// enough that the next inserts return to the fast path, which takes no
/// `layers.write`: an insert past the capacity must not stay on the slow
/// path for good (#2306).
#[test]
fn test_growing_past_the_capacity_restores_the_fast_path() {
    let hnsw = graph(8);
    for i in 0..64_u8 {
        hnsw.insert(&[f32::from(i), 0.0, 0.0, 0.0]).expect("insert");
    }
    let covered = hnsw.pre_allocated_capacity.load(Ordering::Relaxed);
    assert!(
        covered > hnsw.len(),
        "capacity {covered} does not cover the next insert past {} nodes",
        hnsw.len()
    );
}

/// A layer the slow path adds covers the capacity the fast path already
/// trusts. `Layer`'s accessors skip an id past a layer's end in silence, so
/// a new top layer sized to the node that created it would drop the
/// neighbour writes of every later node the fast path sends to it.
#[test]
fn test_a_layer_added_after_pre_expansion_covers_the_capacity() {
    let hnsw = graph(0);
    hnsw.pre_expand_layers(1000);
    let above_top = hnsw.layers.read().len();

    hnsw.expand_layers(10, above_top);
    hnsw.expand_layers(500, above_top);

    assert!(
        shortest_layer(&hnsw) > 500,
        "a layer holds {} slots, below node 500 the fast path let through",
        shortest_layer(&hnsw)
    );
}

/// Pre-expanding for fewer nodes than the index already covers neither
/// shrinks the published capacity nor adds a layer shorter than it.
#[test]
fn test_a_smaller_pre_expansion_keeps_the_capacity() {
    let hnsw = graph(0);
    hnsw.pre_expand_layers(1000);
    hnsw.pre_expand_layers(10);

    let covered = hnsw.pre_allocated_capacity.load(Ordering::Relaxed);
    assert!(covered >= 1000, "capacity fell to {covered}");
    assert!(shortest_layer(&hnsw) >= covered);
}
