//! A filtered text or sparse search is exact: its filter is part of the
//! admission of the ranked walk, so a matching row is found however far
//! below the best it ranks — no window of the best few is filtered alone.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::{json, Value};

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::index::sparse::SparseVector;
use crate::point::{Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn cat_b() -> Filter {
    Filter::new(Condition::eq("cat", "b"))
}

/// Ids 1..=24 are category `a` and rank first in every branch — a short
/// "banana" text, a vector next to the query `[1, 0]`, a heavy sparse
/// weight — and ids 25..=30, category `b`, rank below all of them: a long
/// text, a vector far from the query, a light sparse weight.
fn point(id: u64) -> Point {
    let (content, vector, cat) = if id < 25 {
        ("banana".to_string(), vec![1.0, 0.001], "a")
    } else {
        (
            format!("banana {}", "filler ".repeat(30)),
            vec![0.0, 1.0],
            "b",
        )
    };
    #[allow(clippy::cast_precision_loss)]
    let sparse = SparseVector::new(vec![(1, 100.0 - id as f32)]);
    Point {
        id,
        vector,
        payload: Some(json!({"content": content, "cat": cat})),
        sparse_vectors: Some(BTreeMap::from([(String::new(), sparse)])),
    }
}

fn vectors(dir: &Path) -> Collection {
    let collection = Collection::create(dir.join("v"), 2, DistanceMetric::Cosine)
        .expect("test: vector collection");
    collection
        .upsert((1..=30).map(point))
        .expect("test: upsert");
    collection
}

fn metadata(dir: &Path) -> Collection {
    let collection =
        Collection::create_metadata_only(dir.join("m"), "m").expect("test: metadata collection");
    collection
        .upsert((1..=30).map(|id| Point {
            vector: Vec::new(),
            ..point(id)
        }))
        .expect("test: upsert");
    collection
}

fn sparse_params() -> HashMap<String, Value> {
    HashMap::from([("q".to_string(), json!({"indices": [1], "values": [1.0]}))])
}

#[test]
fn a_filtered_text_search_finds_matches_ranked_below_the_best() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());

    let hits = collection
        .text_search_with_filter("banana", 5, &cat_b())
        .expect("test: text search");

    assert_eq!(hits.len(), 5, "five rows match: {:?}", ids(&hits));
    assert!(ids(&hits).iter().all(|&id| id >= 25));
}

#[test]
fn a_filtered_sparse_query_finds_matches_ranked_below_the_best() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    for collection in [vectors(dir.path()), metadata(dir.path())] {
        let hits = collection
            .execute_query_str(
                "SELECT * FROM v WHERE vector SPARSE_NEAR $q AND cat = 'b' LIMIT 2",
                &sparse_params(),
            )
            .expect("test: sparse query");
        assert_eq!(ids(&hits), [25, 26]);
    }
}

#[test]
fn a_filtered_hybrid_text_search_fuses_matches_ranked_below_the_best() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());

    let hits = collection
        .hybrid_search_with_filter(&[1.0, 0.0], "banana", 2, None, &cat_b(), None)
        .expect("test: hybrid search");

    assert_eq!(hits.len(), 2, "two rows match: {:?}", ids(&hits));
    assert!(ids(&hits).iter().all(|&id| id >= 25));
}

#[test]
fn a_filtered_score_fused_hybrid_query_fuses_matches_ranked_below_the_best() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    let params = HashMap::from([("v".to_string(), json!([1.0, 0.0]))]);

    let hits = collection
        .execute_query_str(
            "SELECT * FROM v WHERE vector NEAR $v AND content MATCH 'banana' AND cat = 'b' \
             LIMIT 2 USING FUSION(strategy = 'maximum')",
            &params,
        )
        .expect("test: hybrid query");

    assert_eq!(hits.len(), 2, "two rows match: {:?}", ids(&hits));
    assert!(ids(&hits).iter().all(|&id| id >= 25));
}

#[test]
fn a_filtered_dense_sparse_query_fuses_matches_ranked_below_the_best() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    let mut params = sparse_params();
    params.insert("v".to_string(), json!([1.0, 0.0]));

    let hits = collection
        .execute_query_str(
            "SELECT * FROM v WHERE vector NEAR $v AND vector SPARSE_NEAR $q AND cat = 'b' \
             LIMIT 2",
            &params,
        )
        .expect("test: hybrid query");

    assert_eq!(hits.len(), 2, "two rows match: {:?}", ids(&hits));
    assert!(ids(&hits).iter().all(|&id| id >= 25));
}
