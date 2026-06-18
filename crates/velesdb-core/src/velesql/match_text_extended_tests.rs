//! Extended parser tests for the MATCH text search operator (BM25).
//!
//! Covers nominal, edge-case, and negative scenarios for `column MATCH 'query'`
//! syntax, including combinations with vector NEAR, AND/OR/NOT, ORDER BY,
//! LIMIT/OFFSET, and WITH clauses.

use crate::velesql::{Condition, OrderByExpr, Parser};

// =============================================================================
// Nominal: basic MATCH parsing
// =============================================================================

#[test]
fn test_match_basic() {
    let query = Parser::parse("SELECT * FROM docs WHERE content MATCH 'database' LIMIT 10")
        .expect("basic MATCH should parse");

    assert!(
        query.select.where_clause.is_some(),
        "WHERE clause must be present"
    );
    match query.select.where_clause.as_ref() {
        Some(Condition::Match(m)) => {
            assert_eq!(m.column, "content");
            assert_eq!(m.query, "database");
        }
        other => panic!("Expected Match condition, got {other:?}"),
    }
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_match_with_vector_near_hybrid() {
    let sql = "SELECT * FROM docs WHERE vector NEAR $v AND content MATCH 'vector' LIMIT 10";
    let query = Parser::parse(sql).expect("hybrid MATCH + NEAR should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::And(left, right)) => {
            assert!(
                matches!(left.as_ref(), Condition::VectorSearch(_)),
                "Left should be VectorSearch, got {left:?}"
            );
            assert!(
                matches!(right.as_ref(), Condition::Match(_)),
                "Right should be Match, got {right:?}"
            );
        }
        other => panic!("Expected AND(VectorSearch, Match), got {other:?}"),
    }
}

#[test]
fn test_match_with_multiple_and_conditions() {
    let sql = "SELECT * FROM docs WHERE vector NEAR $v AND title MATCH 'AI' AND category = 'tech' LIMIT 10";
    let query = Parser::parse(sql).expect("MATCH with multiple ANDs should parse");

    assert_eq!(query.select.limit, Some(10));

    // The tree is AND(AND(VectorSearch, Match), Comparison) — left-to-right fold.
    match query.select.where_clause.as_ref() {
        Some(Condition::And(left, right)) => {
            // left is itself AND(VectorSearch, Match) per the left-to-right fold.
            match left.as_ref() {
                Condition::And(l, r) => {
                    assert!(
                        matches!(l.as_ref(), Condition::VectorSearch(_)),
                        "inner-left should be VectorSearch, got {l:?}"
                    );
                    assert!(
                        matches!(r.as_ref(), Condition::Match(m) if m.column == "title" && m.query == "AI"),
                        "inner-right should be Match(title,'AI'), got {r:?}"
                    );
                }
                other => panic!("Expected nested And(VectorSearch, Match), got {other:?}"),
            }
            assert!(
                matches!(right.as_ref(), Condition::Comparison(c) if c.column == "category"),
                "right should be Comparison on category, got {right:?}"
            );
        }
        other => panic!("Expected And(And(VectorSearch, Match), Comparison), got {other:?}"),
    }
}

#[test]
fn test_match_on_different_column() {
    let query = Parser::parse("SELECT * FROM docs WHERE title MATCH 'neural network' LIMIT 5")
        .expect("MATCH on title column should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::Match(m)) => {
            assert_eq!(m.column, "title");
            assert_eq!(m.query, "neural network");
        }
        other => panic!("Expected Match condition, got {other:?}"),
    }
    assert_eq!(query.select.limit, Some(5));
}

#[test]
fn test_match_with_order_by_similarity() {
    let sql = "SELECT * FROM docs WHERE content MATCH 'search' ORDER BY similarity() DESC LIMIT 10";
    let query = Parser::parse(sql).expect("MATCH + ORDER BY similarity() should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::Match(m)) => {
            assert_eq!(m.column, "content");
            assert_eq!(m.query, "search");
        }
        other => panic!("Expected Match condition, got {other:?}"),
    }

    let order_by = query
        .select
        .order_by
        .as_ref()
        .expect("ORDER BY clause must be present");
    assert_eq!(order_by.len(), 1);
    assert_eq!(
        order_by[0].expr,
        OrderByExpr::SimilarityBare,
        "similarity() must parse to SimilarityBare"
    );
    assert!(order_by[0].descending, "DESC must set descending=true");
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_match_with_limit_and_offset() {
    let sql = "SELECT * FROM docs WHERE content MATCH 'test' LIMIT 10 OFFSET 20";
    let query = Parser::parse(sql).expect("MATCH + LIMIT + OFFSET should parse");

    assert!(query.select.where_clause.is_some());
    assert_eq!(query.select.limit, Some(10));
    assert_eq!(query.select.offset, Some(20));
}

#[test]
fn test_match_with_with_clause() {
    let sql = "SELECT * FROM docs WHERE content MATCH 'query' LIMIT 5 WITH (mode = 'accurate')";
    let query = Parser::parse(sql).expect("MATCH + WITH clause should parse");

    assert!(query.select.where_clause.is_some());
    assert_eq!(query.select.limit, Some(5));

    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");
    assert!(
        !with_clause.options.is_empty(),
        "WITH clause must have options"
    );
    assert_eq!(
        with_clause.get_mode(),
        Some("accurate"),
        "mode should be 'accurate'"
    );
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_match_single_character_query() {
    let query = Parser::parse("SELECT * FROM docs WHERE content MATCH 'a' LIMIT 10")
        .expect("MATCH with single-char query should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::Match(m)) => {
            assert_eq!(m.query, "a");
        }
        other => panic!("Expected Match condition, got {other:?}"),
    }
}

#[test]
fn test_match_special_characters() {
    let query = Parser::parse("SELECT * FROM docs WHERE content MATCH 'C++' LIMIT 10")
        .expect("MATCH with special chars should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::Match(m)) => {
            assert_eq!(m.query, "C++");
        }
        other => panic!("Expected Match condition, got {other:?}"),
    }
}

#[test]
fn test_match_combined_with_or() {
    let sql = "SELECT * FROM docs WHERE content MATCH 'database' OR category = 'tech' LIMIT 10";
    let query = Parser::parse(sql).expect("MATCH + OR should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::Or(left, right)) => {
            assert!(
                matches!(left.as_ref(), Condition::Match(_)),
                "Left should be Match, got {left:?}"
            );
            assert!(
                matches!(right.as_ref(), Condition::Comparison(_)),
                "Right should be Comparison, got {right:?}"
            );
        }
        other => panic!("Expected OR(Match, Comparison), got {other:?}"),
    }
}

#[test]
fn test_match_combined_with_not() {
    let sql = "SELECT * FROM docs WHERE content MATCH 'AI' AND NOT (category = 'spam') LIMIT 10";
    let query = Parser::parse(sql).expect("MATCH + AND NOT should parse");

    match query.select.where_clause.as_ref() {
        Some(Condition::And(left, right)) => {
            assert!(
                matches!(left.as_ref(), Condition::Match(_)),
                "Left should be Match, got {left:?}"
            );
            assert!(
                matches!(right.as_ref(), Condition::Not(_)),
                "Right should be Not, got {right:?}"
            );
        }
        other => panic!("Expected AND(Match, Not), got {other:?}"),
    }
}

// =============================================================================
// Negative: parser must reject invalid MATCH syntax
// =============================================================================

#[test]
fn test_match_without_string_value_fails() {
    // MATCH expects a string literal, not an integer.
    let result = Parser::parse("SELECT * FROM docs WHERE content MATCH 123 LIMIT 10");
    assert!(
        result.is_err(),
        "MATCH with integer value should fail to parse"
    );
}

#[test]
fn test_match_without_column_fails() {
    // MATCH requires a leading column name.
    let result = Parser::parse("SELECT * FROM docs WHERE MATCH 'test' LIMIT 10");
    assert!(
        result.is_err(),
        "MATCH without column name should fail to parse"
    );
}
