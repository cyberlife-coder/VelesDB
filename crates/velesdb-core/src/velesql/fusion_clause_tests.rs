//! Tests for USING FUSION clause (EPIC-040 US-005).
//!
//! Covers:
//! - USING FUSION parsing with default RRF
//! - USING FUSION with explicit strategy (rrf, weighted, maximum)
//! - USING FUSION with parameters (k, weights)

use crate::velesql::Parser;

#[test]
fn test_using_fusion_default() {
    // USING FUSION without parameters - default RRF strategy
    let sql = "SELECT * FROM docs USING FUSION";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse USING FUSION: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let fusion = query
        .select
        .fusion_clause
        .as_ref()
        .expect("FUSION clause should be present");

    // Default strategy is RRF
    assert_eq!(fusion.strategy, crate::velesql::FusionStrategyType::Rrf);
}

#[test]
fn test_using_fusion_rrf_with_k() {
    let sql = "SELECT * FROM docs USING FUSION(strategy = 'rrf', k = 30)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse FUSION with k: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let fusion = query
        .select
        .fusion_clause
        .as_ref()
        .expect("FUSION clause should be present");

    assert_eq!(fusion.strategy, crate::velesql::FusionStrategyType::Rrf);
    assert_eq!(fusion.k, Some(30));
}

#[test]
fn test_using_fusion_weighted() {
    let sql = "SELECT * FROM docs USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse FUSION weighted: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let fusion = query
        .select
        .fusion_clause
        .as_ref()
        .expect("FUSION clause should be present");

    assert_eq!(
        fusion.strategy,
        crate::velesql::FusionStrategyType::Weighted
    );
    assert!((fusion.vector_weight.unwrap_or(0.0) - 0.7).abs() < 0.01);
    assert!((fusion.graph_weight.unwrap_or(0.0) - 0.3).abs() < 0.01);
}

#[test]
fn test_using_fusion_maximum() {
    let sql = "SELECT * FROM docs USING FUSION(strategy = 'maximum')";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse FUSION maximum: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let fusion = query
        .select
        .fusion_clause
        .as_ref()
        .expect("FUSION clause should be present");

    assert_eq!(fusion.strategy, crate::velesql::FusionStrategyType::Maximum);
}

#[test]
fn test_fusion_with_where_clause() {
    // USING FUSION combined with WHERE clause
    let sql = "SELECT * FROM docs WHERE category = 'tech' USING FUSION(strategy = 'rrf', k = 60)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse FUSION with WHERE: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert!(query.select.where_clause.is_some());
    assert!(query.select.fusion_clause.is_some());
}
