//! Each branch of a hybrid search supplies its own rows: a test here asserts
//! what only the branch under test can supply — its component score on every
//! row, or a row the other branch does not rank — so a branch that drops its
//! filter, its anchors, its row check or its depth fails it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::json;

use crate::collection::graph::GraphEdge;
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::point::{Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// Ids 1..=24 are category `a` and rank first in both branches — a short
/// "banana" text, a vector next to the query `[1, 0]` — and ids 25..=30,
/// category `b`, rank below all of them: a long text, a vector far from it.
fn ranked_apart(dir: &Path) -> Collection {
    let collection =
        Collection::create(dir.join("v"), 2, DistanceMetric::Cosine).expect("test: collection");
    collection
        .upsert((1..=30u64).map(|id| {
            let (content, vector, cat) = if id < 25 {
                ("banana".to_string(), vec![1.0, 0.001], "a")
            } else {
                (
                    format!("banana {}", "filler ".repeat(30)),
                    vec![0.0, 1.0],
                    "b",
                )
            };
            Point::new(id, vector, Some(json!({"content": content, "cat": cat})))
        }))
        .expect("test: upsert");
    collection
}

/// The filter decides which rows each branch ranks: every row a filtered
/// hybrid returns is ranked by both branches, which only a filtered vector
/// branch and a filtered text branch can give it.
#[test]
fn both_branches_of_a_filtered_hybrid_rank_only_the_rows_the_filter_matches() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = ranked_apart(dir.path());
    let cat_b = Filter::new(Condition::eq("cat", "b"));

    let hits = collection
        .hybrid_search_with_filter(&[1.0, 0.0], "banana", 2, None, &cat_b, None)
        .expect("test: hybrid search");

    assert_eq!(hits.len(), 2, "{:?}", ids(&hits));
    for hit in &hits {
        let vector = hit.component_score("vector_score").unwrap_or_default();
        let bm25 = hit.component_score("bm25_score").unwrap_or_default();
        assert!(
            vector > 0.0 && bm25 > 0.0,
            "row {} is ranked by both branches: {:?}",
            hit.point.id,
            hit.component_scores
        );
    }
}

/// `A` is the nearest vector without the term, `C` the best text far from
/// the query, `B` second in both, `D` and `E` neither — and `E` the second
/// best text far away. Ids: A 1, B 2, C 3, D 4, E 5.
fn fused(dir: &Path) -> Collection {
    let collection =
        Collection::create(dir.join("f"), 2, DistanceMetric::Cosine).expect("test: collection");
    let rows = [
        (1, [1.0, 0.0], "zz"),
        (2, [0.9, 0.3], "apple qq qq qq"),
        (3, [0.0, 1.0], "apple"),
        (4, [-1.0, 0.0], "yy"),
        (5, [-0.9, -0.3], "apple ww ww ww ww ww"),
    ];
    collection
        .upsert(rows.iter().map(|(id, vector, content)| {
            Point::new(*id, vector.to_vec(), Some(json!({"content": content})))
        }))
        .expect("test: upsert");
    collection
}

/// Each branch fuses its best `2k` rows: at `k = 1`, `B` — second in both —
/// outscores the best of either branch alone, which a branch cut at `k`
/// would never fuse.
#[test]
fn a_hybrid_fuses_twice_k_rows_of_each_branch() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = fused(dir.path());

    let hits = collection
        .hybrid_search(&[1.0, 0.0], "apple", 1, None, None)
        .expect("test: hybrid search");

    assert_eq!(ids(&hits), [2]);
}

/// A BM25 document for an id that is no row — what an earlier version let a
/// vector collection store without a vector — takes no row's place among
/// the `k`, and the walk stops at `k` and keeps to the ids `within` takes.
#[test]
fn the_bm25_branch_admits_k_rows_of_the_collection_within_its_restriction() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = fused(dir.path());
    collection
        .storage
        .text_index
        .add_document(9001, "apple apple");

    let best: Vec<u64> = collection
        .bm25_rows("apple", 2, RowUniverse::Vectors)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(best, [3, 2]);
    let within: Vec<u64> = collection
        .bm25_rows_where("apple", 5, RowUniverse::Vectors, |id| id == 5, None)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(within, [5]);
}

/// Both branches of an anchored hybrid rank the anchors only: the best text
/// and the nearest vector outside them are never fused.
#[test]
fn an_anchored_hybrid_fuses_only_its_anchors() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = fused(dir.path());
    let anchors = HashSet::from([4, 5]);

    let hits = collection
        .hybrid_search_with_anchors_clause(&[1.0, 0.0], "apple", 2, None, &anchors, None)
        .expect("test: anchored hybrid");

    let mut found = ids(&hits);
    found.sort_unstable();
    assert_eq!(found, [4, 5]);
    let text = hits.iter().find(|hit| hit.point.id == 5).expect("test: 5");
    assert!(
        text.component_score("bm25_score").unwrap_or_default() > 0.0,
        "{:?}",
        text.component_scores
    );
}

/// The metadata conditions of an anchored hybrid admit its rows in both
/// branches: the matching anchors ranked below the `LIMIT` best of the
/// anchor set are fused, not fetched and filtered away.
#[test]
fn an_anchored_hybrid_admits_its_rows_by_the_metadata_conditions() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection =
        Collection::create(dir.path().join("k"), 2, DistanceMetric::Cosine).expect("test: create");
    collection
        .upsert((1..=40u64).map(|id| {
            #[allow(clippy::cast_precision_loss)]
            let angle = id as f32 * 0.03;
            let cat = if id > 35 { "a" } else { "b" };
            Point::new(
                id,
                vec![angle.cos(), angle.sin()],
                Some(json!({"content": "apple fruit", "cat": cat})),
            )
        }))
        .expect("test: upsert");
    for id in 1..=40u64 {
        let target = if id == 1 { 2 } else { 1 };
        collection
            .add_edge(GraphEdge::new(1000 + id, id, target, "REL").expect("test: edge"))
            .expect("test: add edge");
    }
    let params = HashMap::from([("v".to_string(), json!([1.0, 0.0]))]);

    for fusion in ["", " USING FUSION(strategy = 'maximum')"] {
        let sql = format!(
            "SELECT * FROM k WHERE vector NEAR $v AND content MATCH 'apple' AND cat = 'a' \
             AND MATCH (d)-[:REL]->(x) LIMIT 2{fusion}"
        );
        let hits = collection
            .execute_query_str(&sql, &params)
            .expect("test: query");
        assert_eq!(hits.len(), 2, "{sql}: {:?}", ids(&hits));
        assert!(
            ids(&hits).iter().all(|id| *id > 35),
            "{sql}: {:?}",
            ids(&hits)
        );
    }
}

/// The collection's sparse searches rank a tie as the index does: by
/// ascending id, `VectorCollection::sparse_search` and `SPARSE_NEAR` alike.
#[test]
fn a_sparse_search_ranks_a_tie_by_ascending_id() {
    use std::collections::BTreeMap;

    use crate::index::sparse::SparseVector;

    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection =
        Collection::create(dir.path().join("s"), 2, DistanceMetric::Cosine).expect("test: create");
    collection
        .upsert([5_u64, 3, 1, 4, 2].into_iter().map(|id| {
            let sparse = BTreeMap::from([(String::new(), SparseVector::new(vec![(7, 1.0)]))]);
            Point::with_sparse(id, vec![1.0, 0.0], None, Some(sparse))
        }))
        .expect("test: upsert");

    let hits = collection
        .sparse_hits("", &SparseVector::new(vec![(7, 1.0)]), 3, None, None)
        .expect("test: sparse search");
    assert_eq!(ids(&hits), [1, 2, 3]);
    let params = HashMap::from([("q".to_string(), json!({"7": 1.0}))]);
    let hits = collection
        .execute_query_str(
            "SELECT * FROM s WHERE vector SPARSE_NEAR $q LIMIT 3",
            &params,
        )
        .expect("test: SPARSE_NEAR");
    assert_eq!(ids(&hits), [1, 2, 3]);
}
