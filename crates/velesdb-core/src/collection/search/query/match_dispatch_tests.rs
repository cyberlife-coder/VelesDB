use super::super::match_exec::MatchResult;
use super::merge_match_results;

fn mr(node_id: u64, score: Option<f32>) -> MatchResult {
    let mut r = MatchResult::new(node_id, 0, Vec::new());
    r.score = score;
    r
}

// --- Parallel strategy EXPLAIN counter path (Finding 10) ---
//
// The Parallel strategy cannot be reached end-to-end without a >10k-node,
// avg_degree>5, threshold>0.8 fixture (match_planner::should_use_parallel),
// which is impractical as a unit test. So we exercise `execute_match_parallel`
// DIRECTLY on a small fixture and assert the documented counter contract
// (ActualStats doc: "Parallel sums both legs"): the QueryContext counters
// after the parallel run equal the sum of the two legs run independently.
#[cfg(feature = "persistence")]
mod parallel_counters {
    use crate::collection::graph::GraphEdge;
    use crate::collection::types::Collection;
    use crate::distance::DistanceMetric;
    use crate::point::Point;
    use crate::velesql::match_planner::MatchExecutionStrategy;
    use crate::velesql::{MatchClause, Parser};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// 3-node `:Doc` chain (1->2->3 via LINK) with vectors for the VectorFirst
    /// leg; similarity is on the start node `a` so the leg surfaces candidates.
    fn setup_parallel_collection() -> (tempfile::TempDir, Collection) {
        let dir = tempfile::tempdir().expect("temp dir");
        let col = Collection::create(PathBuf::from(dir.path()), 2, DistanceMetric::Cosine)
            .expect("create collection");
        col.upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0],
                Some(serde_json::json!({"_labels": ["Doc"]})),
            ),
            Point::new(
                2,
                vec![0.7, 0.7],
                Some(serde_json::json!({"_labels": ["Doc"]})),
            ),
            Point::new(
                3,
                vec![0.0, 1.0],
                Some(serde_json::json!({"_labels": ["Doc"]})),
            ),
        ])
        .expect("upsert");
        col.add_edge(GraphEdge::new(10, 1, 2, "LINK").expect("edge"))
            .expect("add edge");
        col.add_edge(GraphEdge::new(11, 2, 3, "LINK").expect("edge"))
            .expect("add edge");
        (dir, col)
    }

    fn parallel_match_clause() -> MatchClause {
        Parser::parse(
            "MATCH (a:Doc)-[:LINK]->(b:Doc) WHERE similarity(a, $v) > 0.0 RETURN a, b LIMIT 10",
        )
        .expect("parse parallel MATCH")
        .match_clause
        .expect("MATCH clause present")
    }

    fn vector_first_hint() -> MatchExecutionStrategy {
        MatchExecutionStrategy::VectorFirst {
            similarity_alias: "a".to_string(),
            top_k: 10,
            threshold: 0.0,
        }
    }

    #[test]
    fn parallel_counters_sum_both_legs() {
        let (_dir, col) = setup_parallel_collection();
        let mc = parallel_match_clause();
        let hint = vector_first_hint();
        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0]));

        // Leg 1 in isolation: GraphFirst (execute_match_with_context).
        let graph_ctx = col.runtime.guard_rails.create_context();
        col.execute_match_with_context(&mc, &params, Some(&graph_ctx))
            .expect("graph leg");
        let graph_nodes = graph_ctx.traversal_nodes_visited();
        let graph_edges = graph_ctx.traversal_edges_traversed();

        // Leg 2 in isolation: VectorFirst (execute_match_vector_first).
        let vec_ctx = col.runtime.guard_rails.create_context();
        col.execute_match_vector_first(&mc, &params, &vec_ctx, "a", 10, 0.0)
            .expect("vector leg");
        let vec_nodes = vec_ctx.traversal_nodes_visited();
        let vec_edges = vec_ctx.traversal_edges_traversed();

        // Both legs must actually traverse, else the sum assertion is vacuous.
        assert!(graph_edges > 0, "graph leg must follow LINK edges");
        assert!(vec_nodes > 0, "vector leg must evaluate candidates");

        // Parallel run accumulates BOTH legs into one shared context.
        let par_ctx = col.runtime.guard_rails.create_context();
        col.execute_match_parallel(&mc, &params, &par_ctx, &hint)
            .expect("parallel run");

        assert_eq!(
            par_ctx.traversal_nodes_visited(),
            graph_nodes + vec_nodes,
            "Parallel nodes_visited must equal the sum of both legs"
        );
        assert_eq!(
            par_ctx.traversal_edges_traversed(),
            graph_edges + vec_edges,
            "Parallel edges_traversed must equal the sum of both legs"
        );
    }
}

// --- higher_is_better = true (cosine / dot-product) ---

#[test]
fn test_merge_empty_inputs() {
    let merged = merge_match_results(Vec::new(), Vec::new(), true);
    assert!(merged.is_empty());
}

#[test]
fn test_merge_graph_only() {
    let graph = vec![mr(1, None), mr(2, Some(0.5))];
    let merged = merge_match_results(graph, Vec::new(), true);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].node_id, 2);
}

#[test]
fn test_merge_vector_only() {
    let vector = vec![mr(3, Some(0.9)), mr(4, Some(0.7))];
    let merged = merge_match_results(Vec::new(), vector, true);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].node_id, 3);
    assert_eq!(merged[1].node_id, 4);
}

#[test]
fn test_merge_union_distinct_nodes() {
    let graph = vec![mr(1, None), mr(2, None)];
    let vector = vec![mr(3, Some(0.8)), mr(4, Some(0.6))];
    let merged = merge_match_results(graph, vector, true);
    assert_eq!(merged.len(), 4);
}

#[test]
fn test_merge_duplicate_keeps_higher_score() {
    let graph = vec![mr(1, Some(0.3))];
    let vector = vec![mr(1, Some(0.9))];
    let merged = merge_match_results(graph, vector, true);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].node_id, 1);
    assert!((merged[0].score.expect("test: should have score") - 0.9).abs() < f32::EPSILON);
}

#[test]
fn test_merge_duplicate_graph_wins_when_higher() {
    let graph = vec![mr(1, Some(0.95))];
    let vector = vec![mr(1, Some(0.5))];
    let merged = merge_match_results(graph, vector, true);
    assert_eq!(merged.len(), 1);
    assert!((merged[0].score.expect("test: should have score") - 0.95).abs() < f32::EPSILON);
}

#[test]
fn test_merge_sorted_descending() {
    let graph = vec![mr(1, Some(0.3)), mr(2, Some(0.1))];
    let vector = vec![mr(3, Some(0.9)), mr(4, Some(0.5))];
    let merged = merge_match_results(graph, vector, true);
    let scores: Vec<f32> = merged
        .iter()
        .map(|r| r.score.unwrap_or(f32::NEG_INFINITY))
        .collect();
    for w in scores.windows(2) {
        assert!(w[0] >= w[1], "scores should be descending: {scores:?}");
    }
}

#[test]
fn test_merge_none_scores_sorted_last() {
    let graph = vec![mr(1, None), mr(2, None)];
    let vector = vec![mr(3, Some(0.5))];
    let merged = merge_match_results(graph, vector, true);
    assert_eq!(merged.len(), 3);
    assert_eq!(merged[0].node_id, 3);
}

// --- higher_is_better = false (euclidean / hamming) ---

#[test]
fn test_merge_euclidean_duplicate_keeps_lower_score() {
    let graph = vec![mr(1, Some(0.9))];
    let vector = vec![mr(1, Some(0.2))];
    let merged = merge_match_results(graph, vector, false);
    assert_eq!(merged.len(), 1);
    assert!(
        (merged[0].score.expect("test: should have score") - 0.2).abs() < f32::EPSILON,
        "Euclidean: lower distance should win"
    );
}

#[test]
fn test_merge_euclidean_graph_wins_when_lower() {
    let graph = vec![mr(1, Some(0.1))];
    let vector = vec![mr(1, Some(0.8))];
    let merged = merge_match_results(graph, vector, false);
    assert_eq!(merged.len(), 1);
    assert!(
        (merged[0].score.expect("test: should have score") - 0.1).abs() < f32::EPSILON,
        "Euclidean: graph result with lower distance should win"
    );
}

#[test]
fn test_merge_euclidean_sorted_ascending() {
    let graph = vec![mr(1, Some(0.9)), mr(2, Some(0.3))];
    let vector = vec![mr(3, Some(0.1)), mr(4, Some(0.5))];
    let merged = merge_match_results(graph, vector, false);
    let scores: Vec<f32> = merged.iter().map(|r| r.score.unwrap_or(f32::MAX)).collect();
    for w in scores.windows(2) {
        assert!(
            w[0] <= w[1],
            "Euclidean scores should be ascending (best first): {scores:?}"
        );
    }
}

#[test]
fn test_merge_euclidean_none_scores_sorted_last() {
    let graph = vec![mr(1, None), mr(2, None)];
    let vector = vec![mr(3, Some(0.5))];
    let merged = merge_match_results(graph, vector, false);
    assert_eq!(merged.len(), 3);
    assert_eq!(
        merged[0].node_id, 3,
        "Euclidean: scored result should sort before None"
    );
}

#[test]
fn test_merge_empty_inputs_euclidean() {
    let merged = merge_match_results(Vec::new(), Vec::new(), false);
    assert!(merged.is_empty());
}

// --- collision data merge (audit 2026-06 cluster F2, finding 5) ---

/// Builds a GraphFirst-style result: unscored, with edge projection data.
fn graph_mr_with_edge_data(node_id: u64) -> MatchResult {
    let mut r = MatchResult::new(node_id, 1, vec![100]);
    r.bindings.insert("b".to_string(), node_id);
    r.edge_bindings.insert("r".to_string(), 100);
    r.projected
        .insert("r.since".to_string(), serde_json::json!(2020));
    r
}

/// GIVEN a GraphFirst result carrying `r.since` projection + edge binding
///   and a scored VectorFirst candidate for the same node without them
/// WHEN the candidate wins the score comparison
/// THEN the winning score is kept BUT the GraphFirst-only projection,
///      edge bindings, and node bindings survive the merge.
#[test]
fn test_merge_collision_preserves_graph_edge_data() {
    let graph = vec![graph_mr_with_edge_data(1)];
    let vector = vec![mr(1, Some(0.9))];

    let merged = merge_match_results(graph, vector, true);

    assert_eq!(merged.len(), 1);
    assert!(
        (merged[0].score.expect("test: should have score") - 0.9).abs() < f32::EPSILON,
        "the better (vector) score must win"
    );
    assert_eq!(
        merged[0].projected.get("r.since"),
        Some(&serde_json::json!(2020)),
        "GraphFirst projection must survive the collision merge"
    );
    assert_eq!(
        merged[0].edge_bindings.get("r"),
        Some(&100),
        "GraphFirst edge binding must survive the collision merge"
    );
    assert_eq!(
        merged[0].bindings.get("b"),
        Some(&1),
        "GraphFirst node binding must survive the collision merge"
    );
}

/// GIVEN a scored GraphFirst result that beats the vector candidate
/// WHEN the candidate loses the score comparison
/// THEN candidate-only data (e.g. its projection keys) still survives.
#[test]
fn test_merge_collision_preserves_loser_only_keys() {
    let mut graph = graph_mr_with_edge_data(1);
    graph.score = Some(0.95);
    let mut vector = mr(1, Some(0.5));
    vector
        .projected
        .insert("similarity()".to_string(), serde_json::json!(0.5));

    let merged = merge_match_results(vec![graph], vec![vector], true);

    assert_eq!(merged.len(), 1);
    assert!(
        (merged[0].score.expect("test: should have score") - 0.95).abs() < f32::EPSILON,
        "the better (graph) score must win"
    );
    assert!(
        merged[0].projected.contains_key("similarity()"),
        "loser-only projection keys must survive the collision merge"
    );
    assert_eq!(
        merged[0].projected.get("r.since"),
        Some(&serde_json::json!(2020)),
        "winner projection must be untouched"
    );
}

/// GIVEN two parallel-edge graph rows for the same node (distinct edge
///   bindings) and one scored vector candidate for that node
/// WHEN the Parallel strategy merges the result sets
/// THEN BOTH rows survive AND both carry the node-level score (review
///      2026-06-11: enrichment must reach every row of the node group,
///      not the first one found).
#[test]
fn test_merge_enriches_all_parallel_edge_rows() {
    let mut g1 = graph_mr_with_edge_data(1);
    g1.edge_bindings.insert("r".to_string(), 100);
    let mut g2 = graph_mr_with_edge_data(1);
    g2.edge_bindings.insert("r".to_string(), 101);
    let vector = vec![mr(1, Some(0.9))];

    let merged = merge_match_results(vec![g1, g2], vector, true);

    assert_eq!(merged.len(), 2, "both parallel-edge rows must survive");
    for row in &merged {
        assert!(
            (row.score.expect("test: enriched score") - 0.9).abs() < f32::EPSILON,
            "every row of the node group must carry the node-level score"
        );
    }
    let mut edge_ids: Vec<u64> = merged
        .iter()
        .filter_map(|r| r.edge_bindings.get("r").copied())
        .collect();
    edge_ids.sort_unstable();
    assert_eq!(edge_ids, vec![100, 101], "edge identities must be distinct");
}
