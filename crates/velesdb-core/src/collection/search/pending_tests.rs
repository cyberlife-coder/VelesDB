//! The delta buffer and the deferred indexer hold points the HNSW graph does
//! not answer for, as the async index builder does, and every search merges
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

/// The pending points are stored and, as during a rebuild, held by the
/// delta buffer rather than the index. The deferred indexer is how they get
/// stored without an index insert; its buffer is then emptied, unmerged.
fn with_delta_points(dir: &tempfile::TempDir) -> Collection {
    let coll = with_deferred_points(dir);
    let held = coll
        .deferred_indexer()
        .expect("premise: installed")
        .drain_all();
    assert_eq!(
        held.len(),
        usize::try_from(BATCH).expect("test: fits"),
        "premise"
    );
    coll.streaming.delta_buffer.activate();
    coll.push_to_delta_if_active(&held);
    assert!(coll.streaming.delta_buffer.is_searchable(), "premise");
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

#[test]
fn a_restricted_search_restricts_the_delta_buffers_points() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    assert_restrictions_hold(&with_delta_points(&dir), "delta buffer");
}

/// The graph-anchored bitmap search admits none of them outside its anchors.
#[test]
fn an_anchored_search_admits_no_delta_or_deferred_point_outside_its_anchors() {
    let anchors: std::collections::HashSet<u64> = (0..BATCH).collect();
    for (source, build) in [
        (
            "deferred indexer",
            with_deferred_points as fn(&tempfile::TempDir) -> Collection,
        ),
        ("delta buffer", with_delta_points),
    ] {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let coll = build(&dir);
        let hits = coll
            .search_anchors_bitmap(&query(), &anchors, None, K, DistanceMetric::Euclidean)
            .expect("test: anchored");
        let outside: Vec<u64> = hits
            .iter()
            .map(|h| h.point.id)
            .filter(|id| !anchors.contains(id))
            .collect();
        assert!(
            outside.is_empty(),
            "{source}: non-anchors returned {outside:?}"
        );
    }
}
