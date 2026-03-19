//! Tests for similarity() in SELECT, qualified wildcards, and ORDER BY similarity() bare.
//!
//! Phase 1 tests for the SQL projection feature.

use super::*;

// ========== similarity() in SELECT ==========

#[test]
fn test_parse_similarity_select_with_alias() {
    let query =
        Parser::parse("SELECT similarity() AS score FROM t WHERE vector NEAR $v LIMIT 10").unwrap();
    match &query.select.columns {
        SelectColumns::SimilarityScore(expr) => {
            assert_eq!(expr.alias.as_deref(), Some("score"));
        }
        other => panic!("Expected SimilarityScore, got {other:?}"),
    }
}

#[test]
fn test_parse_similarity_select_no_alias() {
    let query = Parser::parse("SELECT similarity() FROM t WHERE vector NEAR $v LIMIT 10").unwrap();
    match &query.select.columns {
        SelectColumns::SimilarityScore(expr) => {
            assert!(expr.alias.is_none());
        }
        other => panic!("Expected SimilarityScore, got {other:?}"),
    }
}

#[test]
fn test_parse_similarity_mixed_with_columns() {
    let query = Parser::parse(
        "SELECT title, similarity() AS relevance FROM t WHERE vector NEAR $v LIMIT 10",
    )
    .unwrap();
    match &query.select.columns {
        SelectColumns::Mixed {
            columns,
            similarity_scores,
            ..
        } => {
            assert_eq!(columns.len(), 1);
            assert_eq!(columns[0].name, "title");
            assert_eq!(similarity_scores.len(), 1);
            assert_eq!(similarity_scores[0].alias.as_deref(), Some("relevance"));
        }
        other => panic!("Expected Mixed with similarity_scores, got {other:?}"),
    }
}

// ========== Qualified wildcard ==========

#[test]
fn test_parse_qualified_wildcard() {
    let query = Parser::parse("SELECT ctx.* FROM t AS ctx WHERE vector NEAR $v LIMIT 10").unwrap();
    match &query.select.columns {
        SelectColumns::QualifiedWildcard(alias) => {
            assert_eq!(alias, "ctx");
        }
        other => panic!("Expected QualifiedWildcard, got {other:?}"),
    }
}

#[test]
fn test_parse_qualified_wildcard_mixed() {
    let query = Parser::parse(
        "SELECT d.*, similarity() AS score FROM docs AS d WHERE vector NEAR $v LIMIT 10",
    )
    .unwrap();
    match &query.select.columns {
        SelectColumns::Mixed {
            qualified_wildcards,
            similarity_scores,
            ..
        } => {
            assert_eq!(qualified_wildcards, &["d"]);
            assert_eq!(similarity_scores.len(), 1);
        }
        other => panic!("Expected Mixed with qualified_wildcards, got {other:?}"),
    }
}

// ========== ORDER BY similarity() zero-arg ==========

#[test]
fn test_parse_order_by_similarity_bare() {
    let query =
        Parser::parse("SELECT * FROM t WHERE vector NEAR $v ORDER BY similarity() LIMIT 10")
            .unwrap();
    let order_by = query
        .select
        .order_by
        .as_ref()
        .expect("ORDER BY should exist");
    assert_eq!(order_by.len(), 1);
    assert_eq!(order_by[0].expr, OrderByExpr::SimilarityBare);
    assert!(order_by[0].descending); // default DESC for similarity
}

#[test]
fn test_parse_order_by_similarity_bare_asc() {
    let query =
        Parser::parse("SELECT * FROM t WHERE vector NEAR $v ORDER BY similarity() ASC LIMIT 10")
            .unwrap();
    let order_by = query.select.order_by.as_ref().unwrap();
    assert!(!order_by[0].descending);
    assert_eq!(order_by[0].expr, OrderByExpr::SimilarityBare);
}

#[test]
fn test_parse_order_by_similarity_bare_with_field() {
    let query = Parser::parse(
        "SELECT * FROM t WHERE vector NEAR $v ORDER BY similarity() DESC, created_at ASC LIMIT 10",
    )
    .unwrap();
    let order_by = query.select.order_by.as_ref().unwrap();
    assert_eq!(order_by.len(), 2);
    assert_eq!(order_by[0].expr, OrderByExpr::SimilarityBare);
    assert!(order_by[0].descending);
    match &order_by[1].expr {
        OrderByExpr::Field(name) => assert_eq!(name, "created_at"),
        other => panic!("Expected Field, got {other:?}"),
    }
}

// ========== Validation: similarity() without context ==========

#[test]
fn test_reject_similarity_select_without_score_context() {
    // No NEAR or similarity() in WHERE — should fail validation
    let result = Parser::parse("SELECT similarity() FROM t WHERE name = 'x'");
    // The parser itself should succeed but validation should reject it
    assert!(result.is_ok(), "Parse should succeed: {result:?}");

    let query = result.unwrap();
    let validation = QueryValidator::validate(&query);
    assert!(
        validation.is_err(),
        "Should reject similarity() without score context"
    );
    let err = validation.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::SimilarityWithoutContext);
}

#[test]
fn test_reject_order_by_similarity_bare_without_context() {
    let result = Parser::parse("SELECT * FROM t WHERE name = 'x' ORDER BY similarity()");
    assert!(result.is_ok());
    let query = result.unwrap();
    let validation = QueryValidator::validate(&query);
    assert!(validation.is_err());
    assert_eq!(
        validation.unwrap_err().kind,
        ValidationErrorKind::SimilarityWithoutContext
    );
}

#[test]
fn test_accept_similarity_select_with_near() {
    let result = Parser::parse("SELECT similarity() AS score FROM t WHERE vector NEAR $v LIMIT 10");
    assert!(result.is_ok());
    let query = result.unwrap();
    let validation = QueryValidator::validate(&query);
    assert!(
        validation.is_ok(),
        "Should accept similarity() with NEAR: {validation:?}"
    );
}

#[test]
fn test_accept_similarity_select_with_similarity_where() {
    let result = Parser::parse(
        "SELECT similarity() AS score FROM t WHERE similarity(embedding, $v) > 0.8 LIMIT 10",
    );
    assert!(result.is_ok());
    let query = result.unwrap();
    let validation = QueryValidator::validate(&query);
    assert!(
        validation.is_ok(),
        "Should accept similarity() with similarity() in WHERE: {validation:?}"
    );
}

// ========== Validation: undeclared alias ==========

#[test]
fn test_reject_qualified_wildcard_undeclared_alias() {
    let result = Parser::parse("SELECT memory.* FROM t WHERE vector NEAR $v LIMIT 10");
    assert!(result.is_ok());
    let query = result.unwrap();
    let validation = QueryValidator::validate(&query);
    assert!(
        validation.is_err(),
        "Should reject undeclared alias 'memory'"
    );
    assert_eq!(
        validation.unwrap_err().kind,
        ValidationErrorKind::UndeclaredAlias
    );
}

#[test]
fn test_accept_qualified_wildcard_declared_alias() {
    let result = Parser::parse("SELECT ctx.* FROM t AS ctx WHERE vector NEAR $v LIMIT 10");
    assert!(result.is_ok());
    let query = result.unwrap();
    let validation = QueryValidator::validate(&query);
    assert!(
        validation.is_ok(),
        "Should accept declared alias: {validation:?}"
    );
}

#[test]
fn test_accept_qualified_wildcard_matches_table_name() {
    let result = Parser::parse("SELECT docs.* FROM docs WHERE vector NEAR $v LIMIT 10");
    assert!(result.is_ok());
    let query = result.unwrap();
    let validation = QueryValidator::validate(&query);
    assert!(
        validation.is_ok(),
        "Should accept table name as qualified wildcard: {validation:?}"
    );
}

// ========== Regression: existing queries unchanged ==========

#[test]
fn test_regression_select_all() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 5").unwrap();
    assert_eq!(query.select.columns, SelectColumns::All);
}

#[test]
fn test_regression_select_columns() {
    let query = Parser::parse("SELECT name, age FROM users LIMIT 10").unwrap();
    match &query.select.columns {
        SelectColumns::Columns(cols) => {
            assert_eq!(cols.len(), 2);
            assert_eq!(cols[0].name, "name");
            assert_eq!(cols[1].name, "age");
        }
        other => panic!("Expected Columns, got {other:?}"),
    }
}

#[test]
fn test_regression_order_by_similarity_two_arg() {
    let query =
        Parser::parse("SELECT * FROM docs ORDER BY similarity(embedding, $v) DESC LIMIT 10")
            .unwrap();
    let order_by = query.select.order_by.as_ref().unwrap();
    assert!(matches!(&order_by[0].expr, OrderByExpr::Similarity(_)));
}
