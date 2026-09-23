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

/// A pre-expansion smaller than a capacity the slow path already grew adds
/// its new layers at that capacity, not at its own smaller count: the clamp
/// in `grow_layers` is what keeps them long enough for the fast path.
#[test]
fn test_a_layer_added_by_a_smaller_pre_expansion_covers_the_capacity() {
    let hnsw = graph(0);
    hnsw.expand_layers(0, 0);
    let covered = hnsw.pre_allocated_capacity.load(Ordering::Relaxed);
    let layers_before = hnsw.layers.read().len();

    hnsw.pre_expand_layers(100);

    assert!(
        hnsw.layers.read().len() > layers_before,
        "no layer was added"
    );
    assert!(
        shortest_layer(&hnsw) >= covered,
        "a layer holds {} slots, below the capacity {covered}",
        shortest_layer(&hnsw)
    );
}

/// A new index grows its upper layers from the nodes it holds, not from the
/// `max_elements` slots its base layer is created with: seeding the capacity
/// with those made every upper layer as long as the base.
#[test]
fn test_a_small_index_keeps_its_upper_layers_small() {
    let hnsw = graph(100_000);
    for i in 0..200_u8 {
        hnsw.insert(&[f32::from(i), 0.0, 0.0, 0.0]).expect("insert");
    }
    let layers = hnsw.layers.read();
    for (level, layer) in layers.iter().enumerate().skip(1) {
        assert!(
            layer.neighbors.len() < 1_000,
            "layer {level} holds {} slots for 200 nodes",
            layer.neighbors.len()
        );
    }
}
