use super::*;
use velesdb_core::point::Point;
use velesdb_core::velesql::{ArithmeticExpr, ArithmeticOp, OrderByExpr, SelectOrderBy};

fn mk(id: u64, score: f32, payload: serde_json::Value) -> SearchResult {
    SearchResult::new(Point::new(id, Vec::new(), Some(payload)), score)
}

fn by_field(name: &str, desc: bool) -> Vec<SelectOrderBy> {
    vec![SelectOrderBy {
        expr: OrderByExpr::Field(name.to_string()),
        descending: desc,
    }]
}

fn sort(stmt_order: Vec<SelectOrderBy>, rows: &mut [SearchResult]) {
    let mut stmt = SelectStatement::empty();
    stmt.order_by = Some(stmt_order);
    sort_rows(&stmt, rows, &OrderContext::without_vectors(true)).expect("test: sort");
}

#[test]
fn similarity_score_order_keeps_nan_after_numbers_in_both_directions() {
    for descending in [true, false] {
        for higher in [true, false] {
            let mut rows = vec![
                mk(5, -f32::NAN, serde_json::Value::Null),
                mk(4, f32::NAN, serde_json::Value::Null),
                mk(3, -0.0, serde_json::Value::Null),
                mk(2, 0.0, serde_json::Value::Null),
                mk(1, 1.0, serde_json::Value::Null),
            ];
            let mut stmt = SelectStatement::empty();
            stmt.order_by = Some(vec![SelectOrderBy {
                expr: OrderByExpr::SimilarityBare,
                descending,
            }]);
            sort_rows(&stmt, &mut rows, &OrderContext::without_vectors(higher))
                .expect("test: sort");
            let ids: Vec<_> = rows.iter().map(|r| r.point.id).collect();
            let expected = if descending == higher {
                [1, 2, 3, 4, 5]
            } else {
                [2, 3, 1, 4, 5]
            };
            assert_eq!(ids, expected);
        }
    }
}

#[test]
fn test_sort_by_id_asc() {
    let mut rows = vec![
        mk(3, 0.0, serde_json::json!({})),
        mk(1, 0.0, serde_json::json!({})),
        mk(2, 0.0, serde_json::json!({})),
    ];
    sort(by_field("id", false), &mut rows);
    assert_eq!(rows[0].point.id, 1);
    assert_eq!(rows[2].point.id, 3);
}

#[test]
fn test_sort_by_payload_column_desc() {
    let mut rows = vec![
        mk(1, 0.0, serde_json::json!({"price": 20})),
        mk(2, 0.0, serde_json::json!({"price": 10})),
        mk(3, 0.0, serde_json::json!({"price": 30})),
    ];
    sort(by_field("price", true), &mut rows);
    assert_eq!(rows[0].point.id, 3);
    assert_eq!(rows[2].point.id, 2);
}

#[test]
fn test_sort_nulls_last_asc() {
    let mut rows = vec![
        mk(1, 0.0, serde_json::json!({"x": 5})),
        mk(2, 0.0, serde_json::json!({})),
        mk(3, 0.0, serde_json::json!({"x": 1})),
    ];
    sort(by_field("x", false), &mut rows);
    assert_eq!(rows[0].point.id, 3);
    assert_eq!(rows[2].point.id, 2);
}

#[test]
fn test_sort_by_similarity_bare_desc() {
    let mut rows = vec![
        mk(1, 0.1, serde_json::json!({})),
        mk(2, 0.9, serde_json::json!({})),
        mk(3, 0.5, serde_json::json!({})),
    ];
    sort(
        vec![SelectOrderBy {
            expr: OrderByExpr::SimilarityBare,
            descending: true,
        }],
        &mut rows,
    );
    assert_eq!(rows[0].point.id, 2);
    assert_eq!(rows[2].point.id, 1);
}

#[test]
fn test_sort_by_arithmetic_formula() {
    // ORDER BY (price - 2*score) ASC.
    let expr = ArithmeticExpr::BinaryOp {
        left: Box::new(ArithmeticExpr::Variable("price".to_string())),
        op: ArithmeticOp::Sub,
        right: Box::new(ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Literal(2.0)),
            op: ArithmeticOp::Mul,
            right: Box::new(ArithmeticExpr::Variable("score".to_string())),
        }),
    };
    let mut rows = vec![
        mk(1, 1.0, serde_json::json!({"price": 10})), // 10 - 2 = 8
        mk(2, 0.0, serde_json::json!({"price": 1})),  // 1
        mk(3, 0.0, serde_json::json!({"price": 30})), // 30
    ];
    sort(
        vec![SelectOrderBy {
            expr: OrderByExpr::Arithmetic(expr),
            descending: false,
        }],
        &mut rows,
    );
    assert_eq!(rows[0].point.id, 2); // 1
    assert_eq!(rows[1].point.id, 1); // 8
    assert_eq!(rows[2].point.id, 3); // 30
}

#[test]
fn named_similarity_over_rows_of_no_collection_is_rejected() {
    use velesdb_core::velesql::{SimilarityOrderBy, VectorExpr};
    let mut rows = vec![mk(1, 0.5, serde_json::json!({}))];
    let mut stmt = SelectStatement::empty();
    stmt.order_by = Some(vec![SelectOrderBy {
        expr: OrderByExpr::Similarity(SimilarityOrderBy {
            field: "image_vec".to_string(),
            vector: VectorExpr::Parameter("q".to_string()),
        }),
        descending: true,
    }]);
    let err = sort_rows(&stmt, &mut rows, &OrderContext::without_vectors(true));
    assert!(err.is_err());
}

#[test]
fn test_division_by_zero_yields_zero() {
    let expr = ArithmeticExpr::BinaryOp {
        left: Box::new(ArithmeticExpr::Literal(5.0)),
        op: ArithmeticOp::Div,
        right: Box::new(ArithmeticExpr::Literal(0.0)),
    };
    assert!(eval_arithmetic(&expr, &mk(1, 0.5, serde_json::json!({}))).abs() < f32::EPSILON);
}

/// Score variables and `id` resolve as core's `ScoreContext` resolves them:
/// `score` is the search score, `id` the point id, a component the row's
/// breakdown (0.0 when a tagged row lacks it).
#[test]
fn score_variables_and_id_resolve_as_in_core() {
    let mut row = mk(7, 0.25, serde_json::json!({"score": 99}));
    row.component_scores = Some(std::iter::once(("vector_score", 0.25)).collect());
    let var = |name: &str| ArithmeticExpr::Variable(name.to_string());
    assert!(
        (eval_arithmetic(&var("score"), &row) - 0.25).abs() < f32::EPSILON,
        "not the payload's 99"
    );
    assert!((eval_arithmetic(&var("id"), &row) - 7.0).abs() < f32::EPSILON);
    assert!((eval_arithmetic(&var("vector_score"), &row) - 0.25).abs() < f32::EPSILON);
    assert!(eval_arithmetic(&var("bm25_score"), &row).abs() < f32::EPSILON);
}

#[test]
fn test_similarity_desc_puts_the_nearest_row_first_under_a_distance_metric() {
    // Euclidean scores are distances: the smallest is the most similar.
    let mut rows = vec![
        mk(1, 3.0, serde_json::json!({})),
        mk(2, 1.0, serde_json::json!({})),
        mk(3, 2.0, serde_json::json!({})),
    ];
    let mut stmt = SelectStatement::empty();
    stmt.order_by = Some(vec![SelectOrderBy {
        expr: OrderByExpr::SimilarityBare,
        descending: true,
    }]);
    sort_rows(&stmt, &mut rows, &OrderContext::without_vectors(false)).expect("test: sort");
    let ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, [2, 3, 1]);
}
