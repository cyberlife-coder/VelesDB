//! Tests for `match_planner` module - MATCH query execution planning.

use super::match_planner::*;
use crate::velesql::{
    CompareOp, Condition, GraphPattern, MatchClause, NodePattern, RelationshipPattern,
    ReturnClause, SimilarityCondition, VectorExpr,
};

fn default_stats() -> MatchGraphStats {
    MatchGraphStats {
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
                collection: None,
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
fn test_planner_routes_similarity_with_payload_order_by_to_graph_first() {
    // Backlog #1b: a start-similarity MATCH that ORDER BYs a non-similarity
    // payload field must use GraphFirst's exact enumeration — VectorFirst's
    // approximate HNSW prefix cannot answer a global payload ordering.
    let query = crate::velesql::Parser::parse(
        "MATCH (a:Person) WHERE similarity(a.embedding, $v) > 0.8 \
         RETURN a ORDER BY a.year DESC LIMIT 10",
    )
    .expect("test: parse similarity + payload ORDER BY");
    let match_clause = query.match_clause.expect("test: match clause present");
    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "similarity + ORDER BY payload must route to GraphFirst (exact), not VectorFirst"
    );
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

// =========================================================================
// W6-A: stats-to-strategy regression tests
// =========================================================================

#[test]
fn test_planner_with_empty_collection_stats_defaults_to_graph_first() {
    let match_clause = make_match_clause(false, Some(10));
    let stats = MatchGraphStats::default();
    let strategy = MatchQueryPlanner::plan(&match_clause, &stats);
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "empty collection should use GraphFirst"
    );
}

#[test]
fn test_planner_with_zero_labels_sets_full_selectivity() {
    let stats = MatchGraphStats {
        total_nodes: 100,
        total_edges: 50,
        avg_degree: 0.5,
        label_count: 0,
        label_selectivity: 1.0,
    };
    let match_clause = make_match_clause(false, Some(10));
    let strategy = MatchQueryPlanner::plan(&match_clause, &stats);
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "zero labels should use GraphFirst for non-similarity queries"
    );
}

#[test]
fn test_planner_graph_first_returns_start_labels() {
    let match_clause = make_match_clause(false, Some(10));
    let stats = default_stats();
    let strategy = MatchQueryPlanner::plan(&match_clause, &stats);
    if let MatchExecutionStrategy::GraphFirst { start_labels, .. } = strategy {
        assert_eq!(start_labels, vec!["Person".to_string()]);
    } else {
        panic!("expected GraphFirst strategy");
    }
}

#[test]
fn test_planner_vector_first_returns_threshold() {
    let match_clause = make_match_clause(true, Some(10));
    let stats = default_stats();
    let strategy = MatchQueryPlanner::plan(&match_clause, &stats);
    if let MatchExecutionStrategy::VectorFirst { threshold, .. } = strategy {
        assert!(
            (threshold - 0.8).abs() < f32::EPSILON,
            "threshold should match similarity condition"
        );
    } else {
        panic!("expected VectorFirst strategy");
    }
}

// =========================================================================
// Audit 2026-06 cluster F2: relationship-alias references force GraphFirst
// =========================================================================

/// Adds a `r` alias to the pattern relationship of a match clause.
fn with_rel_alias(mut match_clause: MatchClause) -> MatchClause {
    match_clause.patterns[0].relationships[0].alias = Some("r".to_string());
    match_clause
}

#[test]
fn test_planner_forces_graph_first_when_where_references_edge_alias() {
    let mut match_clause = with_rel_alias(make_match_clause(true, Some(10)));
    let sim = match_clause
        .where_clause
        .take()
        .expect("test: similarity condition");
    match_clause.where_clause = Some(Condition::And(
        Box::new(sim),
        Box::new(Condition::Comparison(crate::velesql::Comparison {
            column: "r.since".to_string(),
            operator: CompareOp::Eq,
            value: crate::velesql::Value::Integer(2020),
        })),
    ));

    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "WHERE r.since must force GraphFirst (VectorFirst cannot bind edge aliases), got {strategy:?}"
    );
}

#[test]
fn test_planner_forces_graph_first_when_return_references_edge_alias() {
    let mut match_clause = with_rel_alias(make_match_clause(true, Some(10)));
    match_clause.return_clause.items = vec![crate::velesql::ReturnItem {
        expression: "r.since".to_string(),
        alias: None,
    }];

    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "RETURN r.since must force GraphFirst (VectorFirst cannot bind edge aliases), got {strategy:?}"
    );
}

#[test]
fn test_planner_forces_graph_first_when_edge_alias_referenced_on_large_graphs() {
    // similarity on a NON-start alias over large, dense stats, plus an
    // edge-alias reference in RETURN: GraphFirst, which binds edge aliases.
    let mut match_clause = with_rel_alias(make_match_clause(true, Some(10)));
    if let Some(Condition::Similarity(sim)) = match_clause.where_clause.as_mut() {
        sim.field = "b.embedding".to_string();
        sim.threshold = 0.85;
    }
    match_clause.return_clause.items = vec![crate::velesql::ReturnItem {
        expression: "r.since".to_string(),
        alias: None,
    }];
    let stats = MatchGraphStats {
        total_nodes: 50_000,
        total_edges: 500_000,
        avg_degree: 10.0,
        label_count: 10,
        label_selectivity: 0.1,
    };

    let strategy = MatchQueryPlanner::plan(&match_clause, &stats);
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "edge-alias reference must force GraphFirst \
         (VectorFirst cannot bind edge aliases), got {strategy:?}"
    );
}

#[test]
fn test_planner_keeps_vector_first_without_edge_alias_reference() {
    // The relationship HAS an alias, but neither WHERE nor RETURN uses it.
    let match_clause = with_rel_alias(make_match_clause(true, Some(10)));

    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());
    assert!(
        matches!(strategy, MatchExecutionStrategy::VectorFirst { .. }),
        "an unused relationship alias must not disable VectorFirst, got {strategy:?}"
    );
}

#[test]
fn test_planner_node_alias_column_does_not_force_graph_first() {
    // `b.age` references a NODE alias — VectorFirst handles node aliases.
    let mut match_clause = with_rel_alias(make_match_clause(true, Some(10)));
    let sim = match_clause
        .where_clause
        .take()
        .expect("test: similarity condition");
    match_clause.where_clause = Some(Condition::And(
        Box::new(sim),
        Box::new(Condition::Comparison(crate::velesql::Comparison {
            column: "b.age".to_string(),
            operator: CompareOp::Gt,
            value: crate::velesql::Value::Integer(18),
        })),
    ));

    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());
    assert!(
        matches!(strategy, MatchExecutionStrategy::VectorFirst { .. }),
        "node-alias columns must not force GraphFirst, got {strategy:?}"
    );
}

#[test]
fn test_planner_is_null_on_edge_alias_forces_graph_first() {
    let mut match_clause = with_rel_alias(make_match_clause(true, Some(10)));
    let sim = match_clause
        .where_clause
        .take()
        .expect("test: similarity condition");
    match_clause.where_clause = Some(Condition::And(
        Box::new(sim),
        Box::new(Condition::IsNull(crate::velesql::IsNullCondition {
            column: "r.since".to_string(),
            is_null: true,
        })),
    ));

    let strategy = MatchQueryPlanner::plan(&match_clause, &default_stats());
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "IS NULL on an edge alias must force GraphFirst, got {strategy:?}"
    );
}

/// A similarity on a non-start alias over a large, dense graph plans
/// GraphFirst. It planned `Parallel`, which ran GraphFirst's exact
/// enumeration of the pattern — every row VectorFirst's approximate top-k
/// could return — and then VectorFirst on top: the union added nothing and
/// cost both legs.
#[test]
fn test_planner_never_runs_vector_first_beside_graph_first() {
    let mut match_clause = make_match_clause(true, Some(10));
    if let Some(Condition::Similarity(sim)) = match_clause.where_clause.as_mut() {
        sim.field = "b.embedding".to_string();
        sim.threshold = 0.85;
    }
    let stats = MatchGraphStats {
        total_nodes: 50_000,
        total_edges: 500_000,
        avg_degree: 10.0,
        label_count: 10,
        label_selectivity: 0.1,
    };

    let strategy = MatchQueryPlanner::plan(&match_clause, &stats);
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "got {strategy:?}"
    );
}
