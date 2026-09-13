//! Tests for the batch connect in `backend_adapter`: every node of a batch
//! connects with its own query (#2264).

use super::distance::CachedSimdDistance;
use super::graph::NativeHnsw;
use crate::distance::DistanceMetric;

/// A distinct point per id: even ids in one cluster, odd ids in another far
/// from it.
fn clustered(id: usize, dim: usize) -> Vec<f32> {
    let offset = if id.is_multiple_of(2) { 0.0 } else { 1_000.0 };
    (0..dim)
        .map(|j| {
            let k = u16::try_from(id * dim + j).expect("test: fits a u16");
            offset + (f32::from(k) * 0.618_034).sin()
        })
        .collect()
}

/// A batch placed into an empty graph connects every node with its own
/// vector. The first node claims the entry point, so the others connect from
/// one position into the batch; read without that offset, each would connect
/// with its predecessor's vector. Even and odd ids fall in two clusters far
/// apart, so a node that connected with its predecessor's vector would list
/// the other cluster only.
#[test]
fn a_batch_into_an_empty_graph_connects_each_node_with_its_own_vector() {
    const POINTS: usize = 200;
    const DIM: usize = 8;
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, DIM);
    let hnsw = NativeHnsw::new(engine, 16, 100, POINTS);
    let vectors: Vec<Vec<f32>> = (0..POINTS).map(|id| clustered(id, DIM)).collect();
    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    let slots = hnsw.place_batch(&refs).expect("test: place the batch");
    assert_eq!(hnsw.len(), POINTS, "premise: every node is linked");

    let cluster: std::collections::HashMap<usize, usize> = slots
        .iter()
        .enumerate()
        .map(|(id, &slot)| (slot, id % 2))
        .collect();
    let layers = hnsw.layers.read();
    let with_own = slots
        .iter()
        .filter(|&&slot| {
            layers[0]
                .with_neighbors(slot, |neighbours| {
                    neighbours.iter().any(|n| cluster[n] == cluster[&slot])
                })
                .unwrap_or(false)
        })
        .count();
    drop(layers);
    assert!(
        with_own * 10 >= POINTS * 9,
        "{with_own} of {POINTS} nodes list a neighbour from their own cluster"
    );
}
