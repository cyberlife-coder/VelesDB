use super::*;
use crate::point::Point;

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

fn max_score_agg(alias: Option<&str>) -> AggregateFunction {
    AggregateFunction {
        function_type: AggregateType::Max,
        argument: AggregateArg::Column("score".to_string()),
        alias: alias.map(String::from),
    }
}

fn avg_score_agg(alias: Option<&str>) -> AggregateFunction {
    AggregateFunction {
        function_type: AggregateType::Avg,
        argument: AggregateArg::Column("score".to_string()),
        alias: alias.map(String::from),
    }
}

fn first_agg(col: &str, alias: Option<&str>) -> AggregateFunction {
    AggregateFunction {
        function_type: AggregateType::First,
        argument: AggregateArg::Column(col.to_string()),
        alias: alias.map(String::from),
    }
}

#[test]
fn test_group_accumulator_single_chunk() {
    let results = vec![make_result(
        1,
        0.9,
        serde_json::json!({"parent": "A", "text": "hello"}),
    )];
    let aggs = vec![max_score_agg(Some("relevance"))];
    let config = VectorGroupByConfig {
        group_by_columns: &["parent".to_string()],
        aggregations: &aggs,
        limit_hint: Some(10),
    };
    let grouped = group_search_results(&results, &config);
    assert_eq!(grouped.len(), 1);
    assert!((grouped[0].score - 0.9).abs() < f32::EPSILON);
}

#[test]
fn test_group_accumulator_multiple_chunks() {
    let results = vec![
        make_result(1, 0.5, serde_json::json!({"parent": "A", "text": "low"})),
        make_result(2, 0.9, serde_json::json!({"parent": "A", "text": "high"})),
        make_result(3, 0.7, serde_json::json!({"parent": "A", "text": "mid"})),
        make_result(4, 0.8, serde_json::json!({"parent": "B", "text": "only"})),
    ];
    let aggs = vec![
        max_score_agg(Some("relevance")),
        first_agg("text", Some("excerpt")),
    ];
    let config = VectorGroupByConfig {
        group_by_columns: &["parent".to_string()],
        aggregations: &aggs,
        limit_hint: Some(10),
    };
    let grouped = group_search_results(&results, &config);
    assert_eq!(grouped.len(), 2);

    let group_a = grouped
        .iter()
        .find(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("parent"))
                .and_then(|v| v.as_str())
                == Some("A")
        })
        .expect("group A");
    assert!((group_a.score - 0.9).abs() < f32::EPSILON);
    let excerpt = group_a
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("excerpt"))
        .and_then(|v| v.as_str());
    assert_eq!(excerpt, Some("high"));
}

#[test]
fn test_group_skip_missing_parent_field() {
    let results = vec![
        make_result(1, 0.9, serde_json::json!({"parent": "A"})),
        make_result(2, 0.8, serde_json::json!({"other": "no parent"})),
        make_result(3, 0.7, serde_json::json!({"parent": "A"})),
    ];
    let aggs = vec![max_score_agg(Some("relevance"))];
    let config = VectorGroupByConfig {
        group_by_columns: &["parent".to_string()],
        aggregations: &aggs,
        limit_hint: Some(10),
    };
    let grouped = group_search_results(&results, &config);
    assert_eq!(grouped.len(), 1);
}

#[test]
fn test_first_null_when_column_missing() {
    let results = vec![make_result(1, 0.9, serde_json::json!({"parent": "A"}))];
    let aggs = vec![first_agg("nonexistent", Some("val"))];
    let config = VectorGroupByConfig {
        group_by_columns: &["parent".to_string()],
        aggregations: &aggs,
        limit_hint: Some(10),
    };
    let grouped = group_search_results(&results, &config);
    assert_eq!(grouped.len(), 1);
    let val = grouped[0].point.payload.as_ref().and_then(|p| p.get("val"));
    assert_eq!(val, Some(&serde_json::Value::Null));
}

#[test]
fn test_is_vector_group_by_query_true() {
    let stmt = SelectStatement {
        group_by: Some(crate::velesql::GroupByClause {
            columns: vec!["parent".to_string()],
        }),
        where_clause: Some(crate::velesql::Condition::VectorSearch(
            crate::velesql::VectorSearch {
                vector: crate::velesql::VectorExpr::Literal(vec![1.0, 0.0, 0.0, 0.0]),
            },
        )),
        ..SelectStatement::empty()
    };
    assert!(is_vector_group_by_query(&stmt));
}

#[test]
fn test_is_vector_group_by_query_false_no_near() {
    let stmt = SelectStatement {
        group_by: Some(crate::velesql::GroupByClause {
            columns: vec!["parent".to_string()],
        }),
        where_clause: None,
        ..SelectStatement::empty()
    };
    assert!(!is_vector_group_by_query(&stmt));
}

#[test]
fn test_is_vector_group_by_query_false_no_group_by() {
    let stmt = SelectStatement {
        group_by: None,
        where_clause: Some(crate::velesql::Condition::VectorSearch(
            crate::velesql::VectorSearch {
                vector: crate::velesql::VectorExpr::Literal(vec![1.0, 0.0, 0.0, 0.0]),
            },
        )),
        ..SelectStatement::empty()
    };
    assert!(!is_vector_group_by_query(&stmt));
}

#[test]
fn test_avg_score_strategy() {
    let results = vec![
        make_result(1, 0.5, serde_json::json!({"parent": "A"})),
        make_result(2, 0.9, serde_json::json!({"parent": "A"})),
    ];
    let aggs = vec![avg_score_agg(Some("relevance"))];
    let config = VectorGroupByConfig {
        group_by_columns: &["parent".to_string()],
        aggregations: &aggs,
        limit_hint: Some(10),
    };
    let grouped = group_search_results(&results, &config);
    assert_eq!(grouped.len(), 1);
    let expected_avg = 0.5_f32.midpoint(0.9);
    assert!((grouped[0].score - expected_avg).abs() < 0.001);
}
