//! The deferred indexer holds points the HNSW graph does not answer for, as
//! the async index builder does, and every search merges
//! them through the same entry point (`super::pending`). A restricted search
//! must restrict them too: points outside its filter must neither appear nor
//! take the place of a match.
//!
//! Fixture: 300 points upserted the ordinary way (`cat` `"rare"` for ids
//! 0..3, `"linked"` otherwise), then 300 points far nearer the query, `cat`
//! `"pending"`, held by the source under test.

use std::collections::HashMap;

use crate::collection::streaming::DeferredIndexerConfig;
use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::point::{Point, SearchResult};

const DIM: usize = 16;
const BATCH: u64 = 300;
const K: usize = 5;

/// Dispersed values (xorshift), so every point is its own nearest neighbour.
#[allow(clippy::cast_precision_loss)] // the top 24 bits of the state are exact in an f32
fn dispersed(id: u64) -> Vec<f32> {
    let mut state = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32
        })
        .collect()
}

fn query() -> Vec<f32> {
    dispersed(1_000_000)
}

/// A point next to `query`: one coordinate nudged.
#[allow(clippy::cast_precision_loss)] // id % 7 is tiny
fn near_query(id: u64) -> Vec<f32> {
    let mut v = query();
    let i = usize::try_from(id).expect("test: id fits a usize") % DIM;
    v[i] += 1e-3 * (1.0 + (id % 7) as f32);
    v
}

fn linked_points() -> Vec<Point> {
    (0..BATCH)
        .map(|id| {
            let cat = if id < 3 { "rare" } else { "linked" };
            Point::new(id, dispersed(id), Some(serde_json::json!({ "cat": cat })))
        })
        .collect()
}

fn pending_points() -> Vec<Point> {
    (BATCH..2 * BATCH)
        .map(|id| {
            Point::new(
                id,
                near_query(id),
                Some(serde_json::json!({ "cat": "pending" })),
            )
        })
        .collect()
}

/// A collection over the linked points, with a secondary index on `cat`.
fn collection(dir: &tempfile::TempDir) -> Collection {
    let coll = Collection::create(dir.path().join("c"), DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    coll.create_index("cat").expect("test: index cat");
    coll.upsert_bulk(&linked_points()).expect("test: linked");
    coll
}

/// The pending points sit in the deferred indexer's buffer, not the graph.
fn with_deferred_points(dir: &tempfile::TempDir) -> Collection {
    let coll = collection(dir);
    let deferred = DeferredIndexerConfig {
        enabled: true,
        merge_threshold: 100_000,
        ..DeferredIndexerConfig::default()
    };
    coll.apply_advanced_config(None, Some(Some(deferred)), None)
        .expect("test: install the deferred indexer");
    coll.upsert_bulk(&pending_points()).expect("test: deferred");
    let indexer = coll.deferred_indexer().expect("premise: installed");
    assert!(indexer.is_searchable(), "premise: the buffer holds them");
    coll
}

fn cats(results: &[SearchResult]) -> Vec<String> {
    results
        .iter()
        .map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("cat"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?")
                .to_string()
        })
        .collect()
}

/// Runs the restricted searches the fixture's filters select against.
fn assert_restrictions_hold(coll: &Collection, source: &str) {
    let q = query();
    let linked = Filter::new(Condition::eq("cat", "linked"));
    let hits = coll
        .search_with_filter(&q, K, &linked)
        .expect("test: filtered");
    assert_eq!(
        cats(&hits),
        vec!["linked"; K],
        "{source}: search_with_filter"
    );

    let batch = coll
        .search_batch_with_filters(&[q.as_slice()], K, &[Some(linked)])
        .expect("test: batch");
    assert_eq!(cats(&batch[0]), vec!["linked"; K], "{source}: batch");

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!(q));
    let rare = coll
        .execute_query_str(
            "SELECT * FROM c WHERE vector NEAR $v AND cat = 'rare' LIMIT 3 \
             WITH (mode='balanced')",
            &params,
        )
        .expect("test: VelesQL");
    let mut ids: Vec<u64> = rare.iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![0, 1, 2], "{source}: exact prefilter");
}

#[test]
fn a_restricted_search_restricts_the_deferred_indexers_points() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    assert_restrictions_hold(&with_deferred_points(&dir), "deferred indexer");
}

/// The graph-anchored bitmap search admits none of them outside its anchors.
#[test]
fn an_anchored_search_admits_no_deferred_point_outside_its_anchors() {
    let anchors: std::collections::HashSet<u64> = (0..BATCH).collect();
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = with_deferred_points(&dir);
    let hits = coll
        .search_anchors_bitmap(&query(), &anchors, None, K, DistanceMetric::Euclidean)
        .expect("test: anchored");
    let outside: Vec<u64> = hits
        .iter()
        .map(|h| h.point.id)
        .filter(|id| !anchors.contains(id))
        .collect();
    assert!(outside.is_empty(), "non-anchors returned {outside:?}");
}

/// A collection whose `upsert_bulk` takes the async builder's V2 path and
/// whose other writes go through the deferred indexer.
fn with_async_and_deferred(dir: &tempfile::TempDir) -> Collection {
    let coll = Collection::create_with_async_builder(
        dir.path().join("c"),
        DIM,
        DistanceMetric::Euclidean,
        crate::collection::streaming::AsyncIndexBuilderConfig {
            merge_threshold: 100_000,
            segment_count: None,
        },
    )
    .expect("test: create");
    coll.upsert(linked_points()).expect("test: linked");
    let deferred = DeferredIndexerConfig {
        enabled: true,
        merge_threshold: 100_000,
        ..DeferredIndexerConfig::default()
    };
    coll.apply_advanced_config(None, Some(Some(deferred)), None)
        .expect("test: install the deferred indexer");
    coll
}

fn first_id(coll: &Collection, query: &[f32]) -> Option<u64> {
    coll.search(query, 1)
        .expect("test: search")
        .first()
        .map(|hit| hit.point.id)
}

const X: u64 = 10_000;

/// A point bulk-loaded through the async builder, then overwritten through
/// the deferred indexer, is found on what storage holds now: its new vector,
/// not the arena slot of the first write.
#[test]
fn a_point_overwritten_after_its_bulk_load_is_scored_on_its_new_vector() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = with_async_and_deferred(&dir);
    let (old, new) = (dispersed(X), dispersed(X + 1));
    coll.upsert_bulk(&[Point::without_payload(X, old.clone())])
        .expect("test: bulk");
    coll.upsert([Point::without_payload(X, new.clone())])
        .expect("test: overwrite");

    assert_eq!(first_id(&coll, &new), Some(X), "found on its new vector");
    assert_ne!(first_id(&coll, &old), Some(X), "found on its old vector");
}

/// A linked point overwritten through the deferred indexer with a far
/// vector is not found on the index's stale copy, near the query.
#[test]
fn an_index_result_is_masked_by_a_newer_write_the_index_has_not_seen() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = with_async_and_deferred(&dir);
    let stale = dispersed(7);
    coll.upsert([Point::without_payload(7, dispersed(X))])
        .expect("test: overwrite");
    assert_ne!(
        first_id(&coll, &stale),
        Some(7),
        "found on its stale vector"
    );
    assert_eq!(first_id(&coll, &dispersed(X)), Some(7));
}

/// A point deleted and upserted again while the deferred indexer holds its
/// writes is found: the delete does not outlive the write after it.
#[test]
fn a_point_deleted_then_upserted_again_while_deferred_is_found() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = with_async_and_deferred(&dir);
    let v = dispersed(X);
    coll.upsert([Point::without_payload(X, v.clone())])
        .expect("test: upsert");
    coll.delete(&[X]).expect("test: delete");
    assert_ne!(first_id(&coll, &v), Some(X), "premise: deleted");
    coll.upsert([Point::without_payload(X, v.clone())])
        .expect("test: upsert again");
    assert_eq!(first_id(&coll, &v), Some(X));
}

/// A deferred write overwritten through the async builder leaves the
/// deferred buffer at the next merge: the merge indexes what storage holds
/// and retires the entry it copied, though storage no longer holds that
/// entry's vector.
#[test]
fn a_merge_retires_a_deferred_write_storage_has_moved_past() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = with_async_and_deferred(&dir);
    let (old, new) = (dispersed(X), dispersed(X + 1));
    coll.upsert([Point::without_payload(X, old)])
        .expect("test: deferred write");
    coll.upsert_bulk(&[Point::without_payload(X, new.clone())])
        .expect("test: bulk overwrite");

    coll.flush().expect("test: flush merges");

    let deferred = coll.deferred_indexer().expect("premise: installed");
    assert!(deferred.ids().is_empty(), "the entry left the buffer");
    assert_eq!(first_id(&coll, &new), Some(X), "indexed on its new vector");
}

/// Pending ids crowding the index's results take a number of refetches that
/// grows with the logarithm of the snapshot, not linearly: 100 pending ids
/// ranked first, `k = 10`, are passed in at most four refetches (10, 20, 40,
/// 80, then the cap of 110), and the last one holds `k` others.
#[test]
fn crowding_pending_ids_take_logarithmically_many_refetches() {
    use crate::collection::search::pending::PendingSnapshot;
    use crate::scored_result::ScoredResult;

    let k = 10;
    let pending = PendingSnapshot::for_test((0..100).collect());
    let calls = std::cell::Cell::new(0_usize);
    let search = |n: usize| -> crate::error::Result<Vec<ScoredResult>> {
        calls.set(calls.get() + 1);
        Ok((0..u64::try_from(n).expect("test: fits"))
            .map(|id| ScoredResult::new(id, 0.0))
            .collect())
    };

    let first = search(k).expect("test: search");
    let results =
        Collection::index_results_past_pending(&pending, k, first, search).expect("test: refetch");

    let unmasked = results.iter().filter(|r| r.id >= 100).count();
    assert!(unmasked >= k, "{unmasked} results past the pending ids");
    assert!(
        calls.get() - 1 <= 4,
        "{} refetches for 100 pending ids",
        calls.get() - 1
    );
}

/// A batch search scores its pending points on the calling thread, never on
/// a rayon worker: the batch then joins the global pool while holding the
/// `vector_storage` read guard, and a worker taking that guard again parks
/// behind any queued writer, which would stop the whole process.
#[test]
fn a_batch_search_reads_no_storage_on_a_rayon_worker() {
    use crate::collection::search::pending::merge_hooks::STORAGE_READS_ON_RAYON;
    use std::sync::atomic::Ordering;

    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = with_deferred_points(&dir);
    let queries: Vec<Vec<f32>> = (0..16).map(|i| near_query(i * 7)).collect();
    let queries: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();
    let filters = vec![None; queries.len()];
    let before = STORAGE_READS_ON_RAYON.load(Ordering::Relaxed);

    coll.search_batch_parallel(&queries, K)
        .expect("test: batch");
    coll.search_batch_with_filters(&queries, K, &filters)
        .expect("test: filtered batch");
    coll.multi_query_search_ids(
        &queries[..4],
        K,
        crate::fusion::FusionStrategy::RRF { k: 60 },
    )
    .expect("test: multi-query");

    assert_eq!(
        STORAGE_READS_ON_RAYON.load(Ordering::Relaxed),
        before,
        "a batch search read storage on a rayon worker"
    );
}
