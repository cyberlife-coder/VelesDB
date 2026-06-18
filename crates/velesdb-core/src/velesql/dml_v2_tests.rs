//! Tests for VelesQL v3.5 Phase 4: multi-row INSERT, UPSERT, WITH (quality=...).
//!
//! Covers:
//! - Multi-row INSERT parsing (2 rows, 3 rows, mixed value types)
//! - UPSERT parsing (single row, multi row, with parameter)
//! - Negative: column count mismatch in multi-row, UPSERT without INTO
//! - `quality` option in WITH clause
//! - AST validation: row count, values per row

use crate::velesql::{DmlStatement, Parser, Value};

// ============================================================================
// A. Multi-row INSERT parsing
// ============================================================================

#[test]
fn test_multi_row_insert_two_rows() {
    let query = "INSERT INTO docs (id, title) VALUES (1, 'Hello'), (2, 'World')";
    let parsed = Parser::parse(query).expect("multi-row INSERT should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Insert(insert) = dml else {
        panic!("Expected Insert variant");
    };

    assert_eq!(insert.table, "docs");
    assert_eq!(insert.columns, vec!["id", "title"]);
    assert_eq!(insert.rows.len(), 2);
    assert_eq!(insert.rows[0][0], Value::Integer(1));
    assert_eq!(insert.rows[0][1], Value::String("Hello".to_string()));
    assert_eq!(insert.rows[1][0], Value::Integer(2));
    assert_eq!(insert.rows[1][1], Value::String("World".to_string()));
}

#[test]
fn test_multi_row_insert_three_rows() {
    let query = "INSERT INTO docs (id, name) VALUES (1, 'A'), (2, 'B'), (3, 'C')";
    let parsed = Parser::parse(query).expect("3-row INSERT should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Insert(insert) = dml else {
        panic!("Expected Insert variant");
    };

    assert_eq!(insert.rows.len(), 3);
    for (i, row) in insert.rows.iter().enumerate() {
        assert_eq!(row.len(), 2, "Row {i} should have 2 values");
    }
    assert_eq!(insert.rows[2][0], Value::Integer(3));
    assert_eq!(insert.rows[2][1], Value::String("C".to_string()));
}

#[test]
fn test_multi_row_insert_mixed_value_types() {
    let query = "INSERT INTO items (id, price, active) VALUES (1, 9.99, true), (2, 4.50, false)";
    let parsed = Parser::parse(query).expect("multi-row with mixed types should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Insert(insert) = dml else {
        panic!("Expected Insert variant");
    };

    assert_eq!(insert.rows.len(), 2);
    assert_eq!(insert.rows[0][0], Value::Integer(1));
    assert_eq!(insert.rows[0][1], Value::Float(9.99));
    assert_eq!(insert.rows[0][2], Value::Boolean(true));
    assert_eq!(insert.rows[1][0], Value::Integer(2));
    assert_eq!(insert.rows[1][1], Value::Float(4.50));
    assert_eq!(insert.rows[1][2], Value::Boolean(false));
}

#[test]
fn test_single_row_insert_still_works() {
    let query = "INSERT INTO docs (id, title) VALUES (1, 'Only')";
    let parsed = Parser::parse(query).expect("single-row INSERT should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Insert(insert) = dml else {
        panic!("Expected Insert variant");
    };

    assert_eq!(insert.rows.len(), 1);
    assert_eq!(insert.rows[0].len(), 2);
    assert_eq!(insert.rows[0][0], Value::Integer(1));
    assert_eq!(insert.rows[0][1], Value::String("Only".to_string()));
}

// ============================================================================
// B. UPSERT parsing
// ============================================================================

#[test]
fn test_upsert_single_row() {
    let query = "UPSERT INTO docs (id, title) VALUES (1, 'Updated')";
    let parsed = Parser::parse(query).expect("UPSERT should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Upsert(upsert) = dml else {
        panic!("Expected Upsert variant, got {dml:?}");
    };

    assert_eq!(upsert.table, "docs");
    assert_eq!(upsert.columns, vec!["id", "title"]);
    assert_eq!(upsert.rows.len(), 1);
    assert_eq!(upsert.rows[0][0], Value::Integer(1));
    assert_eq!(upsert.rows[0][1], Value::String("Updated".to_string()));
}

#[test]
fn test_upsert_multi_row() {
    let query = "UPSERT INTO docs (id, title) VALUES (1, 'First'), (2, 'Second')";
    let parsed = Parser::parse(query).expect("multi-row UPSERT should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Upsert(upsert) = dml else {
        panic!("Expected Upsert variant");
    };

    assert_eq!(upsert.rows.len(), 2);
    assert_eq!(upsert.rows[0][0], Value::Integer(1));
    assert_eq!(upsert.rows[1][0], Value::Integer(2));
}

#[test]
fn test_upsert_with_vector_parameter() {
    let query = "UPSERT INTO docs (id, vector, title) VALUES (1, $v, 'test')";
    let parsed = Parser::parse(query).expect("UPSERT with parameter should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Upsert(upsert) = dml else {
        panic!("Expected Upsert variant");
    };

    assert_eq!(upsert.columns, vec!["id", "vector", "title"]);
    assert_eq!(upsert.rows[0][1], Value::Parameter("v".to_string()));
}

// ============================================================================
// C. Negative tests
// ============================================================================

#[test]
fn test_multi_row_insert_column_count_mismatch_fails() {
    // Second row has 3 values but only 2 columns
    let result = Parser::parse("INSERT INTO docs (id, title) VALUES (1, 'A'), (2, 'B', 'Extra')");
    assert!(
        result.is_err(),
        "Column/value count mismatch in row 2 should fail"
    );
}

#[test]
fn test_upsert_missing_into_keyword_fails() {
    let result = Parser::parse("UPSERT docs (id) VALUES (1)");
    assert!(
        result.is_err(),
        "Missing INTO keyword in UPSERT should fail"
    );
}

#[test]
fn test_upsert_missing_values_keyword_fails() {
    let result = Parser::parse("UPSERT INTO docs (id, name) (1, 'x')");
    assert!(
        result.is_err(),
        "Missing VALUES keyword in UPSERT should fail"
    );
}

// ============================================================================
// D. WITH (quality = '...') — parsed via existing WITH clause
// ============================================================================

#[test]
fn test_quality_option_parses_in_select() {
    let query = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (quality = 'fast')";
    let parsed = Parser::parse(query).expect("WITH (quality=...) should parse");
    let with = parsed
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should exist");

    assert_eq!(with.get_mode(), Some("fast"), "quality should alias mode");
}

#[test]
fn test_quality_option_accurate() {
    let query = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (quality = 'accurate')";
    let parsed = Parser::parse(query).expect("WITH (quality='accurate') should parse");
    let with = parsed
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should exist");

    assert_eq!(
        with.get_mode(),
        Some("accurate"),
        "quality='accurate' should be surfaced via get_mode()"
    );
}

#[test]
fn test_mode_takes_precedence_over_quality() {
    let query = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (quality = 'fast', mode = 'accurate')";
    let parsed = Parser::parse(query).expect("both quality and mode should parse");
    let with = parsed
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should exist");

    // `mode` key takes precedence over `quality` key
    assert_eq!(
        with.get_mode(),
        Some("accurate"),
        "mode should take precedence over quality"
    );
}

// ============================================================================
// E. AST validation helpers
// ============================================================================

#[test]
fn test_multi_row_insert_rows_count_and_shape() {
    let query = "INSERT INTO metrics (id, val) VALUES (1, 100), (2, 200), (3, 300), (4, 400)";
    let parsed = Parser::parse(query).expect("4-row INSERT should parse");
    let dml = parsed.dml.expect("Expected DML");
    let DmlStatement::Insert(insert) = dml else {
        panic!("Expected Insert variant");
    };

    assert_eq!(insert.rows.len(), 4, "Should have 4 rows");
    for (i, row) in insert.rows.iter().enumerate() {
        assert_eq!(
            row.len(),
            insert.columns.len(),
            "Row {i} should match column count"
        );
    }
    assert_eq!(insert.rows[0][0], Value::Integer(1));
    assert_eq!(insert.rows[0][1], Value::Integer(100));
    assert_eq!(insert.rows[3][0], Value::Integer(4));
    assert_eq!(insert.rows[3][1], Value::Integer(400));
}
