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

/// Past the 256-slot floor the slow path's headroom is an eighth of the
/// nodes held: every layer stays within that of the count, where a doubling
/// growth would leave up to as many slots unused as used.
#[test]
fn test_growth_past_the_floor_stays_within_an_eighth() {
    let hnsw = graph(0);
    for i in 0..3_000_u16 {
        hnsw.insert(&[f32::from(i), 0.0, 0.0, 0.0]).expect("insert");
    }
    let held = hnsw.len();
    let bound = held + held / 8 + 1;
    for (level, layer) in hnsw.layers.read().iter().enumerate() {
        assert!(
            layer.neighbors.len() <= bound,
            "layer {level} holds {} slots for {held} nodes, past {bound}",
            layer.neighbors.len()
        );
    }
}

/// A pseudo-random 64-d vector, the same for the same `seed`.
fn seeded_vector(seed: u64) -> Vec<f32> {
    let mut state = super::super::xorshift64(seed.wrapping_add(1).wrapping_mul(0x9E37_79B9));
    (0..64)
        .map(|_| {
            state = super::super::xorshift64(state);
            let unit = f32::from(u16::try_from(state % 10_000).expect("below 10 000")) / 10_000.0;
            unit - 0.5
        })
        .collect()
}

/// A search racing an insert never stops at the node being inserted.
///
/// An insert links its node layer by layer. Published from the top layer
/// down, a node drawn to layer 1 or above was reachable there — and, being
/// the exact match, the greedy descent's pick — while it still had no layer-0
/// neighbour: a layer-0 walk started from it returned that one node and
/// nothing else, so a search for `k = 10` over a thousand nodes returned 1.
/// Every searcher here queries the vector being inserted, which puts the
/// descent on the new node whenever the window is open.
#[test]
fn test_a_search_racing_an_insert_never_stops_at_the_node_being_linked() {
    const PREFILL: u64 = 1_000;
    const INSERTS: u64 = 400;
    const K: usize = 10;
    let hnsw = graph(PREFILL as usize + INSERTS as usize);
    for seed in 0..PREFILL {
        hnsw.insert(&seeded_vector(seed)).expect("prefill insert");
    }
    let inserting = parking_lot::Mutex::new(seeded_vector(PREFILL));
    let done = std::sync::atomic::AtomicBool::new(false);
    let searched = std::sync::atomic::AtomicUsize::new(0);
    let short = std::sync::atomic::AtomicUsize::new(0);
    let raced = std::thread::scope(|scope| {
        for _ in 0..SEARCHERS {
            scope.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    let query = inserting.lock().clone();
                    if hnsw.search(&query, K, 64).len() < K {
                        short.fetch_add(1, Ordering::Relaxed);
                    }
                    searched.fetch_add(1, Ordering::AcqRel);
                }
            });
        }
        let before = wait_for_every_searcher(&searched);
        for seed in PREFILL..PREFILL + INSERTS {
            let vector = seeded_vector(seed);
            inserting.lock().clone_from(&vector);
            hnsw.insert(&vector).expect("insert");
        }
        let raced = searched.load(Ordering::Acquire) - before;
        done.store(true, Ordering::Release);
        raced
    });
    assert!(raced > 0, "no search ran while the inserts did");
    assert_eq!(
        short.load(Ordering::Relaxed),
        0,
        "{} of {raced} searches racing the inserts returned fewer than {K} of {} nodes",
        short.load(Ordering::Relaxed),
        PREFILL + INSERTS
    );
}

/// How many searchers race the inserts.
const SEARCHERS: usize = 3;

/// Waits until every searcher has finished one search, so the inserts start
/// with all of them running, and returns the searches counted so far.
fn wait_for_every_searcher(searched: &std::sync::atomic::AtomicUsize) -> usize {
    loop {
        let seen = searched.load(Ordering::Acquire);
        if seen >= SEARCHERS {
            return seen;
        }
        std::thread::yield_now();
    }
}
