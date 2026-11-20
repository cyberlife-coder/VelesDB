//! Tests for the `VelesQL` query command's row conversion.

use super::search_result_to_hybrid;
use velesdb_core::{Point, SearchResult};

fn point() -> Point {
    Point::new(7, vec![1.0, 0.0], Some(serde_json::json!({"k": 1})))
}

/// A row reports the `vector_score` / `graph_score` components core tagged,
/// `None` for one it lacks, and its final score as `fused_score`.
#[test]
fn test_hybrid_row_reports_only_the_tagged_component_scores() {
    let near = SearchResult::with_component_scores(
        point(),
        0.9,
        [("vector_score", 0.9)].into_iter().collect(),
    );
    let row = search_result_to_hybrid(&near);
    assert_eq!(row.node_id, 7);
    assert_eq!(row.vector_score, Some(0.9));
    assert_eq!(row.graph_score, None);
    assert!((row.fused_score - 0.9).abs() < f32::EPSILON);
    assert_eq!(row.bindings, Some(serde_json::json!({"k": 1})));

    let hybrid = SearchResult::with_component_scores(
        point(),
        0.03,
        [("vector_score", 0.02), ("bm25_score", 0.01)]
            .into_iter()
            .collect(),
    );
    let row = search_result_to_hybrid(&hybrid);
    assert_eq!(row.vector_score, Some(0.02));
    assert!((row.fused_score - 0.03).abs() < f32::EPSILON);

    let text = SearchResult::with_component_scores(
        point(),
        0.5,
        [("bm25_score", 0.5)].into_iter().collect(),
    );
    assert_eq!(search_result_to_hybrid(&text).vector_score, None);

    // A row no vector scored (a metadata row) has no vector score.
    let metadata = SearchResult::new(point(), 1.0);
    let row = search_result_to_hybrid(&metadata);
    assert_eq!(row.vector_score, None);
    assert!((row.fused_score - 1.0).abs() < f32::EPSILON);
}
