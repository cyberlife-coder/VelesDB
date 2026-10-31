use super::*;
use crate::velesql::Parser;

/// Creates a `SearchResult` with the given id, score, and payload.
fn make_result(id: u64, score: f32, payload: serde_json::Value) -> SearchResult {
    SearchResult {
        point: Point {
            id,
            vector: vec![0.0; 4],
            payload: Some(payload),
            sparse_vectors: None,
        },
        score,
        component_scores: None,
    }
}

fn select(sql: &str) -> SelectStatement {
    Parser::parse(sql).expect("test: parse").select
}

fn group(sql: &str, results: &[SearchResult]) -> Vec<SearchResult> {
    group_search_results(&select(sql), results, None, true).expect("test: group")
}

fn field<'a>(result: &'a SearchResult, name: &str) -> Option<&'a serde_json::Value> {
    result.point.payload.as_ref().and_then(|p| p.get(name))
}

#[test]
fn test_a_single_row_is_its_own_group() {
    let results = vec![make_result(
        1,
        0.9,
        serde_json::json!({"parent": "A", "text": "hello"}),
    )];
    let grouped = group(
        "SELECT parent, MAX(score) AS relevance FROM t WHERE vector NEAR $v GROUP BY parent",
        &results,
    );
    assert_eq!(grouped.len(), 1);
    assert!((grouped[0].score - 0.9).abs() < f32::EPSILON);
}

#[test]
fn test_a_group_is_its_best_row_with_first_read_from_it() {
    let results = vec![
        make_result(1, 0.5, serde_json::json!({"parent": "A", "text": "low"})),
        make_result(2, 0.9, serde_json::json!({"parent": "A", "text": "high"})),
        make_result(3, 0.7, serde_json::json!({"parent": "A", "text": "mid"})),
        make_result(4, 0.8, serde_json::json!({"parent": "B", "text": "only"})),
    ];
    let grouped = group(
        "SELECT parent, MAX(score) AS relevance, FIRST(text) AS excerpt, COUNT(*) AS n \
         FROM t WHERE vector NEAR $v GROUP BY parent",
        &results,
    );
    assert_eq!(grouped.len(), 2);
    // Best group first: A scores 0.9, B 0.8.
    assert_eq!(grouped[0].point.id, 2, "a group carries its best row's id");
    assert!((grouped[0].score - 0.9).abs() < f32::EPSILON);
    assert_eq!(
        field(&grouped[0], "excerpt"),
        Some(&serde_json::json!("high"))
    );
    assert_eq!(field(&grouped[0], "n"), Some(&serde_json::json!(3)));
}

#[test]
fn test_a_row_without_the_group_column_joins_the_null_group() {
    let results = vec![
        make_result(1, 0.9, serde_json::json!({"parent": "A"})),
        make_result(2, 0.8, serde_json::json!({"other": "no parent"})),
        make_result(3, 0.7, serde_json::json!({"parent": "A"})),
    ];
    let grouped = group(
        "SELECT parent, COUNT(*) AS n FROM t WHERE vector NEAR $v GROUP BY parent",
        &results,
    );
    assert_eq!(grouped.len(), 2);
    assert_eq!(field(&grouped[1], "parent"), Some(&serde_json::Value::Null));
}

#[test]
fn test_first_null_when_column_missing() {
    let results = vec![make_result(1, 0.9, serde_json::json!({"parent": "A"}))];
    let grouped = group(
        "SELECT parent, FIRST(nonexistent) AS val FROM t WHERE vector NEAR $v GROUP BY parent",
        &results,
    );
    assert_eq!(field(&grouped[0], "val"), Some(&serde_json::Value::Null));
}

#[test]
fn test_is_vector_group_by_true() {
    assert!(
        select("SELECT parent FROM t WHERE vector NEAR $v GROUP BY parent").is_vector_group_by()
    );
}

#[test]
fn test_is_vector_group_by_false_no_near() {
    assert!(!select("SELECT parent FROM t GROUP BY parent").is_vector_group_by());
}

#[test]
fn test_is_vector_group_by_false_no_group_by() {
    assert!(!select("SELECT * FROM t WHERE vector NEAR $v LIMIT 5").is_vector_group_by());
}

#[test]
fn test_avg_score_strategy() {
    let results = vec![
        make_result(1, 0.5, serde_json::json!({"parent": "A"})),
        make_result(2, 0.9, serde_json::json!({"parent": "A"})),
    ];
    let grouped = group(
        "SELECT parent, AVG(score) AS relevance FROM t WHERE vector NEAR $v GROUP BY parent",
        &results,
    );
    assert_eq!(grouped.len(), 1);
    let expected_avg = 0.5_f32.midpoint(0.9);
    assert!((grouped[0].score - expected_avg).abs() < 0.001);
}

#[test]
fn test_a_distance_metric_makes_the_nearest_row_the_best() {
    let results = vec![
        make_result(1, 3.0, serde_json::json!({"parent": "A"})),
        make_result(2, 1.0, serde_json::json!({"parent": "A"})),
    ];
    let stmt = select("SELECT parent, COUNT(*) AS n FROM t WHERE vector NEAR $v GROUP BY parent");
    let grouped = group_search_results(&stmt, &results, None, false).expect("test: group");
    assert_eq!(grouped[0].point.id, 2);
    assert!((grouped[0].score - 1.0).abs() < f32::EPSILON);
}
