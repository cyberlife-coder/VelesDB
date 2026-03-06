//! Tests for sparse vector search parsing (SPARSE_NEAR).

use crate::velesql::ast::condition::{SparseVectorExpr, SparseVectorSearch};
use crate::velesql::{Condition, Parser};

#[test]
fn test_parse_sparse_near_param() {
    let query = Parser::parse("SELECT * FROM docs WHERE vector SPARSE_NEAR $sv LIMIT 10").unwrap();
    let where_clause = query.select.where_clause.as_ref().unwrap();
    match where_clause {
        Condition::SparseVectorSearch(svs) => {
            assert!(matches!(&svs.vector, SparseVectorExpr::Parameter(name) if name == "sv"));
            assert!(svs.index_name.is_none());
        }
        other => panic!("Expected SparseVectorSearch, got {other:?}"),
    }
}

#[test]
fn test_parse_sparse_near_literal() {
    let query =
        Parser::parse("SELECT * FROM docs WHERE vector SPARSE_NEAR {12: 0.8, 45: 0.3} LIMIT 10")
            .unwrap();
    let where_clause = query.select.where_clause.as_ref().unwrap();
    match where_clause {
        Condition::SparseVectorSearch(svs) => {
            match &svs.vector {
                SparseVectorExpr::Literal(sv) => {
                    assert_eq!(sv.indices, vec![12, 45]);
                    assert!((sv.values[0] - 0.8).abs() < 1e-5);
                    assert!((sv.values[1] - 0.3).abs() < 1e-5);
                }
                other => panic!("Expected Literal, got {other:?}"),
            }
            assert!(svs.index_name.is_none());
        }
        other => panic!("Expected SparseVectorSearch, got {other:?}"),
    }
}

#[test]
fn test_parse_sparse_near_using() {
    let query = Parser::parse(
        "SELECT * FROM docs WHERE vector SPARSE_NEAR $sv USING 'title-sparse' LIMIT 10",
    )
    .unwrap();
    let where_clause = query.select.where_clause.as_ref().unwrap();
    match where_clause {
        Condition::SparseVectorSearch(svs) => {
            assert!(matches!(&svs.vector, SparseVectorExpr::Parameter(name) if name == "sv"));
            assert_eq!(svs.index_name.as_deref(), Some("title-sparse"));
        }
        other => panic!("Expected SparseVectorSearch, got {other:?}"),
    }
}

#[test]
fn test_parse_hybrid_near_and_sparse() {
    let query = Parser::parse(
        "SELECT * FROM docs WHERE vector NEAR $dv AND vector SPARSE_NEAR $sv LIMIT 10",
    )
    .unwrap();
    let where_clause = query.select.where_clause.as_ref().unwrap();
    match where_clause {
        Condition::And(left, right) => {
            assert!(
                matches!(left.as_ref(), Condition::VectorSearch(_)),
                "Left should be VectorSearch"
            );
            assert!(
                matches!(right.as_ref(), Condition::SparseVectorSearch(_)),
                "Right should be SparseVectorSearch"
            );
        }
        other => panic!("Expected And(VectorSearch, SparseVectorSearch), got {other:?}"),
    }
}

#[test]
fn test_parse_sparse_with_filter() {
    let query = Parser::parse(
        "SELECT * FROM docs WHERE vector SPARSE_NEAR $sv AND category = 'tech' LIMIT 10",
    )
    .unwrap();
    let where_clause = query.select.where_clause.as_ref().unwrap();
    match where_clause {
        Condition::And(left, right) => {
            assert!(
                matches!(left.as_ref(), Condition::SparseVectorSearch(_)),
                "Left should be SparseVectorSearch"
            );
            assert!(
                matches!(right.as_ref(), Condition::Comparison(_)),
                "Right should be Comparison"
            );
        }
        other => panic!("Expected And(SparseVectorSearch, Comparison), got {other:?}"),
    }
}

#[test]
fn test_parse_rsf_fusion() {
    let query = Parser::parse(
        "SELECT * FROM docs WHERE vector NEAR $dv AND vector SPARSE_NEAR $sv LIMIT 10 USING FUSION(strategy = 'rsf', dense_w = 0.7, sparse_w = 0.3)",
    )
    .unwrap();
    // Verify the fusion clause
    let fusion = query.select.fusion_clause.as_ref().unwrap();
    assert_eq!(fusion.strategy, crate::velesql::FusionStrategyType::Rsf);
    assert!((fusion.dense_weight.unwrap() - 0.7).abs() < 0.01);
    assert!((fusion.sparse_weight.unwrap() - 0.3).abs() < 0.01);
}

#[test]
fn test_parse_sparse_near_case_insensitive() {
    // SPARSE_NEAR is case-insensitive
    let query = Parser::parse("SELECT * FROM docs WHERE vector sparse_near $sv LIMIT 5").unwrap();
    let where_clause = query.select.where_clause.as_ref().unwrap();
    assert!(matches!(where_clause, Condition::SparseVectorSearch(_)));
}

#[test]
fn test_parse_sparse_literal_sorted() {
    // Sparse literal with out-of-order indices should be sorted
    let query =
        Parser::parse("SELECT * FROM docs WHERE vector SPARSE_NEAR {45: 0.3, 12: 0.8} LIMIT 5")
            .unwrap();
    let where_clause = query.select.where_clause.as_ref().unwrap();
    match where_clause {
        Condition::SparseVectorSearch(SparseVectorSearch {
            vector: SparseVectorExpr::Literal(sv),
            ..
        }) => {
            // SparseVector::new sorts by index
            assert_eq!(sv.indices, vec![12, 45]);
        }
        other => panic!("Expected SparseVectorSearch with Literal, got {other:?}"),
    }
}
