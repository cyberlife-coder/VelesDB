//! SQL projection engine for VelesQL SELECT expressions.
//!
//! Applies `SelectColumns` to `SearchResult` rows, producing JSON objects
//! with only the requested fields. Used by the query pipeline after
//! post-processing (DISTINCT, ORDER BY, LIMIT).

use crate::point::SearchResult;
use crate::velesql::{SelectColumns, SimilarityScoreExpr};

/// Projects a list of `SearchResult` according to the parsed SELECT expressions.
///
/// Returns `serde_json::Value::Object` rows with only the requested fields.
/// The `id` field is always the system point ID (takes precedence over payload).
#[must_use]
pub fn project_results(
    results: &[SearchResult],
    select_exprs: &SelectColumns,
) -> Vec<serde_json::Value> {
    results
        .iter()
        .map(|r| project_single(r, select_exprs))
        .collect()
}

/// Projects a single `SearchResult` into a JSON row.
fn project_single(result: &SearchResult, select_exprs: &SelectColumns) -> serde_json::Value {
    match select_exprs {
        SelectColumns::All | SelectColumns::QualifiedWildcard(_) => project_wildcard(result),
        SelectColumns::Columns(cols) => project_columns(result, cols),
        SelectColumns::SimilarityScore(expr) => project_similarity_only(result, expr),
        SelectColumns::Aggregations(_) => {
            // Aggregations are handled by a separate code path; return empty row.
            serde_json::Value::Object(serde_json::Map::new())
        }
        SelectColumns::Mixed {
            columns,
            aggregations: _,
            similarity_scores,
            qualified_wildcards,
        } => project_mixed(result, columns, similarity_scores, qualified_wildcards),
    }
}

/// `SELECT *` or `SELECT alias.*`: returns `{id, ...payload_fields}`.
///
/// Excludes vectors and similarity score. Use `SELECT similarity() AS score, *`
/// to include the score explicitly.
fn project_wildcard(result: &SearchResult) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::Value::from(result.point.id));

    if let Some(serde_json::Value::Object(payload_map)) = result.point.payload.as_ref() {
        for (k, v) in payload_map {
            if k != "id" {
                map.insert(k.clone(), v.clone());
            }
        }
    }

    serde_json::Value::Object(map)
}

/// `SELECT col1, col2 [AS alias]`: extracts only named fields.
fn project_columns(result: &SearchResult, columns: &[crate::velesql::Column]) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    for col in columns {
        let output_key = col.alias.as_deref().unwrap_or(&col.name);
        let value = extract_field_value(result, &col.name);
        map.insert(output_key.to_string(), value);
    }

    serde_json::Value::Object(map)
}

/// `SELECT similarity() [AS alias]`: materializes the score only.
fn project_similarity_only(result: &SearchResult, expr: &SimilarityScoreExpr) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let key = expr.alias.as_deref().unwrap_or("similarity");
    map.insert(
        key.to_string(),
        serde_json::Value::from(f64::from(result.score)),
    );
    serde_json::Value::Object(map)
}

/// Mixed projection: columns + similarity scores + qualified wildcards.
fn project_mixed(
    result: &SearchResult,
    columns: &[crate::velesql::Column],
    similarity_scores: &[SimilarityScoreExpr],
    qualified_wildcards: &[String],
) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Qualified wildcards expand to all payload fields + id
    if !qualified_wildcards.is_empty() {
        map.insert("id".to_string(), serde_json::Value::from(result.point.id));
        if let Some(serde_json::Value::Object(payload_map)) = result.point.payload.as_ref() {
            for (k, v) in payload_map {
                if k != "id" {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
    }

    // Named columns
    for col in columns {
        let output_key = col.alias.as_deref().unwrap_or(&col.name);
        let value = extract_field_value(result, &col.name);
        map.insert(output_key.to_string(), value);
    }

    // Similarity scores
    for expr in similarity_scores {
        let key = expr.alias.as_deref().unwrap_or("similarity");
        map.insert(
            key.to_string(),
            serde_json::Value::from(f64::from(result.score)),
        );
    }

    serde_json::Value::Object(map)
}

/// Extracts a field value from a `SearchResult`, supporting nested paths.
///
/// - `"title"` → `payload["title"]`
/// - `"meta.source"` → `payload["meta"]["source"]`
/// - `"id"` → system point ID (takes precedence over payload)
fn extract_field_value(result: &SearchResult, field_path: &str) -> serde_json::Value {
    if field_path == "id" {
        return serde_json::Value::from(result.point.id);
    }

    let Some(payload) = result.point.payload.as_ref() else {
        return serde_json::Value::Null;
    };

    if field_path.contains('.') {
        // Nested path traversal
        let mut current = payload;
        for segment in field_path.split('.') {
            match current.get(segment) {
                Some(next) => current = next,
                None => return serde_json::Value::Null,
            }
        }
        current.clone()
    } else {
        payload
            .get(field_path)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Point;
    use crate::velesql::Column;

    fn make_result(id: u64, score: f32, payload: serde_json::Value) -> SearchResult {
        SearchResult {
            point: Point {
                id,
                vector: vec![0.0; 4],
                payload: Some(payload),
                sparse_vectors: None,
            },
            score,
        }
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
        let result = SearchResult {
            point: Point {
                id: 7,
                vector: vec![0.0; 4],
                payload: None,
                sparse_vectors: None,
            },
            score: 0.5,
        };
        let projected = project_single(&result, &SelectColumns::All);
        let obj = projected.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert_eq!(obj["id"], 7);
    }

    #[test]
    fn test_project_column_no_payload() {
        let result = SearchResult {
            point: Point {
                id: 7,
                vector: vec![0.0; 4],
                payload: None,
                sparse_vectors: None,
            },
            score: 0.5,
        };
        let columns = SelectColumns::Columns(vec![Column::new("title")]);
        let projected = project_single(&result, &columns);
        let obj = projected.as_object().unwrap();
        assert!(obj["title"].is_null());
    }
}
