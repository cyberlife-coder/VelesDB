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
    sort_rows(&stmt, rows).expect("test: sort");
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
fn test_sort_named_similarity_is_rejected() {
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
    let err = sort_rows(&stmt, &mut rows);
    assert!(err.is_err());
}

#[test]
fn test_division_by_zero_yields_zero() {
    assert_eq!(apply_op(ArithmeticOp::Div, 5.0, 0.0), 0.0);
}
