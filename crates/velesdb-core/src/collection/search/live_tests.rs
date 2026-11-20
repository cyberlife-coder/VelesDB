//! Every vector read surface ranks live rows: an expired row nearest the
//! query takes no place among the `k`, on every variant, filtered or not,
//! batched or fused — and past the bound on asking the index again, the read
//! scans exactly.

use std::collections::HashSet;

use serde_json::json;

use super::{exact_scans, on_index_answered, Nearest, MAX_ASK};
use crate::collection::search::query::QuerySearchOptions;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::point::{Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

const QUERY: [f32; 2] = [1.0, 0.0];

/// The first live id: every id below it is expired.
const LIVE: u64 = 10_000;

/// `expired` rows nearest `QUERY` — ids `1..=expired` — then `live` rows
/// farther away, ids from [`LIVE`], the nearer the lower; every row holds
/// `"k": 1` and `"content": "apple"`.
fn crowded(dir: &std::path::Path, expired: u64, live: u64) -> Collection {
    let collection =
        Collection::create(dir.join("c"), 2, DistanceMetric::Cosine).expect("test: collection");
    let mut points: Vec<Point> = (1..=expired)
        .map(|id| {
            #[allow(clippy::cast_precision_loss)]
            let vector = vec![1.0, 0.0001 * id as f32];
            let payload = json!({"k": 1, "content": "apple", "_veles_expires_at": 1});
            Point::new(id, vector, Some(payload))
        })
        .collect();
    points.extend((0..live).map(|i| {
        #[allow(clippy::cast_precision_loss)]
        let vector = vec![1.0, 0.5 + 0.01 * i as f32];
        Point::new(LIVE + i, vector, Some(json!({"k": 1, "content": "apple"})))
    }));
    collection.upsert(points).expect("test: upsert");
    collection
}

fn k_is_one() -> Filter {
    Filter::new(Condition::eq("k", 1))
}

/// The `k` nearest live ids.
fn nearest_live(k: u64) -> Vec<u64> {
    (LIVE..LIVE + k).collect()
}

/// 60 expired rows nearest the query: more than `k`, than a hybrid's dense
/// depth, than a filtered batch's `4k` and than a filter's oversampling —
/// every variant must ask the index again, with what it asked for.
const EXPIRED: u64 = 60;

#[test]
fn every_dense_variant_returns_the_k_nearest_live_rows() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = crowded(dir.path(), EXPIRED, 100);
    let options = |force_rerank| QuerySearchOptions {
        quality: Some(crate::SearchQuality::Balanced),
        force_rerank,
        ..QuerySearchOptions::default()
    };
    let scans = exact_scans();

    for (name, hits) in [
        ("search", collection.search(&QUERY, 3)),
        ("search_with_ef", collection.search_with_ef(&QUERY, 3, 16)),
        (
            "search_with_quality",
            collection.search_with_quality(&QUERY, 3, crate::SearchQuality::Fast),
        ),
        (
            "forced rerank",
            collection.search_with_opts(&QUERY, 3, &options(Some(true))),
        ),
        (
            "no rerank",
            collection.search_with_opts(&QUERY, 3, &options(Some(false))),
        ),
        (
            "search_with_filter",
            collection.search_with_filter(&QUERY, 3, &k_is_one()),
        ),
        (
            "filtered with quality",
            collection.search_with_filter_and_opts(&QUERY, 3, &k_is_one(), &options(None)),
        ),
        (
            "hybrid without a text hit",
            collection.hybrid_search(&QUERY, "zzzz", 3, None, None),
        ),
    ] {
        assert_eq!(ids(&hits.expect("test: search")), nearest_live(3), "{name}");
    }
    let only_ids: Vec<u64> = collection
        .search_ids(&QUERY, 3)
        .expect("test: search_ids")
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(only_ids, nearest_live(3), "search_ids");
    assert_eq!(exact_scans(), scans, "asking the index again sufficed");
}

#[test]
fn every_batched_and_fused_variant_returns_live_rows() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = crowded(dir.path(), EXPIRED, 100);
    let queries: [&[f32]; 2] = [&QUERY, &QUERY];
    let rrf = || crate::fusion::FusionStrategy::RRF { k: 60 };

    let batch = collection
        .search_batch_parallel(&queries, 3)
        .expect("test: batch");
    let filtered = collection
        .search_batch_with_filters(&queries, 3, &[None, Some(k_is_one())])
        .expect("test: filtered batch");
    for hits in batch.iter().chain(&filtered) {
        assert_eq!(ids(hits), nearest_live(3));
    }
    let fused = collection
        .multi_query_search(&queries, 3, rrf(), None)
        .expect("test: multi query");
    let fused_filtered = collection
        .multi_query_search(&queries, 3, rrf(), Some(&k_is_one()))
        .expect("test: filtered multi query");
    assert_eq!(ids(&fused), nearest_live(3));
    assert_eq!(ids(&fused_filtered), nearest_live(3));
    let fused_ids: Vec<u64> = collection
        .multi_query_search_ids(&queries, 3, rrf())
        .expect("test: multi query ids")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(fused_ids, nearest_live(3));
}

/// `NEAR … AND` a filter runs the filtered search: past the expired rows.
#[test]
fn a_near_query_with_a_filter_returns_live_rows() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = crowded(dir.path(), EXPIRED, 100);
    let query =
        crate::velesql::Parser::parse("SELECT * FROM c WHERE vector NEAR $v AND k = 1 LIMIT 3")
            .expect("test: parse");
    let params = std::collections::HashMap::from([("v".to_string(), json!(QUERY))]);

    let hits = collection
        .execute_query(&query, &params)
        .expect("test: query");

    assert_eq!(ids(&hits), nearest_live(3));
}

/// Expired rows crowding the index's answers past the bound make the read
/// scan every stored vector exactly, once: the exact nearest live rows,
/// filtered or not.
#[test]
fn past_the_bound_a_read_scans_exactly() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = crowded(dir.path(), 150, 10);

    let scans = exact_scans();
    let hits = collection.search(&QUERY, 3).expect("test: search");
    assert_eq!(exact_scans(), scans + 1, "the read did not scan exactly");
    let filtered = collection
        .search_with_filter(&QUERY, 2, &k_is_one())
        .expect("test: filtered search");
    assert_eq!(
        exact_scans(),
        scans + 2,
        "the filtered read did not scan exactly"
    );

    assert_eq!(ids(&hits), nearest_live(3));
    assert_eq!(ids(&filtered), nearest_live(2));
    let scores: Vec<f32> = hits.iter().map(|r| r.score).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
}

/// The bound on asking again: twice as many, never past `MAX_ASK` nor up to
/// the stored vectors.
#[test]
fn asking_again_doubles_until_the_bound() {
    assert_eq!(Nearest::next_ask(10, 1_000), Some(20));
    assert_eq!(Nearest::next_ask(10, 20), None);
    assert_eq!(Nearest::next_ask(10, 21), Some(20));
    assert_eq!(Nearest::next_ask(MAX_ASK / 2, usize::MAX), Some(MAX_ASK));
    assert_eq!(Nearest::next_ask(MAX_ASK / 2 + 1, usize::MAX), None);
}

/// Every live row is returned when the collection holds fewer than `k`.
#[test]
fn a_read_for_more_than_the_live_rows_returns_them_all() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = crowded(dir.path(), 30, 5);

    let hits = collection.search(&QUERY, 20).expect("test: search");

    let live: HashSet<u64> = nearest_live(5).into_iter().collect();
    assert_eq!(ids(&hits).into_iter().collect::<HashSet<_>>(), live);
}

/// A row the index answered for and a delete removed before the answer is
/// judged is dead: it takes no place among the `k`, the next live row does.
#[test]
fn a_row_deleted_after_the_index_answered_takes_no_place() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = crowded(dir.path(), 0, 10);
    let writer = collection.clone();
    on_index_answered(move || writer.delete(&[LIVE]).expect("test: delete"));

    let hits = collection.search(&QUERY, 2).expect("test: search");

    assert_eq!(ids(&hits), [LIVE + 1, LIVE + 2]);
}
