#![cfg(all(test, feature = "persistence"))]

use super::having::{DEFAULT_MAX_GROUPS, SERVER_MAX_GROUPS_CEILING};
use super::*;
use crate::velesql::{AggregateArg, AggregateFunction, AggregateType, OrderByExpr, SelectOrderBy};

#[test]
fn test_sort_aggregation_results_order_by_count_desc_sorts_rows() {
    // ARRANGE
    let mut rows = vec![
        serde_json::json!({"category": "science", "count": 2}),
        serde_json::json!({"category": "tech", "count": 5}),
        serde_json::json!({"category": "history", "count": 3}),
    ];
    let order_by = vec![SelectOrderBy {
        expr: OrderByExpr::Aggregate(AggregateFunction {
            function_type: AggregateType::Count,
            argument: AggregateArg::Wildcard,
            alias: None,
        }),
        descending: true,
    }];

    // ACT
    Collection::sort_aggregation_results(&mut rows, &order_by);

    // ASSERT
    let ordered_categories: Vec<&str> = rows
        .iter()
        .map(|row| {
            row.get("category")
                .and_then(serde_json::Value::as_str)
                .expect("category should be a string")
        })
        .collect();
    assert_eq!(ordered_categories, vec!["tech", "history", "science"]);
}

// -------------------------------------------------------------------------
// #903: GROUP BY group ceiling is a server-side hard cap.
// -------------------------------------------------------------------------

/// A query CANNOT raise `max_groups` above the server-side ceiling: an absurdly
/// large `WITH (max_groups=...)` is clamped down, not honored.
#[test]
fn test_max_groups_clamped_to_server_ceiling() {
    use crate::velesql::{WithClause, WithValue};

    // Query tries to demand far more groups than the server permits.
    let with = WithClause::new().with_option("max_groups", WithValue::Integer(i64::MAX));
    let resolved = Collection::extract_max_groups_limit(Some(&with));

    assert_eq!(
        resolved, SERVER_MAX_GROUPS_CEILING,
        "untrusted query must not raise the group ceiling above the server cap"
    );
}

/// A query may LOWER its own group budget below the ceiling.
#[test]
fn test_max_groups_below_ceiling_is_honored() {
    use crate::velesql::{WithClause, WithValue};

    let with = WithClause::new().with_option("group_limit", WithValue::Integer(42));
    let resolved = Collection::extract_max_groups_limit(Some(&with));

    assert_eq!(resolved, 42);
}

/// Absent a WITH clause, the conservative default applies (well below ceiling).
#[test]
fn test_max_groups_default_when_unspecified() {
    let resolved = Collection::extract_max_groups_limit(None);
    assert_eq!(resolved, DEFAULT_MAX_GROUPS);
    assert!(resolved < SERVER_MAX_GROUPS_CEILING);
}
