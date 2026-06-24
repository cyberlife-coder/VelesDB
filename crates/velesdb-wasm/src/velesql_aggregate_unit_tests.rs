//! Internal unit tests for [`crate::velesql_aggregate`].
//!
//! Lives in a dedicated module file so the production module stays under
//! the 500-line NLOC ceiling enforced by Codacy.

use velesdb_core::velesql::{
    AggregateArg, AggregateFunction, AggregateType, Column, CompareOp, DistinctMode, GroupByClause,
    HavingClause, HavingCondition, OrderByExpr, SelectColumns, SelectOrderBy, SelectStatement,
    Value,
};

use crate::velesql_aggregate::{apply, compute_aggregate, needs_aggregation_pipeline, ScannedRow};
use crate::velesql_value::Params;

fn row(id: u64, score: f32, payload: &serde_json::Value) -> (u64, f32, serde_json::Value) {
    (id, score, payload.clone())
}

fn scanned<'a>(rows: &'a [(u64, f32, serde_json::Value)]) -> Vec<ScannedRow<'a>> {
    rows.iter().map(|(id, s, p)| (*id, *s, Some(p))).collect()
}

fn base_select() -> SelectStatement {
    let mut s = SelectStatement::empty();
    s.from = "t".to_string();
    s
}

#[test]
fn test_needs_pipeline_flags_distinct() {
    let mut s = base_select();
    s.distinct = DistinctMode::All;
    assert!(needs_aggregation_pipeline(&s));
}

#[test]
fn test_needs_pipeline_flags_group_by() {
    let mut s = base_select();
    s.group_by = Some(GroupByClause {
        columns: vec!["cat".to_string()],
    });
    assert!(needs_aggregation_pipeline(&s));
}

#[test]
fn test_needs_pipeline_false_plain_select() {
    let s = base_select();
    assert!(!needs_aggregation_pipeline(&s));
}

#[test]
fn test_count_star_global() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"cat": "a"})),
        row(2, 0.0, &serde_json::json!({"cat": "b"})),
        row(3, 0.0, &serde_json::json!({"cat": "a"})),
    ];
    let rows = scanned(&raw);
    let mut s = base_select();
    s.columns = SelectColumns::Aggregations(vec![AggregateFunction {
        function_type: AggregateType::Count,
        argument: AggregateArg::Wildcard,
        alias: Some("total".to_string()),
    }]);
    let out = apply(&s, &rows, &Params::new()).expect("test: agg");
    assert_eq!(out.len(), 1);
    assert!(out[0].data_json_ref().contains("\"total\":3"));
}

#[test]
fn test_group_by_count() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"cat": "a"})),
        row(2, 0.0, &serde_json::json!({"cat": "b"})),
        row(3, 0.0, &serde_json::json!({"cat": "a"})),
    ];
    let rows = scanned(&raw);
    let mut s = base_select();
    s.columns = SelectColumns::Aggregations(vec![AggregateFunction {
        function_type: AggregateType::Count,
        argument: AggregateArg::Wildcard,
        alias: Some("n".to_string()),
    }]);
    s.group_by = Some(GroupByClause {
        columns: vec!["cat".to_string()],
    });
    let out = apply(&s, &rows, &Params::new()).expect("test: agg");
    assert_eq!(out.len(), 2);
    let json: Vec<&str> = out.iter().map(|r| r.data_json_ref()).collect();
    // cat "a" appears twice -> n:2 ; cat "b" once -> n:1
    assert!(
        json.iter()
            .any(|j| j.contains("\"cat\":\"a\"") && j.contains("\"n\":2")),
        "expected group cat=a with n=2, got {json:?}"
    );
    assert!(
        json.iter()
            .any(|j| j.contains("\"cat\":\"b\"") && j.contains("\"n\":1")),
        "expected group cat=b with n=1, got {json:?}"
    );
}

/// Helper: index of the first row whose JSON contains `needle`.
fn pos(rows: &[crate::velesql_result::QueryResultRow], needle: &str) -> usize {
    rows.iter()
        .position(|r| r.data_json_ref().contains(needle))
        .unwrap_or_else(|| panic!("expected a row containing {needle}"))
}

#[test]
fn test_group_by_order_by_count_desc() {
    // GROUP BY cat ORDER BY COUNT(*) DESC must return groups in
    // count-descending order. cat "a" has 3 rows, "b" has 1, "c" has 2.
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"cat": "a"})),
        row(2, 0.0, &serde_json::json!({"cat": "b"})),
        row(3, 0.0, &serde_json::json!({"cat": "a"})),
        row(4, 0.0, &serde_json::json!({"cat": "c"})),
        row(5, 0.0, &serde_json::json!({"cat": "a"})),
        row(6, 0.0, &serde_json::json!({"cat": "c"})),
    ];
    let rows = scanned(&raw);
    let mut s = base_select();
    let count_star = AggregateFunction {
        function_type: AggregateType::Count,
        argument: AggregateArg::Wildcard,
        alias: None,
    };
    s.columns = SelectColumns::Aggregations(vec![count_star.clone()]);
    s.group_by = Some(GroupByClause {
        columns: vec!["cat".to_string()],
    });
    s.order_by = Some(vec![SelectOrderBy {
        expr: OrderByExpr::Aggregate(count_star),
        descending: true,
    }]);
    let out = apply(&s, &rows, &Params::new()).expect("test: agg order by");
    assert_eq!(out.len(), 3);
    // Count-descending: a (3) before c (2) before b (1).
    assert!(pos(&out, "\"cat\":\"a\"") < pos(&out, "\"cat\":\"c\""));
    assert!(pos(&out, "\"cat\":\"c\"") < pos(&out, "\"cat\":\"b\""));
}

#[test]
fn test_group_by_order_by_group_key_asc() {
    // ORDER BY a group key (cat ASC) must sort groups alphabetically.
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"cat": "c"})),
        row(2, 0.0, &serde_json::json!({"cat": "a"})),
        row(3, 0.0, &serde_json::json!({"cat": "b"})),
    ];
    let rows = scanned(&raw);
    let mut s = base_select();
    s.columns = SelectColumns::Aggregations(vec![AggregateFunction {
        function_type: AggregateType::Count,
        argument: AggregateArg::Wildcard,
        alias: Some("n".to_string()),
    }]);
    s.group_by = Some(GroupByClause {
        columns: vec!["cat".to_string()],
    });
    s.order_by = Some(vec![SelectOrderBy {
        expr: OrderByExpr::Field("cat".to_string()),
        descending: false,
    }]);
    let out = apply(&s, &rows, &Params::new()).expect("test: agg order by key");
    assert_eq!(out.len(), 3);
    assert!(pos(&out, "\"cat\":\"a\"") < pos(&out, "\"cat\":\"b\""));
    assert!(pos(&out, "\"cat\":\"b\"") < pos(&out, "\"cat\":\"c\""));
}

#[test]
fn test_avg_numeric() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"price": 10})),
        row(2, 0.0, &serde_json::json!({"price": 20})),
        row(3, 0.0, &serde_json::json!({"price": 30})),
    ];
    let rows = scanned(&raw);
    let mut s = base_select();
    s.columns = SelectColumns::Aggregations(vec![AggregateFunction {
        function_type: AggregateType::Avg,
        argument: AggregateArg::Column("price".to_string()),
        alias: Some("avg_p".to_string()),
    }]);
    let out = apply(&s, &rows, &Params::new()).expect("test: avg");
    assert!(out[0].data_json_ref().contains("\"avg_p\":20"));
}

#[test]
fn test_distinct_dedups() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"cat": "a"})),
        row(2, 0.0, &serde_json::json!({"cat": "a"})),
        row(3, 0.0, &serde_json::json!({"cat": "b"})),
    ];
    let rows = scanned(&raw);
    let mut s = base_select();
    s.distinct = DistinctMode::All;
    s.columns = SelectColumns::Columns(vec![Column::new("cat")]);
    let out = apply(&s, &rows, &Params::new()).expect("test: distinct");
    assert_eq!(out.len(), 2);
    let json: Vec<String> = out.iter().map(|r| r.data_json_ref().to_string()).collect();
    // both distinct values survive, exactly once each
    assert_eq!(
        json.iter().filter(|j| j.contains("\"cat\":\"a\"")).count(),
        1
    );
    assert_eq!(
        json.iter().filter(|j| j.contains("\"cat\":\"b\"")).count(),
        1
    );
}

#[test]
fn test_min_max() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"p": 5})),
        row(2, 0.0, &serde_json::json!({"p": 1})),
        row(3, 0.0, &serde_json::json!({"p": 9})),
    ];
    let rows = scanned(&raw);
    let min_v = compute_aggregate(
        &AggregateFunction {
            function_type: AggregateType::Min,
            argument: AggregateArg::Column("p".to_string()),
            alias: None,
        },
        &rows,
    )
    .expect("test: min");
    let max_v = compute_aggregate(
        &AggregateFunction {
            function_type: AggregateType::Max,
            argument: AggregateArg::Column("p".to_string()),
            alias: None,
        },
        &rows,
    )
    .expect("test: max");
    assert_eq!(min_v.as_f64().expect("min num"), 1.0);
    assert_eq!(max_v.as_f64().expect("max num"), 9.0);
}

#[test]
fn test_count_ignores_null_column() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"x": 1})),
        row(2, 0.0, &serde_json::json!({"x": serde_json::Value::Null})),
        row(3, 0.0, &serde_json::json!({"other": 1})),
    ];
    let rows = scanned(&raw);
    let v = compute_aggregate(
        &AggregateFunction {
            function_type: AggregateType::Count,
            argument: AggregateArg::Column("x".to_string()),
            alias: None,
        },
        &rows,
    )
    .expect("test: count");
    assert_eq!(v.as_u64().expect("int"), 1);
}

#[test]
fn test_having_filters_groups() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"cat": "a"})),
        row(2, 0.0, &serde_json::json!({"cat": "a"})),
        row(3, 0.0, &serde_json::json!({"cat": "b"})),
    ];
    let rows = scanned(&raw);
    let mut s = base_select();
    s.columns = SelectColumns::Aggregations(vec![AggregateFunction {
        function_type: AggregateType::Count,
        argument: AggregateArg::Wildcard,
        alias: Some("n".to_string()),
    }]);
    s.group_by = Some(GroupByClause {
        columns: vec!["cat".to_string()],
    });
    s.having = Some(HavingClause {
        conditions: vec![HavingCondition {
            aggregate: AggregateFunction {
                function_type: AggregateType::Count,
                argument: AggregateArg::Wildcard,
                alias: None,
            },
            operator: CompareOp::Gt,
            value: Value::Integer(1),
        }],
        operators: Vec::new(),
    });
    let out = apply(&s, &rows, &Params::new()).expect("test: having");
    assert_eq!(out.len(), 1);
    assert!(out[0].data_json_ref().contains("\"cat\":\"a\""));
}
