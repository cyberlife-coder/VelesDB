//! The bitmap branch of the graph-anchored `NEAR` search runs at the
//! collection's configured `[search]` quality (#2430), not at a hard-coded
//! `Balanced`.
//!
//! The branch only runs above `ANCHORED_EXACT_SCORE_MAX` anchors through
//! `search_near_with_anchor_ids`, so these tests call `search_anchors_bitmap`
//! directly instead of building a graph of that size.
//!
//! A sparse anchor set is what makes `ef_search` observable on a small
//! corpus: the index keeps only the candidates that fall in the bitmap, so a
//! narrow beam leaves fewer than `K` of them where a wide one does not. The
//! outcome does not depend on how the HNSW graph happened to be built.

#![cfg(all(test, feature = "persistence"))]

use std::collections::HashSet;

use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::test_fixtures::fixtures::setup_collection;
use crate::SearchQuality;

const DIM: usize = 32;
const POINTS: u64 = 4_000;
/// One id in `ANCHOR_STRIDE` is an anchor.
const ANCHOR_STRIDE: u64 = 10;
const K: usize = 10;
/// The smallest `ef_search` a `[search]` section accepts.
const LOW_EF: usize = 16;
const QUERIES: u64 = 8;

/// Deterministic pseudo-random coordinates: a fixture that changes between runs
/// would make a recall-sensitive assertion flap.
#[allow(clippy::cast_precision_loss)] // values below 10_000, exact in f32
fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 10_000) as f32 / 10_000.0
        })
        .collect()
}

fn fixture() -> (tempfile::TempDir, Collection, HashSet<u64>) {
    let (dir, collection) = setup_collection(DIM);
    let points: Vec<Point> = (0..POINTS)
        .map(|id| Point::new(id, vector(id), None))
        .collect();
    collection.upsert(points).expect("test: upsert");
    let anchors = (0..POINTS).filter(|id| id % ANCHOR_STRIDE == 0).collect();
    (dir, collection, anchors)
}

fn queries() -> Vec<Vec<f32>> {
    (0..QUERIES).map(|i| vector(POINTS + 1 + i)).collect()
}

/// What the bitmap branch returns for `query`, as a sorted id list.
fn anchored_ids(collection: &Collection, query: &[f32], anchors: &HashSet<u64>) -> Vec<u64> {
    let results = collection
        .search_anchors_bitmap(query, anchors, None, K, DistanceMetric::Cosine)
        .expect("test: search_anchors_bitmap");
    let mut ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    ids
}

/// What the index returns for the same bitmap at an explicit `quality`: the
/// answer the branch must give when `quality` is the one configured.
fn index_ids(
    collection: &Collection,
    query: &[f32],
    anchors: &HashSet<u64>,
    quality: SearchQuality,
) -> Vec<u64> {
    let bitmap: roaring::RoaringBitmap = anchors
        .iter()
        .map(|&id| u32::try_from(id).expect("test: ids fit in u32"))
        .collect();
    let results = collection
        .storage
        .index
        .search_with_quality_and_bitmap(query, K, quality, &bitmap)
        .expect("test: search_with_quality_and_bitmap");
    let mut ids: Vec<u64> = results.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    ids
}

/// With `[search] ef_search` set, the bitmap branch answers as the index does
/// at that quality.
///
/// The control comes first: unless `LOW_EF` and `Balanced` disagree for some
/// query on this fixture, the assertion below cannot tell a branch that reads
/// the configuration from one that ignores it.
#[test]
fn test_anchor_bitmap_search_follows_the_configured_search_quality() {
    let (_dir, collection, anchors) = fixture();
    let low = SearchQuality::Custom(LOW_EF);
    let queries = queries();

    let disagreeing = queries
        .iter()
        .filter(|q| {
            index_ids(&collection, q, &anchors, low)
                != index_ids(&collection, q, &anchors, SearchQuality::Balanced)
        })
        .count();
    assert!(
        disagreeing > 0,
        "CONTROL: ef_search={LOW_EF} and Balanced must answer differently for at least one of \
         {QUERIES} queries, or the assertion below is vacuous"
    );

    collection.set_runtime_search_quality(low);
    for (qi, query) in queries.iter().enumerate() {
        assert_eq!(
            anchored_ids(&collection, query, &anchors),
            index_ids(&collection, query, &anchors, low),
            "the bitmap branch must use the configured [search] quality, not a hard-coded \
             Balanced (query {qi})"
        );
    }
}

/// A collection whose `[search]` is untouched keeps answering at `Balanced`:
/// the default configuration resolves to it, so no existing caller sees a
/// different result.
#[test]
fn test_anchor_bitmap_search_defaults_to_balanced() {
    let (_dir, collection, anchors) = fixture();
    assert_eq!(
        collection.runtime_search_quality(),
        SearchQuality::Balanced,
        "CONTROL: a fresh collection must start on Balanced"
    );
    for (qi, query) in queries().iter().enumerate() {
        assert_eq!(
            anchored_ids(&collection, query, &anchors),
            index_ids(&collection, query, &anchors, SearchQuality::Balanced),
            "an unconfigured collection must answer exactly as Balanced does (query {qi})"
        );
    }
}

/// The bitmap branch restricts what the async index builder still holds to
/// the anchors too: points `upsert_bulk` wrote but the graph has not linked
/// yet are merged into every search, and none outside the anchor set may be
/// returned, however near the query they lie.
#[test]
fn test_anchor_bitmap_search_returns_no_pending_point_outside_its_anchors() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let collection = Collection::create_with_async_builder(
        dir.path().join("v2"),
        DIM,
        DistanceMetric::Cosine,
        crate::collection::streaming::AsyncIndexBuilderConfig {
            merge_threshold: 100_000,
            segment_count: None,
        },
    )
    .expect("test: create a V2 collection");
    let batch = |ids: std::ops::Range<u64>| -> Vec<Point> {
        ids.map(|id| Point::new(id, vector(id), None)).collect()
    };
    collection
        .upsert_bulk(&batch(0..300))
        .expect("test: linked upsert_bulk");
    collection
        .flush()
        .expect("test: flush links the first batch");
    collection
        .upsert_bulk(&batch(300..600))
        .expect("test: pending upsert_bulk");
    let anchors: HashSet<u64> = (0..300).collect();

    let hits = collection
        .search_anchors_bitmap(&vector(450), &anchors, None, K, DistanceMetric::Cosine)
        .expect("test: search_anchors_bitmap");
    let outside: Vec<u64> = hits
        .iter()
        .map(|hit| hit.point.id)
        .filter(|id| !anchors.contains(id))
        .collect();
    assert!(
        outside.is_empty(),
        "pending non-anchors returned: {outside:?}"
    );
    assert_eq!(hits.len(), K);
}
