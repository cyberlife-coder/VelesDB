//! Tests for `match_planner` module - MATCH query execution planning.

use super::match_planner::*;
use crate::velesql::{
    CompareOp, Condition, GraphPattern, MatchClause, NodePattern, RelationshipPattern,
    ReturnClause, SimilarityCondition, VectorExpr,
};

fn default_stats() -> CollectionStats {
    CollectionStats {
        total_nodes: 1000,
        total_edges: 5000,
        avg_degree: 5.0,
        label_count: 10,
        label_selectivity: 0.1,
    }
}

fn make_match_clause(has_similarity: bool, limit: Option<u64>) -> MatchClause {
    MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![NodePattern {
                alias: Some("a".to_string()),
                labels: vec!["Person".to_string()],
                properties: std::collections::HashMap::new(),
            }],
            relationships: vec![RelationshipPattern {
                alias: None,
                types: vec!["KNOWS".to_string()],
                direction: crate::velesql::Direction::Outgoing,
                range: None,
                properties: std::collections::HashMap::new(),
            }],
        }],
        where_clause: if has_similarity {
            Some(Condition::Similarity(SimilarityCondition {
                field: "a.embedding".to_string(),
                vector: VectorExpr::Parameter("$v".to_string()),
                operator: CompareOp::Gt,
                threshold: 0.8,
            }))
        } else {
            None
        },
        return_clause: ReturnClause {
            items: vec![],
            order_by: None,
            limit,
        },
    }
}

#[test]
fn test_planner_chooses_graph_first_for_pure_graph() {
    let match_clause = make_match_clause(false, Some(10));
    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());

    assert!(matches!(
        strategy,
        MatchExecutionStrategy::GraphFirst { .. }
    ));
}

#[test]
fn test_planner_chooses_vector_first_for_start_similarity() {
    let match_clause = make_match_clause(true, Some(10));
    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());

    assert!(matches!(
        strategy,
        MatchExecutionStrategy::VectorFirst { .. }
    ));
}

#[test]
fn test_estimate_selectivity() {
    assert!((MatchQueryPlanner::estimate_selectivity(0.9) - 0.1).abs() < 0.01);
    assert!((MatchQueryPlanner::estimate_selectivity(0.5) - 0.5).abs() < 0.01);
}

#[test]
fn test_explain_graph_first() {
    let strategy = MatchExecutionStrategy::GraphFirst {
        start_labels: vec!["Person".to_string()],
        max_depth: 3,
    };
    let explanation = MatchQueryPlanner::explain(&strategy);
    assert!(explanation.contains("GraphFirst"));
    assert!(explanation.contains("Person"));
}

#[test]
fn test_explain_vector_first() {
    let strategy = MatchExecutionStrategy::VectorFirst {
        similarity_alias: "doc".to_string(),
        top_k: 100,
        threshold: 0.85,
    };
    let explanation = MatchQueryPlanner::explain(&strategy);
    assert!(explanation.contains("VectorFirst"));
    assert!(explanation.contains("doc"));
}

#[test]
fn test_count_hops() {
    let match_clause = make_match_clause(false, None);
    let hops = MatchQueryPlanner::count_hops(&match_clause);
    assert_eq!(hops, 1);
}
