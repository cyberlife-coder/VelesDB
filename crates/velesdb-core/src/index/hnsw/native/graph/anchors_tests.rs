//! Tests for the reachability anchors (#2259).

use super::super::super::distance::CachedSimdDistance;
use super::super::NativeHnsw;
use crate::distance::DistanceMetric;
use std::sync::atomic::Ordering;

type Graph = NativeHnsw<CachedSimdDistance>;

/// A smooth 4-D curve, ids in curve order: a node's nearest neighbours are
/// its batch mates, the shape that lost 5.6 % of its nodes on average.
#[allow(clippy::cast_precision_loss)]
fn curve(i: usize) -> Vec<f32> {
    let x = i as f32;
    vec![
        (x * 0.013).sin(),
        (x * 0.029).cos(),
        (x * 0.007).sin(),
        (x * 0.041).cos(),
    ]
}

/// Uniform vectors in `[-0.5, 0.5)^dim` from a seeded xorshift.
#[allow(clippy::cast_precision_loss)]
fn random(i: usize, dim: usize, seed: u64) -> Vec<f32> {
    let mut x = ((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03))
        | 1;
    (0..dim)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ((x >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        })
        .collect()
}

fn graph(dim: usize, capacity: usize) -> Graph {
    NativeHnsw::new(
        CachedSimdDistance::new(DistanceMetric::Euclidean, dim),
        24,
        300,
        capacity,
    )
}

fn place_all(g: &Graph, vectors: &[Vec<f32>]) {
    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    g.place_batch(&refs).expect("test: insert");
}

/// Every one of the first `n` nodes is reachable from the entry point over the
/// base layer, and every node but the entry point has an anchor whose list
/// holds it and a chain of anchors that ends at the entry point.
fn assert_anchored_and_reachable(g: &Graph, n: usize) {
    let unreachable = g.unreachable_among(0..n);
    assert!(
        unreachable.is_empty(),
        "{} of {n} nodes unreachable, e.g. {:?}",
        unreachable.len(),
        &unreachable[..unreachable.len().min(8)]
    );
    let entry_point = g.entry_point.load(Ordering::Acquire);
    let layers = g.layers.read();
    let base = &layers[0];
    assert_eq!(
        base.anchor_of(entry_point),
        None,
        "the entry point is the tree's root and has no anchor"
    );
    for node in (0..n).filter(|&node| node != entry_point) {
        let parent = base
            .anchor_of(node)
            .unwrap_or_else(|| panic!("node {node} has no anchor"));
        assert!(
            base.get_neighbors(parent).contains(&node),
            "node {node}'s anchor {parent} does not list it"
        );
        let mut at = node;
        let mut steps = 0;
        while let Some(up) = base.anchor_of(at) {
            steps += 1;
            assert!(steps <= n, "node {node}'s anchors cycle through {at}");
            at = up;
        }
        assert_eq!(
            at, entry_point,
            "node {node}'s anchors lead to {at}, not to the entry point"
        );
    }
    drop(layers);
}

#[test]
fn a_parallel_batch_on_a_curve_leaves_every_node_reachable() {
    for _ in 0..10 {
        let vectors: Vec<Vec<f32>> = (1..=1_100).map(curve).collect();
        let g = graph(4, vectors.len());
        place_all(&g, &vectors);
        assert_anchored_and_reachable(&g, vectors.len());
    }
}

#[test]
fn a_parallel_batch_of_random_vectors_leaves_every_node_reachable() {
    for seed in 0..3 {
        let vectors: Vec<Vec<f32>> = (0..2_000).map(|i| random(i, 64, seed)).collect();
        let g = graph(64, vectors.len());
        place_all(&g, &vectors);
        assert_anchored_and_reachable(&g, vectors.len());
    }
}

#[test]
fn sequential_inserts_leave_every_node_reachable() {
    let vectors: Vec<Vec<f32>> = (0..1_500).map(|i| random(i, 64, 7)).collect();
    let g = graph(64, vectors.len());
    for vector in &vectors {
        g.insert(vector).expect("test: insert");
    }
    assert_anchored_and_reachable(&g, vectors.len());
}

#[test]
fn concurrent_single_inserts_leave_every_node_reachable() {
    let vectors: Vec<Vec<f32>> = (0..2_000).map(|i| random(i, 32, 11)).collect();
    let g = graph(32, vectors.len());
    std::thread::scope(|scope| {
        for part in vectors.chunks(250) {
            let g = &g;
            scope.spawn(move || {
                for vector in part {
                    g.insert(vector).expect("test: insert");
                }
            });
        }
    });
    assert_anchored_and_reachable(&g, vectors.len());
}

#[test]
fn reordering_keeps_every_node_reachable() {
    let vectors: Vec<Vec<f32>> = (1..=1_100).map(curve).collect();
    let g = graph(4, vectors.len());
    place_all(&g, &vectors);
    assert!(
        g.reorder_for_locality().expect("test: reorder").is_some(),
        "the pass must actually renumber for this test to mean anything"
    );
    assert_anchored_and_reachable(&g, vectors.len());
}

#[test]
#[cfg(feature = "persistence")]
fn a_loaded_graph_rebuilds_its_anchors() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let vectors: Vec<Vec<f32>> = (1..=1_100).map(curve).collect();
    let g = graph(4, vectors.len());
    place_all(&g, &vectors);
    g.file_dump(dir.path(), "anchors").expect("test: dump");

    let loaded = Graph::file_load(
        dir.path(),
        "anchors",
        CachedSimdDistance::new(DistanceMetric::Euclidean, 4),
    )
    .expect("test: load");
    assert_anchored_and_reachable(&loaded, vectors.len());

    // Later inserts keep the loaded graph whole.
    let more: Vec<Vec<f32>> = (1_101..=1_600).map(curve).collect();
    place_all(&loaded, &more);
    assert_anchored_and_reachable(&loaded, vectors.len() + more.len());
}

/// Exact duplicates never displace an equal neighbour, so each one's in-edge
/// comes from its anchor. Every node stays reachable, and no list grows past
/// its cap: a full list of protected entries sends the edge a level down.
#[test]
fn duplicates_stay_reachable_without_any_list_growing_past_its_cap() {
    let n = 3_000;
    let same = vec![0.25_f32; 16];
    let batch = graph(16, n);
    place_all(&batch, &vec![same.clone(); n]);
    let sequential = graph(16, n);
    for _ in 0..n {
        sequential.insert(&same).expect("test: insert");
    }
    for g in [&batch, &sequential] {
        assert_anchored_and_reachable(g, n);
        let layers = g.layers.read();
        let longest = (0..n)
            .map(|node| layers[0].get_neighbors(node).len())
            .max()
            .unwrap_or(0);
        drop(layers);
        assert!(
            longest <= g.max_connections_0,
            "a base list holds {longest} entries, over the cap of {}",
            g.max_connections_0
        );
    }
}

/// A node that saw an empty graph, but lost its entry point to another
/// insert, connects through the winner instead of staying with no edge.
#[test]
fn losing_the_first_insert_race_still_links_the_node() {
    let g = graph(16, 2);
    g.insert(&random(0, 16, 3)).expect("test: the winner");
    g.insert_seeing_no_entry_point(&random(1, 16, 3))
        .expect("test: the losing side");
    assert_eq!(
        g.entry_point.load(Ordering::Acquire),
        0,
        "node 1 must have lost the entry point for this test to mean anything"
    );
    assert_anchored_and_reachable(&g, 2);
}

/// A batch whose first node finds the entry point already claimed consumes
/// no node, so that node is connected with the rest.
#[test]
fn a_batch_bootstrap_that_finds_the_entry_point_claimed_consumes_no_node() {
    let g = graph(16, 4);
    g.insert(&random(0, 16, 5))
        .expect("test: another insert claims it");
    let first = random(1, 16, 5);
    let placed = g
        .allocate_batch(&[first.as_slice()])
        .expect("test: allocate");
    assert_eq!(g.bootstrap_entry_point(&placed), 0);
}

/// A graph saved before #2259 can hold a node nothing links to any more; a
/// load anchors it back from its own list.
#[test]
#[cfg(feature = "persistence")]
fn a_loaded_graph_anchors_a_node_nothing_reached() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let vectors: Vec<Vec<f32>> = (1..=1_100).map(curve).collect();
    let g = graph(4, vectors.len());
    place_all(&g, &vectors);
    let entry_point = g.entry_point.load(Ordering::Acquire);
    let lost = if entry_point == 500 { 501 } else { 500 };
    let layers = g.layers.read();
    for node in 0..vectors.len() {
        let _ = layers[0].with_neighbors_mut(node, |list| list.retain(|&n| n != lost));
    }
    drop(layers);
    assert_eq!(
        g.unreachable_among([lost]),
        vec![lost],
        "the cut must orphan the node for this test to mean anything"
    );
    g.file_dump(dir.path(), "lost").expect("test: dump");

    let loaded = Graph::file_load(
        dir.path(),
        "lost",
        CachedSimdDistance::new(DistanceMetric::Euclidean, 4),
    )
    .expect("test: load");
    assert_anchored_and_reachable(&loaded, vectors.len());
}

/// A node whose own list names no anchored node gets a protected edge from
/// the entry point, or from below it, once no retry is left.
#[test]
fn a_node_no_neighbour_can_anchor_is_linked_from_the_entry_point() {
    let vectors: Vec<Vec<f32>> = (0..200).map(|i| random(i, 16, 9)).collect();
    let g = graph(16, vectors.len());
    place_all(&g, &vectors);
    let entry_point = g.entry_point.load(Ordering::Acquire);
    let node = (0..vectors.len())
        .find(|&n| n != entry_point)
        .expect("test: a node other than the entry point");
    let layers = g.layers.read();
    let base = &layers[0];
    let _ = base.with_neighbors_mut(node, |list| list.retain(|&n| n != entry_point));
    for neighbour in base.get_neighbors(node) {
        base.clear_anchor(neighbour);
    }
    base.clear_anchor(node);
    drop(layers);

    g.anchor_pending(vec![node]);
    let layers = g.layers.read();
    let parent = layers[0]
        .anchor_of(node)
        .expect("test: the fallback anchors the node");
    assert!(layers[0].get_neighbors(parent).contains(&node));
    drop(layers);
}

/// Promotions that overlap still leave one tree rooted at the entry point:
/// each moves the entry point and reparents under the promotion lock. Two
/// reparentings that interleaved could clear the anchor the other had just
/// given, and strand the tree below it.
#[test]
fn overlapping_promotions_keep_the_anchor_tree_rooted() {
    const THREADS: usize = 8;
    let n = 64;
    for round in 0..100 {
        let g = graph(8, n);
        let vectors: Vec<Vec<f32>> = (0..n).map(|i| random(i, 8, round)).collect();
        place_all(&g, &vectors);
        let entry_point = g.entry_point.load(Ordering::Acquire);
        let top = g.max_layer.load(Ordering::Acquire);
        let nodes: Vec<usize> = (0..n).filter(|&v| v != entry_point).take(THREADS).collect();
        let start = std::sync::Barrier::new(THREADS);
        std::thread::scope(|s| {
            for (t, &node) in nodes.iter().enumerate() {
                let (g, start) = (&g, &start);
                s.spawn(move || {
                    start.wait();
                    g.promote_entry_point(node, top + 1 + t);
                });
            }
        });
        assert_anchored_and_reachable(&g, n);
    }
}

/// Promoting the entry point itself to a higher layer keeps it the root: it
/// takes no anchor, least of all to itself.
#[test]
fn promoting_the_entry_point_again_keeps_it_the_root() {
    let n = 200;
    let g = graph(8, n);
    let vectors: Vec<Vec<f32>> = (0..n).map(|i| random(i, 8, 7)).collect();
    place_all(&g, &vectors);
    let entry_point = g.entry_point.load(Ordering::Acquire);
    g.promote_entry_point(entry_point, g.max_layer.load(Ordering::Acquire) + 1);
    assert_eq!(g.entry_point.load(Ordering::Acquire), entry_point);
    assert_anchored_and_reachable(&g, n);
}

/// The chain check catches an anchor cycle, which no other check sees: every
/// node on it has an anchor, and each anchor lists its child.
#[test]
#[should_panic(expected = "anchors cycle")]
fn the_chain_check_fails_on_an_anchor_cycle() {
    let n = 200;
    let g = graph(8, n);
    let vectors: Vec<Vec<f32>> = (0..n).map(|i| random(i, 8, 11)).collect();
    place_all(&g, &vectors);
    let entry_point = g.entry_point.load(Ordering::Acquire);
    let layers = g.layers.read();
    let base = &layers[0];
    let (a, b) = (0..n)
        .filter(|&a| a != entry_point)
        .find_map(|a| {
            base.get_neighbors(a)
                .into_iter()
                .find(|&b| b != entry_point && base.get_neighbors(b).contains(&a))
                .map(|b| (a, b))
        })
        .expect("test: two nodes that list each other");
    base.set_anchor(a, b);
    base.set_anchor(b, a);
    drop(layers);
    assert_anchored_and_reachable(&g, n);
}
