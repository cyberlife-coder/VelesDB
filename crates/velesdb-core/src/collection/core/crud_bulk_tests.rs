//! `upsert_bulk`'s V2 path: the direct writer places each vector in the
//! graph's arena and maps its id there, and the async builder links that slot
//! where it is (#2264). The builder used to insert a copy of every vector it
//! had queued, which placed each one a second time, and at the drain gave back
//! to an id deleted or upserted in between the vector it had queued.

use crate::collection::streaming::AsyncIndexBuilderConfig;
use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::path::PathBuf;

const DIM: usize = 8;

/// A builder batch larger than any load here: the builder holds every point
/// until a drain.
const HOLD_ALL: usize = 100_000;

fn v2_collection(path: PathBuf, merge_threshold: usize) -> Collection {
    Collection::create_with_async_builder(
        path,
        DIM,
        DistanceMetric::Euclidean,
        AsyncIndexBuilderConfig {
            merge_threshold,
            segment_count: None,
        },
    )
    .expect("test: create a V2 collection")
}

/// A distinct vector per id. The metric is Euclidean, so the arena stores it
/// as given.
fn vector(id: u64) -> Vec<f32> {
    let x = f32::from(u16::try_from(id).expect("test: ids fit a u16"));
    let dims = u16::try_from(DIM).expect("test: DIM fits a u16");
    (1..=dims)
        .map(|j| (x * f32::from(j) * 0.618_034).sin())
        .collect()
}

fn points(ids: std::ops::Range<u64>) -> Vec<Point> {
    ids.map(|id| Point::without_payload(id, vector(id)))
        .collect()
}

/// A V2 collection holding points `0..n`, bulk-loaded and not yet drained.
fn bulk_loaded(n: u64) -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = v2_collection(dir.path().join("v2"), HOLD_ALL);
    coll.upsert_bulk(&points(0..n)).expect("test: upsert_bulk");
    (dir, coll)
}

/// The arena's slots and the graph's nodes.
fn slots_and_nodes(coll: &Collection) -> (usize, usize) {
    let index = &coll.storage.index;
    (index.graph_vector_count(), index.inner.read().len())
}

/// The vector the index holds for `id`, read from the slot its mapping names.
fn indexed_vector(coll: &Collection, id: u64) -> Option<Vec<f32>> {
    let index = &coll.storage.index;
    let slot = index.mappings.get_idx(id)?;
    index
        .inner
        .read()
        .with_contiguous_vectors(|arena| arena.get(slot).map(<[f32]>::to_vec))
}

/// The mapped slots no graph search can reach.
fn unlinked_slots(coll: &Collection) -> Vec<usize> {
    let index = &coll.storage.index;
    index
        .inner
        .read()
        .unlinked_nodes(index.mappings.iter().map(|(_, slot)| slot))
}

/// Loads `n` points, drains the builder, and checks that the arena holds one
/// slot per point and that the graph links every one of them.
fn assert_placed_once_and_linked(n: u64) {
    let (_dir, coll) = bulk_loaded(n);
    let expected = usize::try_from(n).expect("test: fits a usize");
    assert_eq!(
        slots_and_nodes(&coll),
        (expected, 0),
        "premise: the direct writer placed every point and nothing is linked yet"
    );
    coll.flush().expect("test: flush drains the builder");
    assert_eq!(
        slots_and_nodes(&coll),
        (expected, expected),
        "(arena slots, graph nodes) after a {n}-point load"
    );
    assert_eq!(unlinked_slots(&coll), Vec::<usize>::new());
}

/// Below the size at which the graph connects a batch in parallel, it links
/// the nodes one at a time.
#[test]
fn a_small_bulk_load_places_each_vector_once() {
    assert_placed_once_and_linked(40);
}

/// Above it, in parallel chunks.
#[test]
fn a_bulk_load_places_each_vector_once() {
    assert_placed_once_and_linked(300);
}

/// A load that fills the builder's batch is linked by `upsert_bulk` itself,
/// before any flush.
#[test]
fn a_bulk_load_that_fills_the_builders_batch_is_linked_without_a_flush() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = v2_collection(dir.path().join("v2"), 200);
    coll.upsert_bulk(&points(0..300))
        .expect("test: upsert_bulk");
    assert_eq!(slots_and_nodes(&coll), (300, 300));
}

/// A point deleted after its bulk load, before the drain, stays deleted.
#[test]
fn a_point_deleted_before_the_drain_stays_deleted() {
    let (_dir, coll) = bulk_loaded(300);
    coll.delete(&[7]).expect("test: delete");
    coll.flush().expect("test: flush drains the builder");
    assert_eq!(
        indexed_vector(&coll, 7),
        None,
        "the drain mapped deleted id 7 again"
    );
    // Its slot stays behind, unlinked: a tombstone.
    assert_eq!(slots_and_nodes(&coll), (300, 299));
}

/// A point upserted after its bulk load, before the drain, keeps the vector
/// it was upserted with.
#[test]
fn a_point_upserted_before_the_drain_keeps_its_new_vector() {
    let (_dir, coll) = bulk_loaded(300);
    let newer = vector(10_000);
    coll.upsert([Point::without_payload(7, newer.clone())])
        .expect("test: upsert");
    coll.flush().expect("test: flush drains the builder");
    assert_eq!(
        indexed_vector(&coll, 7).as_deref(),
        Some(newer.as_slice()),
        "id 7 went back to the vector it was bulk-loaded with"
    );
    // The upsert linked its own slot: the drain links the 299 others, and
    // not that one a second time.
    assert_eq!(slots_and_nodes(&coll), (301, 300));
}

/// An id bulk-loaded twice before the drain is linked once, at the slot of
/// its last write.
#[test]
fn an_id_bulk_loaded_twice_before_the_drain_is_linked_once() {
    let (_dir, coll) = bulk_loaded(300);
    let newer = vector(10_000);
    coll.upsert_bulk(&[Point::without_payload(7, newer.clone())])
        .expect("test: second upsert_bulk");
    coll.flush().expect("test: flush drains the builder");
    assert_eq!(indexed_vector(&coll, 7).as_deref(), Some(newer.as_slice()));
    assert_eq!(slots_and_nodes(&coll), (301, 300));
}

/// A vacuum rebuilds the graph from every mapped id, the queued ones
/// included; the drain does not link those slots a second time.
#[test]
fn a_slot_a_vacuum_linked_is_not_linked_again_at_the_drain() {
    let (_dir, coll) = bulk_loaded(300);
    coll.vacuum_hnsw_index().expect("test: vacuum");
    assert_eq!(
        slots_and_nodes(&coll),
        (300, 300),
        "premise: the vacuum linked every queued point"
    );
    coll.flush().expect("test: flush drains the builder");
    assert_eq!(slots_and_nodes(&coll), (300, 300));
}

/// `reorder_for_locality` sizes its permutation by the graph's nodes, and
/// refuses an arena that holds more slots than that.
#[test]
fn a_bulk_loaded_collection_can_be_reordered() {
    // Above the size below which reordering declines to run.
    let (_dir, coll) = bulk_loaded(1_200);
    coll.reorder_for_locality()
        .expect("test: reorder_for_locality after a V2 load");
}

/// An index with its exact-distance features off gives the direct writer no
/// slot to fill, so the builder places those vectors itself at the drain. A
/// collection meets such an index only by loading one, as here.
#[test]
fn a_bulk_load_into_an_index_with_exact_features_off_is_indexed_at_the_drain() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let path = dir.path().join("v2");
    drop(v2_collection(path.clone(), HOLD_ALL));
    crate::index::HnswIndex::new_fast_insert(DIM, DistanceMetric::Euclidean)
        .expect("test: fast-insert index")
        .save(&path)
        .expect("test: save it over the collection's index");
    let coll = Collection::open(path).expect("test: reopen the collection");
    assert!(
        !coll.storage.index.has_vector_storage(),
        "premise: the loaded index keeps its exact-distance features off"
    );
    assert!(
        coll.streaming.async_index_builder.is_some(),
        "premise: upsert_bulk takes the V2 path"
    );

    coll.upsert_bulk(&points(0..300))
        .expect("test: upsert_bulk");
    coll.flush().expect("test: flush drains the builder");
    assert_eq!(coll.storage.index.mappings.len(), 300);
    assert_eq!(slots_and_nodes(&coll), (300, 300));
    assert_eq!(unlinked_slots(&coll), Vec::<usize>::new());
}
