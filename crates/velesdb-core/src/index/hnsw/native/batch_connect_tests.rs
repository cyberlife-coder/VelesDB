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

const POINTS: usize = 200;
const DIM: usize = 8;

/// A graph sized for one batch of [`POINTS`] vectors.
fn empty_graph() -> NativeHnsw<CachedSimdDistance> {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, DIM);
    NativeHnsw::new(engine, 16, 100, POINTS)
}

/// How many of `slots` list, in layer 0, a neighbour from their own cluster.
///
/// `cluster` maps a slot to the cluster its vector belongs to. A node that
/// connected with another node's vector searches the wrong cluster and lists
/// the other one only.
fn slots_with_a_neighbour_from_their_cluster(
    hnsw: &NativeHnsw<CachedSimdDistance>,
    slots: &[usize],
    cluster: &std::collections::HashMap<usize, usize>,
) -> usize {
    let layers = hnsw.layers.read();
    let with_own = slots
        .iter()
        .filter(|&&slot| {
            layers[0]
                .with_neighbors(slot, |neighbours| {
                    neighbours
                        .iter()
                        .any(|n| cluster.get(n) == cluster.get(&slot))
                })
                .unwrap_or(false)
        })
        .count();
    drop(layers);
    with_own
}

/// Maps each slot to the cluster [`clustered`] built its vector in, from the
/// slots in the order their ids were written.
fn clusters_by_slot(slots: &[usize]) -> std::collections::HashMap<usize, usize> {
    slots
        .iter()
        .enumerate()
        .map(|(id, &slot)| (slot, id % 2))
        .collect()
}

/// At least this share of the nodes must list a neighbour from their own
/// cluster, in tenths. Measured: 200 of 200 when each node connects with its
/// own vector, 133 with `queries[0]` for every node and 68 one position off.
const OWN_CLUSTER_FLOOR_TENTHS: usize = 9;

/// A batch placed into an empty graph connects every node with its own
/// vector. The first node claims the entry point, so the others connect from
/// one position into the batch; read without that offset, each would connect
/// with its predecessor's vector. Even and odd ids fall in two clusters far
/// apart, so a node that connected with its predecessor's vector would list
/// the other cluster only.
#[test]
fn a_batch_into_an_empty_graph_connects_each_node_with_its_own_vector() {
    let hnsw = empty_graph();
    let vectors: Vec<Vec<f32>> = (0..POINTS).map(|id| clustered(id, DIM)).collect();
    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    let slots = hnsw.place_batch(&refs).expect("test: place the batch");
    assert_eq!(hnsw.len(), POINTS, "premise: every node is linked");

    let with_own =
        slots_with_a_neighbour_from_their_cluster(&hnsw, &slots, &clusters_by_slot(&slots));
    assert!(
        with_own * 10 >= POINTS * OWN_CLUSTER_FLOOR_TENTHS,
        "{with_own} of {POINTS} nodes list a neighbour from their own cluster"
    );
}

/// The same, for the drain's path: nodes the direct writer pushed unlinked
/// and `link_placed` connects afterwards, each with the vector its own slot
/// holds (`BatchQueries::Prepared`). Without its own mirror the `Prepared`
/// arm was unconstrained — `f(queries[0])` and `f(queries[position - 1])`
/// both passed the whole suite.
#[test]
fn link_placed_connects_each_node_with_the_vector_its_own_slot_holds() {
    let hnsw = empty_graph();
    let vectors: Vec<Vec<f32>> = (0..POINTS).map(|id| clustered(id, DIM)).collect();
    // One ordinary insert first: it initializes the arena the direct writer
    // pushes into, and claims the entry point, as the graph a real drain
    // links into already has one.
    let seed = hnsw.insert(&vectors[0]).expect("test: seed the graph");
    let refs: Vec<&[f32]> = vectors[1..].iter().map(Vec::as_slice).collect();
    let first = hnsw.push_unlinked(&refs).expect("test: push the batch");
    assert_eq!(hnsw.len(), 1, "premise: push_unlinked links nothing");

    let placed: Vec<usize> = (first..first + refs.len()).collect();
    hnsw.link_placed(&placed).expect("test: link the batch");
    assert_eq!(hnsw.len(), POINTS, "premise: every node is linked");

    let mut all = vec![seed];
    all.extend(placed.iter().copied());
    let with_own =
        slots_with_a_neighbour_from_their_cluster(&hnsw, &placed, &clusters_by_slot(&all));
    let linked = placed.len();
    assert!(
        with_own * 10 >= linked * OWN_CLUSTER_FLOOR_TENTHS,
        "{with_own} of {linked} nodes list a neighbour from their own cluster"
    );
}
