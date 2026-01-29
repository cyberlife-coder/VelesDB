//! Tests for `match_exec` module - MATCH clause execution.

use super::match_exec::*;
use std::collections::HashMap;

#[test]
fn test_match_result_creation() {
    let result = MatchResult::new(42, 2, vec![1, 2]);
    assert_eq!(result.node_id, 42);
    assert_eq!(result.depth, 2);
    assert_eq!(result.path, vec![1, 2]);
}

#[test]
fn test_match_result_with_binding() {
    let result = MatchResult::new(42, 0, vec![]).with_binding("n".to_string(), 42);
    assert_eq!(result.bindings.get("n"), Some(&42));
}

// ============================================================================
// Property Projection Tests (EPIC-058 US-007)
// ============================================================================

#[test]
fn test_match_result_with_projected_properties() {
    let mut projected = HashMap::new();
    projected.insert("author.name".to_string(), serde_json::json!("John Doe"));
    projected.insert("doc.title".to_string(), serde_json::json!("Research Paper"));

    let result = MatchResult::new(42, 1, vec![1])
        .with_binding("doc".to_string(), 42)
        .with_projected(projected.clone());

    assert_eq!(result.projected.len(), 2);
    assert_eq!(
        result.projected.get("author.name"),
        Some(&serde_json::json!("John Doe"))
    );
    assert_eq!(
        result.projected.get("doc.title"),
        Some(&serde_json::json!("Research Paper"))
    );
}

#[test]
fn test_parse_property_path_valid() {
    // "author.name" -> ("author", "name")
    let (alias, property) = parse_property_path("author.name").unwrap();
    assert_eq!(alias, "author");
    assert_eq!(property, "name");
}

#[test]
fn test_parse_property_path_nested() {
    // "doc.metadata.category" -> ("doc", "metadata.category")
    let (alias, property) = parse_property_path("doc.metadata.category").unwrap();
    assert_eq!(alias, "doc");
    assert_eq!(property, "metadata.category");
}

#[test]
fn test_parse_property_path_invalid_no_dot() {
    // "nodot" -> None (invalid)
    let result = parse_property_path("nodot");
    assert!(result.is_none());
}

#[test]
fn test_parse_property_path_star() {
    // "*" -> None (wildcard, not a property path)
    let result = parse_property_path("*");
    assert!(result.is_none());
}

#[test]
fn test_parse_property_path_function() {
    // "similarity()" -> None (function, not a property path)
    let result = parse_property_path("similarity()");
    assert!(result.is_none());
}

// ============================================================================
// EPIC-052 US-007: OR/NOT with Similarity Conditions Tests
// ============================================================================

#[test]
fn test_similarity_condition_evaluation_basic() {
    use crate::velesql::{CompareOp, SimilarityCondition, VectorExpr};

    // Test that similarity condition structure is correct
    let cond = SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("query_vec".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    };

    assert_eq!(cond.field, "embedding");
    assert!((cond.threshold - 0.8).abs() < f64::EPSILON);
    assert_eq!(cond.operator, CompareOp::Gt);
}

#[test]
fn test_or_condition_with_comparisons() {
    use crate::velesql::{CompareOp, Comparison, Condition, Value};

    // OR between two comparisons
    let left = Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });

    let right = Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("science".to_string()),
    });

    let or_cond = Condition::Or(Box::new(left), Box::new(right));

    // Verify structure
    match or_cond {
        Condition::Or(l, r) => {
            assert!(matches!(*l, Condition::Comparison(_)));
            assert!(matches!(*r, Condition::Comparison(_)));
        }
        _ => panic!("Expected Or condition"),
    }
}

#[test]
fn test_not_condition_structure() {
    use crate::velesql::{CompareOp, Comparison, Condition, Value};

    let inner = Condition::Comparison(Comparison {
        column: "status".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("deleted".to_string()),
    });

    let not_cond = Condition::Not(Box::new(inner));

    match not_cond {
        Condition::Not(inner) => {
            assert!(matches!(*inner, Condition::Comparison(_)));
        }
        _ => panic!("Expected Not condition"),
    }
}

#[test]
fn test_or_with_similarity_structure() {
    use crate::velesql::{CompareOp, Condition, SimilarityCondition, VectorExpr};

    // similarity(embedding, $vec1) > 0.8 OR similarity(embedding, $vec2) > 0.7
    let sim1 = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("vec1".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });

    let sim2 = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("vec2".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.7,
    });

    let or_cond = Condition::Or(Box::new(sim1), Box::new(sim2));

    match or_cond {
        Condition::Or(l, r) => {
            assert!(matches!(*l, Condition::Similarity(_)));
            assert!(matches!(*r, Condition::Similarity(_)));
        }
        _ => panic!("Expected Or condition with similarities"),
    }
}

#[test]
fn test_not_similarity_structure() {
    use crate::velesql::{CompareOp, Condition, SimilarityCondition, VectorExpr};

    // NOT similarity(embedding, $crypto_vec) > 0.7
    let sim = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("crypto_vec".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.7,
    });

    let not_cond = Condition::Not(Box::new(sim));

    match not_cond {
        Condition::Not(inner) => {
            assert!(matches!(*inner, Condition::Similarity(_)));
        }
        _ => panic!("Expected Not condition with similarity"),
    }
}

#[test]
fn test_combined_and_or_not_structure() {
    use crate::velesql::{CompareOp, Condition, SimilarityCondition, VectorExpr};

    // similarity(embedding, $tech) > 0.8 AND NOT similarity(embedding, $crypto) > 0.7
    let sim_tech = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("tech".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });

    let sim_crypto = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("crypto".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.7,
    });

    let not_crypto = Condition::Not(Box::new(sim_crypto));
    let combined = Condition::And(Box::new(sim_tech), Box::new(not_crypto));

    match combined {
        Condition::And(l, r) => {
            assert!(matches!(*l, Condition::Similarity(_)));
            assert!(matches!(*r, Condition::Not(_)));
        }
        _ => panic!("Expected And condition"),
    }
}

#[test]
fn test_double_negation_structure() {
    use crate::velesql::{CompareOp, Condition, SimilarityCondition, VectorExpr};

    // NOT NOT similarity(embedding, $vec) > 0.8
    let sim = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("vec".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });

    let not_once = Condition::Not(Box::new(sim));
    let not_twice = Condition::Not(Box::new(not_once));

    match not_twice {
        Condition::Not(inner) => {
            assert!(matches!(*inner, Condition::Not(_)));
        }
        _ => panic!("Expected double Not"),
    }
}
