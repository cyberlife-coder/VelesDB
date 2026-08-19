use super::*;
use crate::sparse_index::SparseVector;
use crate::velesql::{
    CompareOp, Comparison, FusionConfig, SimilarityCondition, SparseVectorExpr, SparseVectorSearch,
    Value, VectorExpr, VectorFusedSearch,
};

fn make_similarity_condition() -> Condition {
    Condition::Similarity(SimilarityCondition {
        field: "vector".to_string(),
        vector: VectorExpr::Literal(vec![0.1, 0.2, 0.3]),
        operator: CompareOp::Gt,
        threshold: 0.8,
    })
}

fn make_compare_condition() -> Condition {
    Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    })
}

#[test]
fn test_validate_single_similarity_and_metadata_ok() {
    // similarity() AND category = 'tech' - should be OK
    let cond = Condition::And(
        Box::new(make_similarity_condition()),
        Box::new(make_compare_condition()),
    );
    assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
}

#[test]
fn test_validate_similarity_or_metadata_ok() {
    // EPIC-044 US-002: similarity() OR category = 'tech' - NOW OK (union mode)
    let cond = Condition::Or(
        Box::new(make_similarity_condition()),
        Box::new(make_compare_condition()),
    );
    assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
}

#[test]
fn test_validate_multiple_similarity_with_and_ok() {
    // EPIC-044 US-001: similarity() AND similarity() - should be OK (cascade filtering)
    let cond = Condition::And(
        Box::new(make_similarity_condition()),
        Box::new(make_similarity_condition()),
    );
    assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
}

#[test]
fn test_validate_multiple_similarity_with_or_fails() {
    // similarity() OR similarity() - should FAIL (would require union)
    let cond = Condition::Or(
        Box::new(make_similarity_condition()),
        Box::new(make_similarity_condition()),
    );
    let result = Collection::validate_similarity_query_structure(&cond);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("OR"));
}

#[test]
fn test_validate_three_similarity_with_and_ok() {
    // EPIC-044 US-001: Three similarity() with AND - should be OK
    let cond = Condition::And(
        Box::new(make_similarity_condition()),
        Box::new(Condition::And(
            Box::new(make_similarity_condition()),
            Box::new(make_similarity_condition()),
        )),
    );
    assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
}

#[test]
fn test_validate_metadata_only_ok() {
    // category = 'tech' AND status = 'active' - should be OK
    let cond = Condition::And(
        Box::new(make_compare_condition()),
        Box::new(make_compare_condition()),
    );
    assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
}

#[test]
fn test_validate_metadata_or_ok() {
    // category = 'tech' OR status = 'active' - should be OK (no similarity)
    let cond = Condition::Or(
        Box::new(make_compare_condition()),
        Box::new(make_compare_condition()),
    );
    assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
}

fn make_fused_condition() -> Condition {
    let vectors = vec![
        VectorExpr::Literal(vec![0.1, 0.2]),
        VectorExpr::Literal(vec![0.3]),
    ];
    let fusion = FusionConfig::rrf();
    Condition::VectorFusedSearch(VectorFusedSearch { vectors, fusion })
}

fn make_sparse_condition() -> Condition {
    Condition::SparseVectorSearch(SparseVectorSearch {
        vector: SparseVectorExpr::Literal(SparseVector::new(vec![(1, 0.5), (3, 0.2)])),
        index_name: None,
    })
}

#[test]
fn test_validate_fused_and_sparse_fails() {
    // NEAR_FUSED AND SPARSE_NEAR must reject: SPARSE_NEAR would otherwise
    // bypass the isolation guard and silently drop the fused vectors.
    let cond = Condition::And(
        Box::new(make_fused_condition()),
        Box::new(make_sparse_condition()),
    );
    let result = Collection::validate_similarity_query_structure(&cond);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("NEAR_FUSED"));
}

#[test]
fn test_count_similarity_conditions() {
    assert_eq!(
        Collection::count_similarity_conditions(&make_similarity_condition()),
        1
    );
    assert_eq!(
        Collection::count_similarity_conditions(&make_compare_condition()),
        0
    );

    let double = Condition::And(
        Box::new(make_similarity_condition()),
        Box::new(make_similarity_condition()),
    );
    assert_eq!(Collection::count_similarity_conditions(&double), 2);
}
