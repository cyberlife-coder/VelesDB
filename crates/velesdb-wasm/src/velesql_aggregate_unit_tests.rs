//! Internal unit tests for [`crate::velesql_aggregate`].
//!
//! Lives in a dedicated module file so the production module stays under
//! the 500-line NLOC ceiling enforced by Codacy.

use velesdb_core::velesql::{
    AggregateArg, AggregateFunction, AggregateType, CompareOp, DistinctMode, GroupByClause,
    HavingClause, HavingCondition, OrderByExpr, SelectColumns, SelectOrderBy, SelectStatement,
    Value,
};

use crate::velesql_aggregate::{apply, needs_aggregation_pipeline, ScannedRow};
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
fn test_needs_pipeline_leaves_plain_distinct_to_the_select_pipeline() {
    // DISTINCT without aggregates runs core's `apply_distinct` in
    // `velesql_select::finalize_plain`, where ORDER BY and LIMIT apply.
    let mut s = base_select();
    s.distinct = DistinctMode::All;
    assert!(!needs_aggregation_pipeline(&s));
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
    let out = apply(&s, &rows, &Params::new(), true).expect("test: agg");
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
    let out = apply(&s, &rows, &Params::new(), true).expect("test: agg");
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
    let out = apply(&s, &rows, &Params::new(), true).expect("test: agg order by");
    assert_eq!(out.len(), 3);
    // Count-descending: a (3) before c (2) before b (1).
    assert!(pos(&out, "\"cat\":\"a\"") < pos(&out, "\"cat\":\"c\""));
    assert!(pos(&out, "\"cat\":\"c\"") < pos(&out, "\"cat\":\"b\""));
}

/// `SELECT cat, COUNT(*) AS n ... GROUP BY cat` over rows where cat "a" has 3
/// rows, "b" 1 and "c" 2, in first-seen order a, b, c.
fn aliased_count_by_cat() -> (Vec<(u64, f32, serde_json::Value)>, SelectStatement) {
    let raw = ["a", "b", "a", "c", "a", "c"]
        .iter()
        .zip(1..)
        .map(|(cat, id)| row(id, 0.0, &serde_json::json!({"cat": cat, "x": id})))
        .collect();
    let mut s = base_select();
    s.columns = SelectColumns::Aggregations(vec![AggregateFunction {
        function_type: AggregateType::Count,
        argument: AggregateArg::Wildcard,
        alias: Some("n".to_string()),
    }]);
    s.group_by = Some(GroupByClause {
        columns: vec!["cat".to_string()],
    });
    (raw, s)
}

fn order_by_aggregate(function_type: AggregateType, argument: AggregateArg) -> SelectOrderBy {
    SelectOrderBy {
        expr: OrderByExpr::Aggregate(AggregateFunction {
            function_type,
            argument,
            alias: None,
        }),
        descending: true,
    }
}

#[test]
fn test_group_by_order_by_count_expression_sorts_groups_selected_under_an_alias() {
    // The parser gives `ORDER BY COUNT(*)` no alias, while the SELECT stores
    // the count under `n`: the sort must still find it there.
    let (raw, mut s) = aliased_count_by_cat();
    let rows = scanned(&raw);
    s.order_by = Some(vec![order_by_aggregate(
        AggregateType::Count,
        AggregateArg::Wildcard,
    )]);
    let out = apply(&s, &rows, &Params::new(), true).expect("test: aliased agg order by");
    assert!(pos(&out, "\"cat\":\"a\"") < pos(&out, "\"cat\":\"c\""));
    assert!(pos(&out, "\"cat\":\"c\"") < pos(&out, "\"cat\":\"b\""));
}

#[test]
fn test_group_by_order_by_aggregate_missing_from_select_is_rejected() {
    let (raw, mut s) = aliased_count_by_cat();
    let rows = scanned(&raw);
    s.order_by = Some(vec![order_by_aggregate(
        AggregateType::Sum,
        AggregateArg::Column("x".to_string()),
    )]);
    let err = apply(&s, &rows, &Params::new(), true)
        .expect_err("test: ORDER BY an aggregate the rows do not carry must fail");
    assert!(err.contains("V016"), "got: {err}");
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
    let out = apply(&s, &rows, &Params::new(), true).expect("test: agg order by key");
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
    let out = apply(&s, &rows, &Params::new(), true).expect("test: avg");
    assert!(out[0].data_json_ref().contains("\"avg_p\":20"));
}

/// The single row of `SELECT <sql aggregates> FROM t` over `raw`.
fn global_row(
    columns: Vec<AggregateFunction>,
    raw: &[(u64, f32, serde_json::Value)],
) -> serde_json::Value {
    let mut s = base_select();
    s.columns = SelectColumns::Aggregations(columns);
    let out = apply(&s, &scanned(raw), &Params::new(), true).expect("test: aggregate");
    serde_json::from_str(out[0].data_json_ref()).expect("test: row JSON")
}

fn agg(function_type: AggregateType, column: &str) -> AggregateFunction {
    AggregateFunction {
        function_type,
        argument: AggregateArg::Column(column.to_string()),
        alias: None,
    }
}

#[test]
fn test_min_max() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"p": 5})),
        row(2, 0.0, &serde_json::json!({"p": 1})),
        row(3, 0.0, &serde_json::json!({"p": 9})),
    ];
    let out = global_row(
        vec![agg(AggregateType::Min, "p"), agg(AggregateType::Max, "p")],
        &raw,
    );
    assert_eq!(out["min_p"].as_f64(), Some(1.0));
    assert_eq!(out["max_p"].as_f64(), Some(9.0));
}

#[test]
fn test_count_ignores_null_column() {
    let raw = vec![
        row(1, 0.0, &serde_json::json!({"x": 1})),
        row(2, 0.0, &serde_json::json!({"x": serde_json::Value::Null})),
        row(3, 0.0, &serde_json::json!({"other": 1})),
    ];
    let out = global_row(vec![agg(AggregateType::Count, "x")], &raw);
    assert_eq!(out["count_x"].as_u64(), Some(1));
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
    let out = apply(&s, &rows, &Params::new(), true).expect("test: having");
    assert_eq!(out.len(), 1);
    assert!(out[0].data_json_ref().contains("\"cat\":\"a\""));
}

/// Parses `sql` into its SELECT statement.
fn select(sql: &str) -> SelectStatement {
    velesdb_core::velesql::Parser::parse(sql)
        .expect("test: parse")
        .select
}

/// Group names of `out`, in row order, read from field `column`.
fn column_values(out: &[crate::velesql_result::QueryResultRow], column: &str) -> Vec<String> {
    out.iter()
        .map(|r| {
            let row: serde_json::Value =
                serde_json::from_str(r.data_json_ref()).expect("test: row JSON");
            row[column].to_string()
        })
        .collect()
}

/// Rows of a vector search, best first: cat "a" scores 0.9 and 0.2, "c" 0.7
/// and 0.6, "b" 0.5.
fn ranked_rows() -> Vec<(u64, f32, serde_json::Value)> {
    [
        (1, 0.9, "a"),
        (2, 0.7, "c"),
        (3, 0.6, "c"),
        (4, 0.5, "b"),
        (5, 0.2, "a"),
    ]
    .iter()
    .map(|&(id, score, cat)| row(id, score, &serde_json::json!({ "cat": cat })))
    .collect()
}

#[test]
fn test_vector_group_by_order_by_similarity_sorts_groups_by_their_best_score() {
    // A vector GROUP BY orders groups by score, as core does, instead of
    // refusing ORDER BY similarity() like a plain GROUP BY.
    let raw = ranked_rows();
    let s = select(
        "SELECT cat, MAX(score) AS best FROM t WHERE vector NEAR $v \
         GROUP BY cat ORDER BY similarity() ASC",
    );
    let out = apply(&s, &scanned(&raw), &Params::new(), true).expect("test: vector group by");
    assert_eq!(column_values(&out, "cat"), ["\"b\"", "\"c\"", "\"a\""]);
}

#[test]
fn test_vector_group_by_order_by_similarity_uses_the_average_when_only_avg_score_is_selected() {
    // Core scores a group by its average when the SELECT asks for AVG(score)
    // and not MAX(score): a averages 0.55, c 0.65, b 0.5.
    let raw = ranked_rows();
    let s = select(
        "SELECT cat, AVG(score) AS mean FROM t WHERE vector NEAR $v \
         GROUP BY cat ORDER BY similarity() DESC",
    );
    let out = apply(&s, &scanned(&raw), &Params::new(), true).expect("test: vector group by avg");
    assert_eq!(column_values(&out, "cat"), ["\"c\"", "\"a\"", "\"b\""]);
}
