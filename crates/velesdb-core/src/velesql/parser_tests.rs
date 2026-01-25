//! Tests for parser module

use super::*;

// ========== Basic SELECT tests ==========

#[test]
fn test_parse_select_all() {
    let query = Parser::parse("SELECT * FROM documents").unwrap();
    assert_eq!(query.select.columns, SelectColumns::All);
    assert_eq!(query.select.from, "documents");
    assert!(query.select.where_clause.is_none());
    assert!(query.select.limit.is_none());
}

#[test]
fn test_parse_select_with_limit() {
    let query = Parser::parse("SELECT * FROM documents LIMIT 10").unwrap();
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_parse_select_with_offset() {
    let query = Parser::parse("SELECT * FROM documents LIMIT 10 OFFSET 5").unwrap();
    assert_eq!(query.select.limit, Some(10));
    assert_eq!(query.select.offset, Some(5));
}

#[test]
fn test_parse_select_columns() {
    let query = Parser::parse("SELECT id, score FROM documents").unwrap();
    match query.select.columns {
        SelectColumns::Columns(cols) => {
            assert_eq!(cols.len(), 2);
            assert_eq!(cols[0].name, "id");
            assert_eq!(cols[1].name, "score");
        }
        _ => panic!("Expected columns list"),
    }
}

#[test]
fn test_parse_select_nested_column() {
    let query = Parser::parse("SELECT payload.title FROM documents").unwrap();
    match query.select.columns {
        SelectColumns::Columns(cols) => {
            assert_eq!(cols[0].name, "payload.title");
        }
        _ => panic!("Expected columns list"),
    }
}

// ========== Vector search tests ==========

#[test]
fn test_parse_vector_near_parameter() {
    let query = Parser::parse("SELECT * FROM documents WHERE vector NEAR $v").unwrap();
    match query.select.where_clause {
        Some(Condition::VectorSearch(vs)) => {
            assert_eq!(vs.vector, VectorExpr::Parameter("v".to_string()));
        }
        _ => panic!("Expected vector search condition"),
    }
}

#[test]
fn test_parse_vector_near_literal() {
    let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3]").unwrap();
    match query.select.where_clause {
        Some(Condition::VectorSearch(vs)) => match vs.vector {
            VectorExpr::Literal(v) => {
                assert_eq!(v.len(), 3);
                assert!((v[0] - 0.1).abs() < 0.001);
            }
            VectorExpr::Parameter(_) => panic!("Expected literal vector"),
        },
        _ => panic!("Expected vector search condition"),
    }
}

// ========== Comparison tests ==========

#[test]
fn test_parse_comparison_eq_string() {
    let query = Parser::parse("SELECT * FROM docs WHERE category = 'tech'").unwrap();
    match query.select.where_clause {
        Some(Condition::Comparison(c)) => {
            assert_eq!(c.column, "category");
            assert_eq!(c.operator, CompareOp::Eq);
            assert_eq!(c.value, Value::String("tech".to_string()));
        }
        _ => panic!("Expected comparison condition"),
    }
}

#[test]
fn test_parse_comparison_gt_integer() {
    let query = Parser::parse("SELECT * FROM docs WHERE price > 100").unwrap();
    match query.select.where_clause {
        Some(Condition::Comparison(c)) => {
            assert_eq!(c.column, "price");
            assert_eq!(c.operator, CompareOp::Gt);
            assert_eq!(c.value, Value::Integer(100));
        }
        _ => panic!("Expected comparison condition"),
    }
}

#[test]
fn test_parse_comparison_neq() {
    let query = Parser::parse("SELECT * FROM docs WHERE status != 'deleted'").unwrap();
    match query.select.where_clause {
        Some(Condition::Comparison(c)) => {
            assert_eq!(c.operator, CompareOp::NotEq);
        }
        _ => panic!("Expected comparison condition"),
    }
}

// ========== Logical operators tests ==========

#[test]
fn test_parse_and_condition() {
    let query =
        Parser::parse("SELECT * FROM docs WHERE category = 'tech' AND price > 100").unwrap();
    match query.select.where_clause {
        Some(Condition::And(_, _)) => {}
        _ => panic!("Expected AND condition"),
    }
}

#[test]
fn test_parse_or_condition() {
    let query = Parser::parse("SELECT * FROM docs WHERE category = 'tech' OR category = 'science'")
        .unwrap();
    match query.select.where_clause {
        Some(Condition::Or(_, _)) => {}
        _ => panic!("Expected OR condition"),
    }
}

#[test]
fn test_parse_vector_with_filter() {
    let query =
        Parser::parse("SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10")
            .unwrap();
    match query.select.where_clause {
        Some(Condition::And(left, _)) => match *left {
            Condition::VectorSearch(_) => {}
            _ => panic!("Expected vector search on left"),
        },
        _ => panic!("Expected AND condition"),
    }
    assert_eq!(query.select.limit, Some(10));
}

// ========== IN/BETWEEN/LIKE tests ==========

#[test]
fn test_parse_in_condition() {
    let query = Parser::parse("SELECT * FROM docs WHERE category IN ('tech', 'science')").unwrap();
    match query.select.where_clause {
        Some(Condition::In(c)) => {
            assert_eq!(c.column, "category");
            assert_eq!(c.values.len(), 2);
        }
        _ => panic!("Expected IN condition"),
    }
}

#[test]
fn test_parse_between_condition() {
    let query = Parser::parse("SELECT * FROM docs WHERE price BETWEEN 10 AND 100").unwrap();
    match query.select.where_clause {
        Some(Condition::Between(c)) => {
            assert_eq!(c.column, "price");
            assert_eq!(c.low, Value::Integer(10));
            assert_eq!(c.high, Value::Integer(100));
        }
        _ => panic!("Expected BETWEEN condition"),
    }
}

#[test]
fn test_parse_like_condition() {
    let query = Parser::parse("SELECT * FROM docs WHERE title LIKE '%rust%'").unwrap();
    match query.select.where_clause {
        Some(Condition::Like(c)) => {
            assert_eq!(c.column, "title");
            assert_eq!(c.pattern, "%rust%");
            assert!(!c.case_insensitive); // LIKE is case-sensitive
        }
        _ => panic!("Expected LIKE condition"),
    }
}

#[test]
fn test_parse_ilike_condition() {
    let query = Parser::parse("SELECT * FROM docs WHERE title ILIKE '%Rust%'").unwrap();
    match query.select.where_clause {
        Some(Condition::Like(c)) => {
            assert_eq!(c.column, "title");
            assert_eq!(c.pattern, "%Rust%");
            assert!(c.case_insensitive); // ILIKE is case-insensitive
        }
        _ => panic!("Expected ILIKE condition"),
    }
}

#[test]
fn test_parse_ilike_lowercase() {
    // ILIKE keyword should work regardless of case
    let query = Parser::parse("SELECT * FROM docs WHERE name ilike 'test%'").unwrap();
    match query.select.where_clause {
        Some(Condition::Like(c)) => {
            assert_eq!(c.column, "name");
            assert_eq!(c.pattern, "test%");
            assert!(c.case_insensitive);
        }
        _ => panic!("Expected ILIKE condition"),
    }
}

// ========== IS NULL tests ==========

#[test]
fn test_parse_is_null() {
    let query = Parser::parse("SELECT * FROM docs WHERE deleted_at IS NULL").unwrap();
    match query.select.where_clause {
        Some(Condition::IsNull(c)) => {
            assert_eq!(c.column, "deleted_at");
            assert!(c.is_null);
        }
        _ => panic!("Expected IS NULL condition"),
    }
}

#[test]
fn test_parse_is_not_null() {
    let query = Parser::parse("SELECT * FROM docs WHERE title IS NOT NULL").unwrap();
    match query.select.where_clause {
        Some(Condition::IsNull(c)) => {
            assert_eq!(c.column, "title");
            assert!(!c.is_null);
        }
        _ => panic!("Expected IS NOT NULL condition"),
    }
}

// ========== Error tests ==========

#[test]
fn test_parse_syntax_error() {
    let result = Parser::parse("SELEC * FROM docs");
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_from() {
    let result = Parser::parse("SELECT * docs");
    assert!(result.is_err());
}

// ========== Case insensitivity tests ==========
// VelesQL follows standard SQL conventions: keywords are case-insensitive

#[test]
fn test_parse_case_insensitive_lowercase() {
    let query = Parser::parse("select * from documents where vector near $v limit 10").unwrap();
    assert_eq!(query.select.from, "documents");
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_parse_case_insensitive_uppercase() {
    let query = Parser::parse("SELECT * FROM DOCUMENTS WHERE VECTOR NEAR $V LIMIT 10").unwrap();
    assert_eq!(query.select.from, "DOCUMENTS");
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_parse_case_insensitive_mixed() {
    let query = Parser::parse("Select * From documents Where Vector Near $v Limit 10").unwrap();
    assert_eq!(query.select.from, "documents");
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_parse_case_insensitive_order_by() {
    let query = Parser::parse("select * from docs order by name desc").unwrap();
    assert!(query.select.order_by.is_some());
    assert!(query.select.order_by.unwrap()[0].descending);
}

#[test]
fn test_parse_case_insensitive_where_clauses() {
    // AND, OR, BETWEEN, IN, LIKE, IS NULL
    let q1 = Parser::parse("select * from t where a = 1 and b = 2").unwrap();
    assert!(q1.select.where_clause.is_some());

    let q2 = Parser::parse("select * from t where a = 1 or b = 2").unwrap();
    assert!(q2.select.where_clause.is_some());

    let q3 = Parser::parse("select * from t where x between 1 and 10").unwrap();
    assert!(q3.select.where_clause.is_some());

    let q4 = Parser::parse("select * from t where id in (1, 2, 3)").unwrap();
    assert!(q4.select.where_clause.is_some());

    let q5 = Parser::parse("select * from t where name like '%test%'").unwrap();
    assert!(q5.select.where_clause.is_some());

    let q6 = Parser::parse("select * from t where val is null").unwrap();
    assert!(q6.select.where_clause.is_some());

    let q7 = Parser::parse("select * from t where val is not null").unwrap();
    assert!(q7.select.where_clause.is_some());
}

#[test]
fn test_parse_case_insensitive_join() {
    let query = Parser::parse("select * from a join b on b.id = a.b_id").unwrap();
    assert_eq!(query.select.joins.len(), 1);
    assert_eq!(query.select.joins[0].table, "b");
}

#[test]
fn test_parse_case_insensitive_join_with_alias() {
    let query =
        Parser::parse("select * from products join prices as p on p.product_id = products.id")
            .unwrap();
    assert_eq!(query.select.joins[0].alias, Some("p".to_string()));
}

#[test]
fn test_parse_case_insensitive_with_clause() {
    let query = Parser::parse("select * from docs limit 10 with (mode = 'fast')").unwrap();
    assert!(query.select.with_clause.is_some());
}

#[test]
fn test_parse_case_insensitive_boolean_values() {
    let q1 = Parser::parse("SELECT * FROM t WHERE active = true").unwrap();
    let q2 = Parser::parse("SELECT * FROM t WHERE active = TRUE").unwrap();
    let q3 = Parser::parse("SELECT * FROM t WHERE active = True").unwrap();
    assert!(q1.select.where_clause.is_some());
    assert!(q2.select.where_clause.is_some());
    assert!(q3.select.where_clause.is_some());
}

#[test]
fn test_parse_case_insensitive_null_value() {
    let q1 = Parser::parse("SELECT * FROM t WHERE x = null").unwrap();
    let q2 = Parser::parse("SELECT * FROM t WHERE x = NULL").unwrap();
    let q3 = Parser::parse("SELECT * FROM t WHERE x = Null").unwrap();
    assert!(q1.select.where_clause.is_some());
    assert!(q2.select.where_clause.is_some());
    assert!(q3.select.where_clause.is_some());
}

#[test]
fn test_parse_case_insensitive_similarity() {
    let q1 = Parser::parse("SELECT * FROM t WHERE similarity(vec, $v) > 0.8").unwrap();
    let q2 = Parser::parse("SELECT * FROM t WHERE SIMILARITY(vec, $v) > 0.8").unwrap();
    let q3 = Parser::parse("SELECT * FROM t WHERE Similarity(vec, $v) > 0.8").unwrap();
    assert!(q1.select.where_clause.is_some());
    assert!(q2.select.where_clause.is_some());
    assert!(q3.select.where_clause.is_some());
}

// ========== WITH clause tests ==========

#[test]
fn test_parse_with_clause_single_option() {
    let query =
        Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'accurate')")
            .unwrap();
    let with = query.select.with_clause.expect("Expected WITH clause");
    assert_eq!(with.options.len(), 1);
    assert_eq!(with.options[0].key, "mode");
    assert_eq!(with.get_mode(), Some("accurate"));
}

#[test]
fn test_parse_with_clause_multiple_options() {
    let query = Parser::parse(
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'fast', ef_search = 512, timeout_ms = 5000)"
    ).unwrap();
    let with = query.select.with_clause.expect("Expected WITH clause");
    assert_eq!(with.options.len(), 3);
    assert_eq!(with.get_mode(), Some("fast"));
    assert_eq!(with.get_ef_search(), Some(512));
    assert_eq!(with.get_timeout_ms(), Some(5000));
}

#[test]
fn test_parse_with_clause_boolean_option() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10 WITH (rerank = true)").unwrap();
    let with = query.select.with_clause.expect("Expected WITH clause");
    assert_eq!(with.get_rerank(), Some(true));
}

#[test]
fn test_parse_with_clause_identifier_value() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10 WITH (mode = accurate)").unwrap();
    let with = query.select.with_clause.expect("Expected WITH clause");
    assert_eq!(with.get_mode(), Some("accurate"));
}

#[test]
fn test_parse_without_with_clause() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10").unwrap();
    assert!(query.select.with_clause.is_none());
}

#[test]
fn test_parse_with_clause_float_value() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10 WITH (threshold = 0.95)").unwrap();
    let with = query.select.with_clause.expect("Expected WITH clause");
    let value = with.get("threshold").expect("Expected threshold option");
    assert_eq!(value.as_float(), Some(0.95));
}

// ========== JOIN clause tests (EPIC-031 US-004) ==========

#[test]
fn test_parse_simple_join() {
    let query =
        Parser::parse("SELECT * FROM products JOIN prices ON prices.product_id = products.id")
            .unwrap();
    assert_eq!(query.select.joins.len(), 1);
    let join = &query.select.joins[0];
    assert_eq!(join.table, "prices");
    assert!(join.alias.is_none());
    let cond = join
        .condition
        .as_ref()
        .expect("condition should be present");
    assert_eq!(cond.left.table, Some("prices".to_string()));
    assert_eq!(cond.left.column, "product_id");
    assert_eq!(cond.right.table, Some("products".to_string()));
    assert_eq!(cond.right.column, "id");
}

#[test]
fn test_parse_join_with_alias() {
    let query =
        Parser::parse("SELECT * FROM products JOIN prices AS pr ON pr.product_id = products.id")
            .unwrap();
    assert_eq!(query.select.joins.len(), 1);
    let join = &query.select.joins[0];
    assert_eq!(join.table, "prices");
    assert_eq!(join.alias, Some("pr".to_string()));
    let cond = join
        .condition
        .as_ref()
        .expect("condition should be present");
    assert_eq!(cond.left.table, Some("pr".to_string()));
    assert_eq!(cond.left.column, "product_id");
}

#[test]
fn test_parse_multiple_joins() {
    let query = Parser::parse(
        "SELECT * FROM trips JOIN prices ON prices.trip_id = trips.id JOIN availability ON availability.trip_id = trips.id",
    )
    .unwrap();
    assert_eq!(query.select.joins.len(), 2);
    assert_eq!(query.select.joins[0].table, "prices");
    assert_eq!(query.select.joins[1].table, "availability");
}

#[test]
fn test_parse_join_with_where() {
    // Note: WHERE currently only supports simple identifiers, not table.column
    let query = Parser::parse(
        "SELECT * FROM products JOIN prices ON prices.product_id = products.id WHERE value > 100",
    )
    .unwrap();
    assert_eq!(query.select.joins.len(), 1);
    assert!(query.select.where_clause.is_some());
}

#[test]
fn test_parse_no_join() {
    let query = Parser::parse("SELECT * FROM products WHERE id = 1").unwrap();
    assert!(query.select.joins.is_empty());
}
