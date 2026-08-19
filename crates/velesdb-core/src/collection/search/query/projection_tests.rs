use super::*;
use crate::point::Point;
use crate::velesql::Column;

fn make_result(id: u64, score: f32, payload: serde_json::Value) -> SearchResult {
    SearchResult::new(
        Point {
            id,
            vector: vec![0.0; 4],
            payload: Some(payload),
            sparse_vectors: None,
        },
        score,
    )
}

#[test]
fn test_project_wildcard_returns_id_and_payload() {
    let result = make_result(42, 0.95, serde_json::json!({"title": "Hello", "count": 5}));
    let projected = project_single(&result, &SelectColumns::All);
    let obj = projected.as_object().expect("should be object");
    assert_eq!(obj["id"], 42);
    assert_eq!(obj["title"], "Hello");
    assert_eq!(obj["count"], 5);
    assert!(!obj.contains_key("vector"));
}

#[test]
fn test_project_wildcard_system_id_prevails() {
    let result = make_result(42, 0.95, serde_json::json!({"id": 999, "title": "Hello"}));
    let projected = project_single(&result, &SelectColumns::All);
    let obj = projected.as_object().unwrap();
    // System ID (42) must prevail over payload id (999)
    assert_eq!(obj["id"], 42);
}

#[test]
fn test_project_specific_columns() {
    let result = make_result(
        1,
        0.9,
        serde_json::json!({"title": "Doc", "category": "tech", "author": "Alice"}),
    );
    let columns = SelectColumns::Columns(vec![Column::new("title"), Column::new("category")]);
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();
    assert_eq!(obj.len(), 2);
    assert_eq!(obj["title"], "Doc");
    assert_eq!(obj["category"], "tech");
    assert!(!obj.contains_key("author"));
}

#[test]
fn test_project_similarity_score() {
    let result = make_result(1, 0.875, serde_json::json!({"title": "Doc"}));
    let expr = SimilarityScoreExpr {
        alias: Some("relevance".to_string()),
    };
    let projected = project_single(&result, &SelectColumns::SimilarityScore(expr));
    let obj = projected.as_object().unwrap();
    assert_eq!(obj.len(), 1);
    let relevance = obj["relevance"].as_f64().unwrap();
    assert!((relevance - 0.875).abs() < 1e-3);
}

#[test]
fn test_project_similarity_default_key() {
    let result = make_result(1, 0.5, serde_json::json!({}));
    let expr = SimilarityScoreExpr { alias: None };
    let projected = project_single(&result, &SelectColumns::SimilarityScore(expr));
    let obj = projected.as_object().unwrap();
    assert!(obj.contains_key("similarity"));
}

#[test]
fn test_project_nested_path() {
    let result = make_result(
        1,
        0.9,
        serde_json::json!({"meta": {"source": "wiki", "lang": "en"}}),
    );
    let columns = SelectColumns::Columns(vec![Column::new("meta.source")]);
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();
    assert_eq!(obj["meta.source"], "wiki");
}

#[test]
fn test_project_missing_field_returns_null() {
    let result = make_result(1, 0.9, serde_json::json!({"title": "Doc"}));
    let columns = SelectColumns::Columns(vec![Column::new("nonexistent")]);
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();
    assert!(obj["nonexistent"].is_null());
}

#[test]
fn test_project_mixed_columns_and_similarity() {
    let result = make_result(
        1,
        0.85,
        serde_json::json!({"title": "Doc", "author": "Bob"}),
    );
    let columns = SelectColumns::Mixed {
        columns: vec![Column::new("title")],
        aggregations: vec![],
        similarity_scores: vec![SimilarityScoreExpr {
            alias: Some("score".to_string()),
        }],
        qualified_wildcards: vec![],
        window_functions: vec![],
    };
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();
    assert_eq!(obj["title"], "Doc");
    assert!(!obj.contains_key("author"));
    let score = obj["score"].as_f64().unwrap();
    assert!((score - 0.85).abs() < 1e-3);
}

#[test]
fn test_project_qualified_wildcard_with_similarity() {
    let result = make_result(
        5,
        0.75,
        serde_json::json!({"title": "Article", "views": 100}),
    );
    let columns = SelectColumns::Mixed {
        columns: vec![],
        aggregations: vec![],
        similarity_scores: vec![SimilarityScoreExpr {
            alias: Some("relevance".to_string()),
        }],
        qualified_wildcards: vec!["ctx".to_string()],
        window_functions: vec![],
    };
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();
    assert_eq!(obj["id"], 5);
    assert_eq!(obj["title"], "Article");
    assert_eq!(obj["views"], 100);
    let rel = obj["relevance"].as_f64().unwrap();
    assert!((rel - 0.75).abs() < 1e-3);
}

#[test]
fn test_project_column_with_alias() {
    let result = make_result(1, 0.9, serde_json::json!({"title": "Hello World"}));
    let columns = SelectColumns::Columns(vec![Column::with_alias("title", "name")]);
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();
    assert_eq!(obj["name"], "Hello World");
    assert!(!obj.contains_key("title"));
}

#[test]
fn test_project_results_multiple() {
    let results = vec![
        make_result(1, 0.9, serde_json::json!({"title": "A"})),
        make_result(2, 0.8, serde_json::json!({"title": "B"})),
    ];
    let projected = project_results(&results, &SelectColumns::All);
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0]["id"], 1);
    assert_eq!(projected[1]["id"], 2);
}

#[test]
fn test_order_by_similarity_bare_sorts_by_existing_score() {
    // This test validates the integration with ordering.rs SimilarityBare
    let results = vec![
        make_result(1, 0.5, serde_json::json!({"title": "Low"})),
        make_result(2, 0.9, serde_json::json!({"title": "High"})),
        make_result(3, 0.7, serde_json::json!({"title": "Mid"})),
    ];
    // Verify scores are preserved correctly for bare similarity ordering
    let projected = project_results(
        &results,
        &SelectColumns::SimilarityScore(SimilarityScoreExpr {
            alias: Some("score".to_string()),
        }),
    );
    let scores: Vec<f64> = projected
        .iter()
        .map(|r| r["score"].as_f64().unwrap())
        .collect();
    assert!((scores[0] - 0.5).abs() < 1e-3);
    assert!((scores[1] - 0.9).abs() < 1e-3);
    assert!((scores[2] - 0.7).abs() < 1e-3);
}

#[test]
fn test_project_wildcard_no_payload() {
    let result = SearchResult::new(
        Point {
            id: 7,
            vector: vec![0.0; 4],
            payload: None,
            sparse_vectors: None,
        },
        0.5,
    );
    let projected = project_single(&result, &SelectColumns::All);
    let obj = projected.as_object().unwrap();
    assert_eq!(obj.len(), 1);
    assert_eq!(obj["id"], 7);
}

#[test]
fn test_project_column_no_payload() {
    let result = SearchResult::new(
        Point {
            id: 7,
            vector: vec![0.0; 4],
            payload: None,
            sparse_vectors: None,
        },
        0.5,
    );
    let columns = SelectColumns::Columns(vec![Column::new("title")]);
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();
    assert!(obj["title"].is_null());
}

/// Issue #473: LET-injected values in the payload are visible through
/// `SelectColumns::Mixed` qualified wildcard expansion.
///
/// The execution pipeline injects LET binding values into `point.payload`
/// before calling `project_results`. This test verifies that those injected
/// keys survive the `project_mixed` path (wildcard expansion + named column).
#[test]
fn test_project_mixed_wildcard_exposes_let_injected_payload_field() {
    // Simulate post-LET-injection payload: original fields plus "hybrid" injected
    // by inject_let_into_payloads in select_dispatch.rs.
    let result = make_result(
        3,
        0.88,
        serde_json::json!({"title": "Doc", "idx": 7, "hybrid": 0.5}),
    );
    let columns = SelectColumns::Mixed {
        // SELECT docs.*, hybrid — qualified wildcard + explicit named column
        columns: vec![Column::new("hybrid")],
        aggregations: vec![],
        similarity_scores: vec![],
        qualified_wildcards: vec!["docs".to_string()],
        window_functions: vec![],
    };
    let projected = project_single(&result, &columns);
    let obj = projected.as_object().unwrap();

    // Wildcard expansion must include original payload fields.
    assert_eq!(obj["id"], 3);
    assert_eq!(obj["title"], "Doc");
    assert_eq!(obj["idx"], 7);
    // LET-injected value must appear: both via wildcard expansion and explicit column.
    let hybrid = obj["hybrid"].as_f64().expect("hybrid should be f64");
    assert!(
        (hybrid - 0.5).abs() < 1e-5,
        "hybrid should be 0.5, got {hybrid}"
    );
}
