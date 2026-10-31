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
        coll.async_index_builder().is_some(),
        "premise: upsert_bulk takes the V2 path"
    );

    coll.upsert_bulk(&points(0..300))
        .expect("test: upsert_bulk");
    assert_eq!(
        first_hit(&coll, 150),
        Some(150),
        "a buffered point is searchable before the drain"
    );
    coll.flush().expect("test: flush drains the builder");
    assert_eq!(coll.storage.index.mappings.len(), 300);
    assert_eq!(slots_and_nodes(&coll), (300, 300));
    assert_eq!(unlinked_slots(&coll), Vec::<usize>::new());
}

/// The id of `id`'s own vector's nearest neighbour, by graph search.
fn first_hit(coll: &Collection, id: u64) -> Option<u64> {
    coll.search(&vector(id), 1)
        .expect("test: search")
        .first()
        .map(|hit| hit.point.id)
}

/// `upsert_bulk` returns before the builder links its points, and a graph
/// walk cannot reach an unlinked slot: search merges what the builder holds.
#[test]
fn a_point_the_builder_holds_is_found_by_search() {
    let (_dir, coll) = bulk_loaded(300);
    assert_eq!(
        unlinked_slots(&coll).len(),
        300,
        "premise: the builder holds every point, and the graph links none"
    );
    for id in [0, 150, 299] {
        assert_eq!(first_hit(&coll, id), Some(id));
    }
}

/// A point linked by a vacuum while still queued is reachable both ways at
/// once — through the graph and through the builder — and is returned once.
#[test]
fn a_point_both_linked_and_queued_is_returned_once() {
    let (_dir, coll) = bulk_loaded(300);
    coll.vacuum_hnsw_index().expect("test: vacuum");
    assert_eq!(
        coll.async_index_builder()
            .expect("premise: a V2 collection")
            .pending_len(),
        300,
        "premise: the vacuum linked the points and the builder still holds them"
    );
    let hits = coll.search(&vector(7), 10).expect("test: search");
    let mut ids: Vec<u64> = hits.iter().map(|hit| hit.point.id).collect();
    assert_eq!(ids.first(), Some(&7));
    let returned = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), returned, "an id was returned twice");
}

/// Concurrent `upsert_bulk` and `upsert` must never give one arena slot to
/// two ids (#2246). With the builder holding every bulk id, each one is still
/// mapped to the slot the direct writer gave it, and those slots are the
/// ones the exhaustive search reads (`tests/concurrent_bulk_slots.rs` runs
/// the same race with the builder draining as it goes).
#[test]
fn direct_written_slots_hold_their_own_vectors_while_the_builder_holds_them() {
    const ROUNDS: u64 = 400;
    const BATCH: u64 = 8;
    const BULK_BASE: u64 = 30_000;
    let batch = |base: u64, round: u64| points(base + round * BATCH..base + (round + 1) * BATCH);

    let (_dir, coll) = {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let coll = v2_collection(dir.path().join("v2"), HOLD_ALL);
        (dir, coll)
    };
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for round in 0..ROUNDS {
                coll.upsert_bulk(&batch(BULK_BASE, round))
                    .expect("test: upsert_bulk");
            }
        });
        scope.spawn(|| {
            for round in 0..ROUNDS {
                coll.upsert(batch(0, round)).expect("test: upsert");
            }
        });
    });

    let bulk_ids = BULK_BASE..BULK_BASE + ROUNDS * BATCH;
    let linked = linked_ids(&coll, bulk_ids.clone());
    assert!(
        linked.is_empty(),
        "premise: the builder already linked {} bulk ids, e.g. {:?}",
        linked.len(),
        &linked[..linked.len().min(8)]
    );
    let lost = ids_not_finding_their_vector(&coll, (0..ROUNDS * BATCH).chain(bulk_ids));
    assert!(
        lost.is_empty(),
        "{} ids no longer find their own vector before the drain, e.g. {:?}",
        lost.len(),
        &lost[..lost.len().min(8)]
    );
}

/// The `ids` mapped to a slot the graph links.
fn linked_ids(coll: &Collection, ids: impl Iterator<Item = u64>) -> Vec<u64> {
    let unlinked = unlinked_slots(coll);
    ids.filter(|&id| {
        coll.storage
            .index
            .mappings
            .get_idx(id)
            .is_some_and(|slot| !unlinked.contains(&slot))
    })
    .collect()
}

/// The `ids` whose own vector's exhaustive search does not return them first.
fn ids_not_finding_their_vector(coll: &Collection, ids: impl Iterator<Item = u64>) -> Vec<u64> {
    ids.filter(|&id| {
        let hits = coll
            .search_with_quality(&vector(id), 1, crate::SearchQuality::Perfect)
            .expect("test: exhaustive search");
        hits.first().map(|hit| hit.point.id) != Some(id)
    })
    .collect()
}

/// A V2 collection whose index has its exact-distance features off, loaded
/// from disk as a collection meets one: `upsert_bulk` then buffers vectors.
fn fast_insert_v2_collection() -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let path = dir.path().join("v2");
    drop(v2_collection(path.clone(), HOLD_ALL));
    crate::index::HnswIndex::new_fast_insert(DIM, DistanceMetric::Euclidean)
        .expect("test: fast-insert index")
        .save(&path)
        .expect("test: save it over the collection's index");
    let coll = Collection::open(path).expect("test: reopen the collection");
    assert!(!coll.storage.index.has_vector_storage(), "premise");
    (dir, coll)
}

/// A buffered point deleted or overwritten after its bulk load is scored on
/// what storage holds now, not on the copy the builder queued.
#[test]
fn a_buffered_point_is_found_as_it_is_now() {
    let (_dir, coll) = fast_insert_v2_collection();
    coll.upsert_bulk(&points(0..300))
        .expect("test: upsert_bulk");

    coll.delete(&[150]).expect("test: delete");
    assert_ne!(
        first_hit(&coll, 150),
        Some(150),
        "a deleted point is returned"
    );

    coll.upsert([Point::without_payload(151, vector(10_000))])
        .expect("test: upsert");
    assert_ne!(
        first_hit(&coll, 151),
        Some(151),
        "151 is found on the vector it was bulk-loaded with"
    );
    assert_eq!(first_hit(&coll, 10_000), Some(151), "151 on its new vector");
}

/// A flush inserts what storage holds for each buffered id, not the copy the
/// builder queued: a point overwritten before the flush keeps its new vector
/// after it, and a deleted one stays out of the index.
#[test]
fn a_flush_indexes_buffered_points_as_they_are_now() {
    let (_dir, coll) = fast_insert_v2_collection();
    coll.upsert_bulk(&points(0..300))
        .expect("test: upsert_bulk");
    coll.upsert([Point::without_payload(151, vector(10_000))])
        .expect("test: upsert");
    coll.delete(&[150]).expect("test: delete");

    coll.flush().expect("test: flush drains the builder");

    assert_eq!(
        indexed_vector(&coll, 151).as_deref(),
        Some(vector(10_000).as_slice()),
        "the flush gave 151 back the vector it was bulk-loaded with"
    );
    assert_eq!(
        indexed_vector(&coll, 150),
        None,
        "the flush indexed deleted 150"
    );
    // Exhaustive: 151 went into the graph twice (its upsert, then the
    // flush), and a `k = 1` walk on this graph can come back empty — an
    // index behaviour this test does not cover.
    let hits = coll
        .search_with_quality(&vector(10_000), 1, crate::SearchQuality::Perfect)
        .expect("test: exhaustive search");
    assert_eq!(hits.first().map(|hit| hit.point.id), Some(151));
}
