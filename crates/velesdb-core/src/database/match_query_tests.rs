//! [`Database::match_query`]'s query vector and threshold: an absent
//! threshold keeps every scored row under any metric, a present one keeps
//! the rows on its metric's side of it, and a non-finite one is refused.

use std::collections::HashMap;

use super::*;
use crate::point::Point;
use crate::velesql::Parser;
use crate::DistanceMetric;
use tempfile::tempdir;

/// A database whose collection `docs`, under `metric`, holds one point per
/// vector, ids from 1.
fn seeded(metric: DistanceMetric, vectors: &[[f32; 2]]) -> (tempfile::TempDir, Database) {
    let dir = tempdir().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_collection("docs", 2, metric)
        .expect("test: create");
    if vectors.is_empty() {
        return (dir, db);
    }
    let points: Vec<_> = (1_u64..)
        .zip(vectors)
        .map(|(id, vector)| Point::new(id, vector.to_vec(), None))
        .collect();
    db.get_vector_collection("docs")
        .expect("test: collection")
        .upsert(points)
        .expect("test: upsert");
    (dir, db)
}

/// The scores of `MATCH (n) RETURN n` on `docs` against `[1, 0]`, in row order.
fn scores(db: &Database, threshold: Option<f32>) -> Result<Vec<f32>> {
    let query = Parser::parse("MATCH (n) RETURN n LIMIT 10").expect("test: parse");
    let clause = query.match_clause.expect("test: match clause");
    let scoring = MatchScoring {
        vector: &[1.0, 0.0],
        threshold,
    };
    let rows = db.match_query("docs", &clause, &HashMap::new(), Some(scoring))?;
    Ok(rows
        .into_iter()
        .map(|row| row.score.expect("test: a scored row"))
        .collect())
}

/// Points at distance 0, 3 and 5 from the query `[1, 0]`.
const DISTANCES: [[f32; 2]; 3] = [[1.0, 0.0], [4.0, 0.0], [1.0, 5.0]];

/// Points at cosine 1, 0 and -1 to the query `[1, 0]`.
const COSINES: [[f32; 2]; 3] = [[2.0, 0.0], [0.0, 1.0], [-1.0, 0.0]];

#[test]
fn no_threshold_keeps_every_row_under_a_distance_metric() {
    let (_dir, db) = seeded(DistanceMetric::Euclidean, &DISTANCES);
    assert_eq!(scores(&db, None).expect("test: match"), vec![0.0, 3.0, 5.0]);
}

#[test]
fn no_threshold_keeps_every_row_under_a_similarity_metric() {
    let (_dir, db) = seeded(DistanceMetric::Cosine, &COSINES);
    let scores = scores(&db, None).expect("test: match");
    assert_eq!(scores.len(), 3, "{scores:?}");
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
}

#[test]
fn a_distance_threshold_keeps_the_rows_at_most_that_far() {
    let (_dir, db) = seeded(DistanceMetric::Euclidean, &DISTANCES);
    assert_eq!(scores(&db, Some(4.0)).expect("test: match"), vec![0.0, 3.0]);
    assert_eq!(scores(&db, Some(3.0)).expect("test: match"), vec![0.0, 3.0]);
}

#[test]
fn a_similarity_threshold_keeps_the_rows_at_least_that_close() {
    let (_dir, db) = seeded(DistanceMetric::Cosine, &COSINES);
    let kept = scores(&db, Some(0.5)).expect("test: match");
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!((kept[0] - 1.0).abs() < 1e-6, "{kept:?}");
}

#[test]
fn a_non_finite_threshold_is_refused() {
    let (_dir, db) = seeded(DistanceMetric::Euclidean, &DISTANCES);
    for threshold in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let err = scores(&db, Some(threshold)).expect_err("test: refused");
        assert!(
            matches!(&err, crate::Error::Query(message) if message.contains("threshold")),
            "{threshold}: {err}"
        );
    }
}

/// The collection-level entry point refuses it too, before any row is read:
/// an empty collection has nothing to score, and is refused all the same.
#[test]
fn a_non_finite_threshold_is_refused_on_the_collection_entry_point() {
    let (_dir, db) = seeded(DistanceMetric::Euclidean, &[]);
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let query = Parser::parse("MATCH (n) RETURN n LIMIT 10").expect("test: parse");
    let clause = query.match_clause.expect("test: match clause");
    let result = collection.execute_match_with_similarity(
        &clause,
        &[1.0, 0.0],
        Some(f32::NAN),
        &HashMap::new(),
    );
    assert!(
        matches!(&result, Err(crate::Error::Query(message)) if message.contains("threshold")),
        "{result:?}"
    );
}

/// [`MatchScoring::new`] is the one rule a surface builds its scoring from:
/// a finite threshold with a vector scores, no threshold keeps every scored
/// row, and no vector scores nothing.
#[test]
fn match_scoring_new_builds_the_scoring_of_a_vector_and_its_threshold() {
    let vector = [1.0_f32, 0.0];
    for threshold in [None, Some(0.5), Some(4.0), Some(-1.0)] {
        let scoring = MatchScoring::new(Some(&vector), threshold)
            .expect("test: a finite threshold is accepted")
            .expect("test: a vector scores the rows");
        assert_eq!(scoring.vector, &vector);
        assert_eq!(scoring.threshold, threshold);
    }
    assert!(MatchScoring::new(None, None)
        .expect("test: nothing to score")
        .is_none());
}

/// A threshold that is not a finite number is refused, with or without a
/// vector; a finite one without a vector is refused too: no row is scored,
/// so it would be dropped without a word.
#[test]
fn match_scoring_new_refuses_a_threshold_it_cannot_apply() {
    let vector = [1.0_f32, 0.0];
    for (query_vector, threshold) in [
        (Some(&vector[..]), f32::NAN),
        (Some(&vector[..]), f32::INFINITY),
        (None, f32::NEG_INFINITY),
        (None, 0.5),
    ] {
        let err = MatchScoring::new(query_vector, Some(threshold))
            .expect_err("test: the threshold cannot apply");
        assert!(
            matches!(&err, crate::Error::Query(message) if message.contains("threshold")),
            "{threshold} with vector {query_vector:?}: {err}"
        );
        assert_eq!(err.code(), "VELES-010", "{err}");
    }
}
