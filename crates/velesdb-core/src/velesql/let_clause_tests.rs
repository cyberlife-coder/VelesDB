//! Tests for LET clause parsing (VelesQL v1.10 Phase 3).
//!
//! Validates that `LET name = expr` bindings are parsed correctly and
//! stored in `Query.let_bindings`. Covers nominal cases, edge cases,
//! case-insensitivity, and interaction with existing clauses.

use crate::velesql::{ArithmeticExpr, ArithmeticOp, OrderByExpr, Parser};

// ============================================================================
// A. Single LET binding — nominal
// ============================================================================

/// `LET x = 0.5 SELECT ...` parses to a single literal binding.
#[test]
fn test_parse_single_let_literal() {
    let sql = "LET x = 0.5 SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse LET literal");

    assert_eq!(query.let_bindings.len(), 1);
    assert_eq!(query.let_bindings[0].name, "x");
    assert!(
        matches!(&query.let_bindings[0].expr, ArithmeticExpr::Literal(v) if (*v - 0.5).abs() < 1e-9),
        "expected Literal(0.5), got {:?}",
        query.let_bindings[0].expr
    );
    assert_eq!(query.select.from, "docs");
}

/// `LET x = vector_score SELECT ...` parses to a variable binding.
#[test]
fn test_parse_single_let_variable() {
    let sql = "LET x = vector_score SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse LET variable");

    assert_eq!(query.let_bindings.len(), 1);
    assert_eq!(query.let_bindings[0].name, "x");
    assert_eq!(
        query.let_bindings[0].expr,
        ArithmeticExpr::Variable("vector_score".to_string())
    );
}

/// `LET x = 0.7 * vector_score + 0.3 * bm25_score SELECT ...`
#[test]
fn test_parse_single_let_arithmetic() {
    let sql = "LET hybrid = 0.7 * vector_score + 0.3 * bm25_score SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse LET arithmetic");

    assert_eq!(query.let_bindings.len(), 1);
    assert_eq!(query.let_bindings[0].name, "hybrid");
    // The expression is a BinaryOp at the top level (additive: mul + mul).
    assert!(
        matches!(
            &query.let_bindings[0].expr,
            ArithmeticExpr::BinaryOp {
                op: ArithmeticOp::Add,
                ..
            }
        ),
        "expected top-level Add, got {:?}",
        query.let_bindings[0].expr
    );
}

/// `LET x = similarity() SELECT ...` parses to a similarity binding.
#[test]
fn test_parse_single_let_similarity() {
    let sql = "LET x = similarity() SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse LET similarity");

    assert_eq!(query.let_bindings.len(), 1);
    assert_eq!(query.let_bindings[0].name, "x");
    assert!(
        matches!(
            &query.let_bindings[0].expr,
            ArithmeticExpr::Similarity(inner) if matches!(inner.as_ref(), OrderByExpr::SimilarityBare)
        ),
        "expected Similarity(SimilarityBare), got {:?}",
        query.let_bindings[0].expr
    );
}

// ============================================================================
// B. Multiple LET bindings
// ============================================================================

/// Two LET bindings parsed in order.
#[test]
fn test_parse_multiple_let() {
    let sql = "LET a = 1.0 LET b = a * 2.0 SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse multiple LET");

    assert_eq!(query.let_bindings.len(), 2);
    assert_eq!(query.let_bindings[0].name, "a");
    assert!(matches!(
        &query.let_bindings[0].expr,
        ArithmeticExpr::Literal(v) if (*v - 1.0).abs() < 1e-9
    ));
    assert_eq!(query.let_bindings[1].name, "b");
    assert!(matches!(
        &query.let_bindings[1].expr,
        ArithmeticExpr::BinaryOp {
            op: ArithmeticOp::Mul,
            ..
        }
    ));
}

// ============================================================================
// C. LET with MATCH query
// ============================================================================

/// LET before a MATCH query is accepted.
#[test]
fn test_parse_let_with_match() {
    let sql = "LET x = 0.5 MATCH (a)-[r]->(b) RETURN a LIMIT 5";
    let query = Parser::parse(sql).expect("should parse LET with MATCH");

    assert_eq!(query.let_bindings.len(), 1);
    assert_eq!(query.let_bindings[0].name, "x");
    assert!(query.is_match_query());
}

// ============================================================================
// D. No LET — backward compatibility
// ============================================================================

/// Query without LET has empty `let_bindings`.
#[test]
fn test_parse_no_let() {
    let sql = "SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse without LET");
    assert!(query.let_bindings.is_empty());
}

// ============================================================================
// E. Edge cases
// ============================================================================

/// Binding name is preserved as-is (case-sensitive).
#[test]
fn test_let_name_is_case_sensitive() {
    let sql = "LET MyScore = 0.5 SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse case-sensitive name");
    assert_eq!(query.let_bindings[0].name, "MyScore");
}

/// Nested parentheses in expression.
#[test]
fn test_parse_let_nested_parentheses() {
    let sql = "LET x = (0.7 * (vector_score + 0.1)) SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse nested parens");
    assert_eq!(query.let_bindings.len(), 1);
    assert_eq!(query.let_bindings[0].name, "x");
}

/// `let` keyword is case-insensitive (PEG `^"LET"`).
#[test]
fn test_let_keyword_case_insensitive() {
    let sql = "let x = 0.5 SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse lowercase let");
    assert_eq!(query.let_bindings.len(), 1);
    assert_eq!(query.let_bindings[0].name, "x");
}

/// LET combined with all major clauses.
#[test]
fn test_let_with_all_clauses() {
    let sql = "LET x = similarity() \
               SELECT * FROM docs \
               WHERE vector NEAR $v \
               ORDER BY x DESC \
               LIMIT 10 OFFSET 5 \
               WITH (mode = 'fast')";
    let query = Parser::parse(sql).expect("should parse LET with all clauses");
    assert_eq!(query.let_bindings.len(), 1);
    assert!(query.select.order_by.is_some());
    assert!(query.select.offset.is_some());
    assert!(query.select.with_clause.is_some());
}

// ============================================================================
// F. LET serialization round-trip
// ============================================================================

/// LET bindings survive JSON serialization/deserialization.
#[test]
fn test_let_binding_serde_roundtrip() {
    let sql = "LET hybrid = 0.7 * vector_score + 0.3 * bm25_score SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse");

    let json = serde_json::to_string(&query).expect("should serialize");
    let roundtrip: crate::velesql::Query = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(query, roundtrip);
}

/// Deserializing JSON without `let_bindings` produces empty vec (backward compat).
#[test]
fn test_let_binding_serde_backward_compat() {
    // JSON produced by older VelesQL (no let_bindings field).
    let json = r#"{"select":{"distinct":"None","columns":"All","from":"docs","from_alias":[],"joins":[],"where_clause":null,"order_by":null,"limit":5,"offset":null,"with_clause":null,"group_by":null,"having":null,"fusion_clause":null},"compound":null,"match_clause":null,"dml":null}"#;
    let query: crate::velesql::Query = serde_json::from_str(json).expect("should deserialize");
    assert!(query.let_bindings.is_empty());
}

/// Integer literal in LET expression.
#[test]
fn test_parse_let_integer_literal() {
    let sql = "LET threshold = 1 SELECT * FROM docs LIMIT 5";
    let query = Parser::parse(sql).expect("should parse LET integer");
    assert_eq!(query.let_bindings.len(), 1);
    assert!(matches!(
        &query.let_bindings[0].expr,
        ArithmeticExpr::Literal(v) if (*v - 1.0).abs() < 1e-9
    ));
}
