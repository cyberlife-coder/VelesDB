//! Tests for `Database` private helpers.
//!
//! Covers regressions on [`Database::strip_table_prefix_from_condition`] —
//! the JOIN filter rewriter that strips `table.` prefixes from
//! column references before `Filter::matches` evaluates them against
//! unqualified payload keys.

#![cfg(all(test, feature = "persistence"))]

use crate::velesql::{
    CompareOp, Comparison, Condition, ContainsTextCondition, IsNullCondition, LikeCondition,
    MatchCondition, Value,
};
use crate::Database;

/// Smoke: `Comparison` leaf has prefix stripped.
#[test]
fn test_strip_prefix_on_comparison() {
    let cond = Condition::Comparison(Comparison {
        column: "articles.category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });

    let stripped = Database::strip_table_prefix_from_condition(cond);
    match stripped {
        Condition::Comparison(c) => assert_eq!(c.column, "category"),
        other => panic!("expected Comparison, got {other:?}"),
    }
}

/// Regression (Devin review on PR #630): `ContainsText` leaf must also
/// have its prefix stripped, otherwise JOIN queries with
/// `articles.description CONTAINS_TEXT 'keyword'` silently match zero
/// points because `Filter::matches` looks for the qualified key
/// `articles.description` instead of `description`.
#[test]
fn test_strip_prefix_on_contains_text_regression() {
    let cond = Condition::ContainsText(ContainsTextCondition {
        column: "articles.description".to_string(),
        query: "keyword".to_string(),
    });

    let stripped = Database::strip_table_prefix_from_condition(cond);
    match stripped {
        Condition::ContainsText(c) => assert_eq!(c.column, "description"),
        other => panic!("expected ContainsText, got {other:?}"),
    }
}

/// Prefix stripping also applies inside `And` / `Or` / `Not` / `Group`
/// composite nodes and must reach nested leaves including `ContainsText`.
#[test]
fn test_strip_prefix_recurses_into_composite() {
    let cond = Condition::And(
        Box::new(Condition::ContainsText(ContainsTextCondition {
            column: "articles.description".to_string(),
            query: "rust".to_string(),
        })),
        Box::new(Condition::Like(LikeCondition {
            column: "articles.title".to_string(),
            pattern: "%Rust%".to_string(),
            case_insensitive: false,
        })),
    );

    let stripped = Database::strip_table_prefix_from_condition(cond);
    match stripped {
        Condition::And(l, r) => {
            match *l {
                Condition::ContainsText(c) => assert_eq!(c.column, "description"),
                other => panic!("expected left ContainsText, got {other:?}"),
            }
            match *r {
                Condition::Like(c) => assert_eq!(c.column, "title"),
                other => panic!("expected right Like, got {other:?}"),
            }
        }
        other => panic!("expected And, got {other:?}"),
    }
}

/// Leaves with no prefix pass through unchanged.
#[test]
fn test_strip_prefix_is_noop_without_table_qualifier() {
    let cond = Condition::IsNull(IsNullCondition {
        column: "name".to_string(),
        is_null: true,
    });

    let stripped = Database::strip_table_prefix_from_condition(cond);
    match stripped {
        Condition::IsNull(c) => assert_eq!(c.column, "name"),
        other => panic!("expected IsNull, got {other:?}"),
    }
}

/// `Match` leaf has prefix stripped (representative of the other
/// leaf variants already covered by the original implementation).
#[test]
fn test_strip_prefix_on_match() {
    let cond = Condition::Match(MatchCondition {
        column: "articles.body".to_string(),
        query: "search".to_string(),
    });

    let stripped = Database::strip_table_prefix_from_condition(cond);
    match stripped {
        Condition::Match(c) => assert_eq!(c.column, "body"),
        other => panic!("expected Match, got {other:?}"),
    }
}
