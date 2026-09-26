//! Tests for `extraction` module - Query condition extraction utilities.

use crate::collection::types::Collection;
use crate::sparse_index::SparseVector;
use crate::velesql::{
    CompareOp, Comparison, Condition, FusionConfig, MatchCondition, Parser, SimilarityCondition,
    SparseVectorExpr, SparseVectorSearch, Value, VectorExpr, VectorFusedSearch, VectorSearch,
};

fn make_comparison(column: &str, val: i64) -> Condition {
    Condition::Comparison(Comparison {
        column: column.to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(val),
    })
}

fn make_match(column: &str, query: &str) -> Condition {
    Condition::Match(MatchCondition {
        column: column.to_string(),
        query: query.to_string(),
    })
}

fn make_similarity(field: &str, threshold: f64) -> Condition {
    Condition::Similarity(SimilarityCondition {
        field: field.to_string(),
        vector: VectorExpr::Parameter("v".to_string()),
        operator: CompareOp::Gt,
        threshold,
    })
}

fn make_vector_search() -> Condition {
    Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    })
}

fn make_sparse_vector_search() -> Condition {
    Condition::SparseVectorSearch(SparseVectorSearch {
        vector: SparseVectorExpr::Literal(SparseVector::new(vec![(1, 0.5)])),
        index_name: None,
    })
}

fn make_graph_match() -> Condition {
    let query = Parser::parse("SELECT * FROM docs WHERE MATCH (d:Doc)-[:REFERENCES]->(x)").unwrap();
    match query.select.where_clause {
        Some(condition) => condition,
        None => panic!("expected where clause"),
    }
}

#[test]
fn test_extract_match_query_direct() {
    let cond = make_match("text", "hello world");
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("hello world".to_string()));
}

#[test]
fn test_extract_match_query_in_and() {
    let cond = Condition::And(
        Box::new(make_comparison("a", 1)),
        Box::new(make_match("text", "search term")),
    );
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("search term".to_string()));
}

#[test]
fn test_extract_match_query_in_group() {
    let cond = Condition::Group(Box::new(make_match("text", "query")));
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("query".to_string()));
}

#[test]
fn test_extract_match_query_none() {
    let cond = make_comparison("a", 1);
    let result = Collection::extract_match_query(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_match_query_nested_and() {
    let inner = Condition::And(
        Box::new(make_match("text", "inner query")),
        Box::new(make_comparison("b", 2)),
    );
    let cond = Condition::And(Box::new(make_comparison("a", 1)), Box::new(inner));
    let result = Collection::extract_match_query(&cond);
    assert_eq!(result, Some("inner query".to_string()));
}

#[test]
fn test_extract_metadata_filter_comparison() {
    let cond = make_comparison("category", 1);
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Comparison(_))));
    if let Some(Condition::Comparison(c)) = result {
        assert_eq!(c.column, "category");
    }
}

#[test]
fn test_extract_metadata_filter_removes_similarity() {
    let cond = make_similarity("embedding", 0.8);
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_removes_vector_search() {
    let cond = make_vector_search();
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_and_with_similarity() {
    let cond = Condition::And(
        Box::new(make_similarity("embedding", 0.8)),
        Box::new(make_comparison("category", 1)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_some());
    assert!(matches!(result, Some(Condition::Comparison(_))));
}

#[test]
fn test_extract_metadata_filter_and_both_metadata() {
    let cond = Condition::And(
        Box::new(make_comparison("a", 1)),
        Box::new(make_comparison("b", 2)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::And(_, _))));
}

#[test]
fn test_extract_metadata_filter_and_both_similarity() {
    let cond = Condition::And(
        Box::new(make_similarity("e1", 0.8)),
        Box::new(make_similarity("e2", 0.9)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_or_both_metadata() {
    let cond = Condition::Or(
        Box::new(make_comparison("a", 1)),
        Box::new(make_comparison("b", 2)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Or(_, _))));
}

#[test]
fn test_extract_metadata_filter_or_with_similarity_returns_none() {
    let cond = Condition::Or(
        Box::new(make_similarity("embedding", 0.8)),
        Box::new(make_comparison("category", 1)),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_group() {
    let cond = Condition::Group(Box::new(make_comparison("a", 1)));
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Group(_))));
}

#[test]
fn test_extract_metadata_filter_not() {
    let cond = Condition::Not(Box::new(make_comparison("deleted", 1)));
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Not(_))));
}

#[test]
fn test_extract_metadata_filter_not_similarity_returns_none() {
    let cond = Condition::Not(Box::new(make_similarity("embedding", 0.8)));
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_removes_graph_match() {
    let cond = make_graph_match();
    let result = Collection::extract_metadata_filter(&cond);
    assert!(result.is_none());
}

#[test]
fn test_extract_metadata_filter_and_with_graph_match() {
    let cond = Condition::And(
        Box::new(make_comparison("category", 1)),
        Box::new(make_graph_match()),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(matches!(result, Some(Condition::Comparison(_))));
}

#[test]
fn test_collect_graph_match_predicates_nested() {
    let cond = Condition::And(
        Box::new(make_comparison("a", 1)),
        Box::new(Condition::Or(
            Box::new(make_graph_match()),
            Box::new(Condition::Not(Box::new(make_graph_match()))),
        )),
    );
    let mut predicates = Vec::new();
    Collection::collect_graph_match_predicates(&cond, &mut predicates);
    assert_eq!(predicates.len(), 2);
}

// ---------------------------------------------------------------------------
// C-2 regression: SparseVectorSearch must be stripped by extract_metadata_filter
// ---------------------------------------------------------------------------

#[test]
fn test_extract_metadata_filter_removes_sparse_vector_search() {
    // C-2: SparseVectorSearch was previously falling into `other => Some(other.clone())`.
    // It must return None, just like VectorSearch and Similarity do.
    let cond = make_sparse_vector_search();
    let result = Collection::extract_metadata_filter(&cond);
    assert!(
        result.is_none(),
        "extract_metadata_filter must return None for SparseVectorSearch"
    );
}

#[test]
fn test_extract_metadata_filter_and_with_sparse_vector_search() {
    // Compound condition: metadata AND sparse_near -> keep only metadata part.
    let cond = Condition::And(
        Box::new(make_comparison("category", 42)),
        Box::new(make_sparse_vector_search()),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(
        matches!(result, Some(Condition::Comparison(_))),
        "Only the metadata side of AND should survive"
    );
}

#[test]
fn test_extract_metadata_filter_or_with_sparse_vector_search_returns_none() {
    // OR with a sparse search: both sides must be metadata-only; otherwise None.
    let cond = Condition::Or(
        Box::new(make_comparison("score", 1)),
        Box::new(make_sparse_vector_search()),
    );
    let result = Collection::extract_metadata_filter(&cond);
    assert!(
        result.is_none(),
        "OR containing SparseVectorSearch must return None"
    );
}

// ============================================================================
// Regression (#2106 item 2): an unscorable pair (length mismatch, or empty)
// used to report 0.0 — a perfect score for a distance metric like Euclidean.
// The signature now says "no score" instead of inventing one.
// ============================================================================

#[cfg(feature = "persistence")]
mod metric_score_absence {
    use crate::collection::Collection;
    use crate::distance::DistanceMetric;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn collection(metric: DistanceMetric) -> (TempDir, Collection) {
        let dir = tempfile::tempdir().expect("test: tempdir");
        let col =
            Collection::create(PathBuf::from(dir.path()), 2, metric).expect("test: collection");
        (dir, col)
    }

    #[test]
    fn test_compute_metric_score_reports_no_score_on_length_mismatch() {
        for metric in [DistanceMetric::Euclidean, DistanceMetric::Cosine] {
            let (_dir, col) = collection(metric);
            assert_eq!(
                col.compute_metric_score(&[1.0, 2.0], &[1.0, 2.0, 3.0]),
                None,
                "{metric:?}: a length mismatch has no score, not 0.0"
            );
        }
    }

    #[test]
    fn test_compute_metric_score_reports_no_score_on_empty_vectors() {
        let (_dir, col) = collection(DistanceMetric::Euclidean);
        assert_eq!(col.compute_metric_score(&[], &[]), None);
    }

    #[test]
    fn test_compute_metric_score_returns_the_metric_value_when_scorable() {
        let (_dir, col) = collection(DistanceMetric::Euclidean);
        let score = col
            .compute_metric_score(&[3.0, 4.0], &[0.0, 0.0])
            .expect("test: equal lengths are scorable");
        assert!((score - 5.0).abs() < 1e-5, "euclidean 3-4-5, got {score}");
    }
}

// ---------------------------------------------------------------------------
// The recursive extractors walk one condition tree each. These tests pin what
// every arm of the walk does: which side wins, which nodes are looked through,
// and which are not (#2403 turned all five into associated functions).
// ---------------------------------------------------------------------------

type Params = std::collections::HashMap<String, serde_json::Value>;

const V: [f32; 2] = [1.0, 0.0];
const W: [f32; 2] = [0.0, 1.0];

fn params() -> Params {
    Params::from([
        ("v".to_string(), serde_json::json!(V)),
        ("w".to_string(), serde_json::json!(W)),
    ])
}

/// Asserts that an extractor refused the query for the parameter `$absent`.
fn assert_missing_parameter<T: std::fmt::Debug>(result: crate::error::Result<T>) {
    let error = result.expect_err("test: the parameter is missing");
    assert!(
        error
            .to_string()
            .contains("Missing query parameter: $absent"),
        "{error}"
    );
}

fn vector_search_on(param: &str) -> Condition {
    Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter(param.to_string()),
    })
}

fn fused_on(params: &[&str]) -> Condition {
    Condition::VectorFusedSearch(VectorFusedSearch {
        vectors: params
            .iter()
            .map(|name| VectorExpr::Parameter((*name).to_string()))
            .collect(),
        fusion: FusionConfig::default(),
    })
}

fn similarity_on(field: &str, param: &str, threshold: f64) -> Condition {
    Condition::Similarity(SimilarityCondition {
        field: field.to_string(),
        vector: VectorExpr::Parameter(param.to_string()),
        operator: CompareOp::Gt,
        threshold,
    })
}

fn and(left: Condition, right: Condition) -> Condition {
    Condition::And(Box::new(left), Box::new(right))
}

fn or(left: Condition, right: Condition) -> Condition {
    Condition::Or(Box::new(left), Box::new(right))
}

fn group(inner: Condition) -> Condition {
    Condition::Group(Box::new(inner))
}

fn not(inner: Condition) -> Condition {
    Condition::Not(Box::new(inner))
}

#[test]
fn test_extract_vector_search_looks_through_and_and_group_but_not_or() {
    let params = params();
    let extract = |condition: Condition| {
        Collection::extract_vector_search(&condition, &params).expect("test: resolves")
    };

    assert_eq!(extract(vector_search_on("v")), Some(V.to_vec()));
    assert_eq!(extract(group(vector_search_on("w"))), Some(W.to_vec()));
    assert_eq!(
        extract(and(make_comparison("a", 1), vector_search_on("w"))),
        Some(W.to_vec())
    );
    assert_eq!(
        extract(and(vector_search_on("v"), vector_search_on("w"))),
        Some(V.to_vec()),
        "the left side wins"
    );
    assert_eq!(
        extract(and(
            make_comparison("a", 1),
            group(and(make_comparison("b", 2), vector_search_on("w")))
        )),
        Some(W.to_vec())
    );
    assert_eq!(
        extract(or(vector_search_on("v"), vector_search_on("w"))),
        None,
        "an OR is not looked through"
    );
    assert_eq!(extract(make_comparison("a", 1)), None);
}

#[test]
fn test_extract_vector_search_reports_a_missing_parameter() {
    for condition in [
        and(make_comparison("a", 1), vector_search_on("absent")),
        and(vector_search_on("absent"), make_comparison("a", 1)),
    ] {
        assert_missing_parameter(Collection::extract_vector_search(&condition, &params()));
    }
}

#[test]
fn test_extract_fused_vectors_looks_through_and_and_group_but_not_or() {
    let params = params();
    let extract = |condition: Condition| {
        Collection::extract_fused_vectors(&condition, &params).expect("test: resolves")
    };

    let (vectors, fusion) = extract(fused_on(&["v", "w"])).expect("test: direct");
    assert_eq!(vectors, vec![V.to_vec(), W.to_vec()]);
    assert_eq!(fusion, FusionConfig::default());

    let vectors_of = |condition: Condition| extract(condition).map(|(vectors, _)| vectors);
    assert_eq!(vectors_of(group(fused_on(&["w"]))), Some(vec![W.to_vec()]));
    assert_eq!(
        vectors_of(and(make_comparison("a", 1), fused_on(&["w", "v"]))),
        Some(vec![W.to_vec(), V.to_vec()])
    );
    assert_eq!(
        vectors_of(and(fused_on(&["v"]), fused_on(&["w"]))),
        Some(vec![V.to_vec()]),
        "the left side wins"
    );
    assert_eq!(
        vectors_of(or(fused_on(&["v"]), fused_on(&["w"]))),
        None,
        "an OR is not looked through"
    );
    assert_eq!(vectors_of(make_comparison("a", 1)), None);
}

#[test]
fn test_extract_fused_vectors_reports_a_missing_parameter() {
    for condition in [
        group(fused_on(&["v", "absent"])),
        and(fused_on(&["absent"]), make_comparison("a", 1)),
        and(make_comparison("a", 1), fused_on(&["absent"])),
    ] {
        assert_missing_parameter(Collection::extract_fused_vectors(&condition, &params()));
    }
}

#[test]
fn test_extract_all_similarity_conditions_collects_every_side_in_order() {
    let params = params();
    let (a, b, c) = (
        similarity_on("a", "v", 0.1),
        similarity_on("b", "w", 0.2),
        similarity_on("c", "v", 0.3),
    );
    let fields_and_thresholds = |condition: Condition| {
        Collection::extract_all_similarity_conditions(&condition, &params)
            .expect("test: resolves")
            .into_iter()
            .map(|(field, _, _, threshold)| (field, threshold))
            .collect::<Vec<_>>()
    };
    let ab = vec![("a".to_string(), 0.1), ("b".to_string(), 0.2)];

    let direct = Collection::extract_all_similarity_conditions(&a, &params).expect("test: direct");
    assert_eq!(
        direct,
        vec![("a".to_string(), V.to_vec(), CompareOp::Gt, 0.1)]
    );

    assert_eq!(fields_and_thresholds(and(a.clone(), b.clone())), ab);
    assert_eq!(fields_and_thresholds(or(a.clone(), b.clone())), ab);
    assert_eq!(fields_and_thresholds(group(and(a.clone(), b.clone()))), ab);
    assert_eq!(
        fields_and_thresholds(not(a.clone())),
        vec![("a".to_string(), 0.1)]
    );
    assert_eq!(
        fields_and_thresholds(and(a.clone(), and(b, c))),
        vec![
            ("a".to_string(), 0.1),
            ("b".to_string(), 0.2),
            ("c".to_string(), 0.3)
        ]
    );
    assert_eq!(
        fields_and_thresholds(and(make_comparison("x", 1), a)),
        vec![("a".to_string(), 0.1)]
    );
    assert_eq!(fields_and_thresholds(make_comparison("x", 1)), vec![]);
}

#[test]
fn test_extract_all_similarity_conditions_reports_a_missing_parameter() {
    for condition in [
        and(
            similarity_on("a", "v", 0.1),
            similarity_on("b", "absent", 0.2),
        ),
        and(
            similarity_on("a", "absent", 0.1),
            similarity_on("b", "v", 0.2),
        ),
    ] {
        assert_missing_parameter(Collection::extract_all_similarity_conditions(
            &condition,
            &params(),
        ));
    }
}

#[test]
fn test_extract_not_similarity_condition_reads_inside_not_and_through_and() {
    let params = params();
    let (a, b) = (similarity_on("a", "v", 0.1), similarity_on("b", "w", 0.2));
    let extract =
        |condition: Condition| Collection::extract_not_similarity_condition(&condition, &params);

    assert_eq!(
        extract(not(a.clone())).expect("test: direct"),
        ("a".to_string(), V.to_vec(), CompareOp::Gt, 0.1)
    );
    assert_eq!(
        extract(and(make_comparison("x", 1), not(b.clone())))
            .expect("test: the right side")
            .0,
        "b"
    );
    assert_eq!(
        extract(and(not(a), not(b))).expect("test: both sides").0,
        "a",
        "the left side wins"
    );

    let refused = |condition: Condition, message: &str| {
        let error = extract(condition).expect_err("test: refused");
        assert!(error.to_string().contains(message), "{error}");
    };
    refused(
        not(make_comparison("x", 1)),
        "NOT clause does not contain a similarity condition",
    );
    refused(
        similarity_on("a", "v", 0.1),
        "Expected NOT similarity() condition",
    );
    refused(
        and(make_comparison("x", 1), make_comparison("y", 2)),
        "Expected NOT similarity() condition",
    );
}
