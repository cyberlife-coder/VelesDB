use super::*;

/// #901: a `NOT similarity()` scan within the server ceiling is allowed.
#[test]
fn test_not_similarity_guard_allows_within_ceiling() {
    assert!(Collection::guard_not_similarity_scan(Collection::NOT_SIMILARITY_MAX_SCAN).is_ok());
    assert!(Collection::guard_not_similarity_scan(10_000).is_ok());
}

/// #901: a `NOT similarity()` scan over the server ceiling is REJECTED
/// (not merely warned) to block the unbounded-scan DoS vector.
#[test]
fn test_not_similarity_guard_rejects_above_ceiling() {
    let err = Collection::guard_not_similarity_scan(Collection::NOT_SIMILARITY_MAX_SCAN + 1)
        .expect_err("scan above ceiling must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("scan limit") || msg.contains("exceeding"),
        "error should explain the scan-limit rejection, got: {msg}"
    );
}

// ============================================================================
// Regression (#2101): the GraphFirst exact-rescoring scan clamped every
// metric's score into [-1, 1]. Euclidean/Hamming distances > 1 and raw dot
// products > 1 all collapsed to an artificial 1.0 tie, so the "top-k" was
// insertion-order arbitrary and the user-visible score meaningless. The
// clamp also failed its stated purpose: clamp() propagates NaN.
// ============================================================================

#[cfg(feature = "persistence")]
mod scan_score_semantics {
    use crate::collection::Collection;
    use crate::distance::DistanceMetric;
    use crate::filter::{Condition, Filter};
    use crate::point::Point;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn tagged_point(id: u64, vector: Vec<f32>) -> Point {
        Point {
            id,
            vector,
            payload: Some(serde_json::json!({"cat": "a"})),
            sparse_vectors: None,
        }
    }

    fn setup(metric: DistanceMetric, points: Vec<Point>) -> (TempDir, Collection) {
        let dir = tempfile::tempdir().expect("test: tempdir");
        let col =
            Collection::create(PathBuf::from(dir.path()), 2, metric).expect("test: collection");
        col.upsert(points).expect("test: upsert");
        (dir, col)
    }

    fn cat_filter() -> Filter {
        Filter::new(Condition::Eq {
            field: "cat".into(),
            value: serde_json::json!("a"),
        })
    }

    /// Euclidean distances above 1 must be reported verbatim and ranked
    /// ascending — pre-fix all three clamped to 1.0 (total tie).
    #[test]
    fn test_scan_score_euclidean_distances_above_one_not_clamped() {
        let (_dir, col) = setup(
            DistanceMetric::Euclidean,
            vec![
                tagged_point(1, vec![7.0, 0.0]),
                tagged_point(2, vec![3.0, 0.0]),
                tagged_point(3, vec![10.0, 0.0]),
            ],
        );

        let results = col.scan_and_score_by_vector(&cat_filter(), &[0.0, 0.0], 3);

        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert_eq!(ids, vec![2, 1, 3], "ascending true distance 3 < 7 < 10");
        for (result, want) in results.iter().zip([3.0f32, 7.0, 10.0]) {
            assert!(
                (result.score - want).abs() < 1e-5,
                "id {} score {} != true distance {want}",
                result.point.id,
                result.score
            );
        }
    }

    /// Raw dot products above 1 must be reported verbatim and ranked
    /// descending — pre-fix both clamped to 1.0.
    #[test]
    fn test_scan_score_dot_product_above_one_not_clamped() {
        let (_dir, col) = setup(
            DistanceMetric::DotProduct,
            vec![
                tagged_point(1, vec![2.0, 0.0]),
                tagged_point(2, vec![5.0, 3.0]),
            ],
        );

        let results = col.scan_and_score_by_vector(&cat_filter(), &[1.0, 0.0], 2);

        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert_eq!(ids, vec![2, 1], "descending raw dot 5.0 > 2.0");
        assert!(
            (results[0].score - 5.0).abs() < 1e-5,
            "raw dot 5.0 must survive, got {}",
            results[0].score
        );
    }

    /// A NaN-bearing stored vector must sort last (worst key), never first —
    /// the old clamp let NaN through into the top-k comparator.
    #[test]
    fn test_scan_score_nan_vector_sorts_last() {
        let (_dir, col) = setup(
            DistanceMetric::Euclidean,
            vec![
                tagged_point(1, vec![3.0, 0.0]),
                tagged_point(2, vec![f32::NAN, 0.0]),
                tagged_point(3, vec![7.0, 0.0]),
            ],
        );

        let results = col.scan_and_score_by_vector(&cat_filter(), &[0.0, 0.0], 3);

        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert_eq!(
            ids,
            vec![1, 3, 2],
            "NaN point must rank after every finite distance"
        );
        assert!(
            results[2].score.is_infinite(),
            "NaN maps to the metric's worst key (INFINITY for distances)"
        );
    }

    // ========================================================================
    // Regression (#2106 item 2): a query vector whose length doesn't match
    // the stored vector can't be scored, but the old fallback reported 0.0 —
    // a perfect match for a distance metric like Euclidean. That let a
    // malformed `similarity() < threshold` query pass a mismatched vector as
    // the best possible candidate instead of excluding it.
    // ========================================================================

    /// `filter_by_similarity` must exclude a candidate it can't score against
    /// a length-mismatched query vector, not pass it as a fabricated match.
    #[test]
    fn test_filter_by_similarity_excludes_length_mismatched_query_vector() {
        let (_dir, col) = setup(
            DistanceMetric::Euclidean,
            vec![tagged_point(1, vec![3.0, 0.0])],
        );
        let candidates = vec![crate::point::SearchResult::new(
            tagged_point(1, vec![3.0, 0.0]),
            3.0,
        )];

        // 3-dim query against a 2-dim collection. `similarity() > 0.9` on a
        // distance metric inverts to "distance < 0.9", so the fabricated 0.0
        // used to sail through as the most similar point there is.
        let results = col.filter_by_similarity(
            candidates,
            "vector",
            &[0.0, 0.0, 0.0],
            crate::velesql::CompareOp::Gt,
            0.9,
            10,
        );

        assert!(
            results.is_empty(),
            "a length-mismatched query vector must never pass as a match, got {results:?}"
        );
    }

    /// The `NOT similarity()` scan must likewise exclude, not fabricate a
    /// score for, a candidate whose vector length doesn't match the query.
    #[test]
    fn test_not_similarity_scan_excludes_length_mismatched_vector() {
        let (_dir, col) = setup(
            DistanceMetric::Euclidean,
            vec![tagged_point(1, vec![3.0, 0.0])],
        );

        // NOT similarity(v, $v) < 0.1, queried with a 3-dim vector against
        // the 2-dim collection. Pre-fix, the mismatched point fabricated a
        // 0.0 distance — which passed the `< 0.1` inner threshold and so was
        // wrongly *excluded* by the NOT. It can't be scored either way, so
        // the correct outcome is exclusion from the result set entirely.
        let condition =
            crate::velesql::Condition::Similarity(crate::velesql::SimilarityCondition {
                field: "vector".to_string(),
                vector: crate::velesql::VectorExpr::Literal(vec![0.0, 0.0, 0.0]),
                operator: crate::velesql::CompareOp::Lt,
                threshold: 0.1,
            });
        let condition = crate::velesql::Condition::Not(Box::new(condition));

        let results = col
            .execute_not_similarity_query_over(
                &condition,
                &std::collections::HashMap::new(),
                10,
                None,
            )
            .expect("test: NOT similarity scan");

        assert!(
            results.is_empty(),
            "a length-mismatched vector can't be scored, so it must not appear \
             on either side of NOT similarity(); got {results:?}"
        );
    }
}
