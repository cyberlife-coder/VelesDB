//! ORDER BY expansion for the WASM VelesQL executor (S4-13).
//!
//! Supports ordering on any payload column (not just `id` / `score`),
//! multi-key sort, ASC/DESC per key, and explicit null handling:
//! nulls sort last in ASC and first in DESC (matches Postgres default).
//!
//! The row set passed in is a list of `(sort_keys, final_row)` pairs built
//! by the caller (`velesql_select`), so this module is pure sorting and
//! does not touch the row-building logic.

use std::cmp::Ordering;

use velesdb_core::velesql::{OrderByExpr, SelectOrderBy, SelectStatement};

use crate::velesql_result::QueryResultRow;
use crate::velesql_value::json_values_cmp;

/// A row bundled with the JSON values used to compute its ORDER BY keys.
///
/// The outer code builds one `SortableRow` per result row; this module only
/// sorts them according to the ORDER BY spec.
pub(crate) struct SortableRow {
    /// Point id — used for `ORDER BY id`.
    pub id: u64,
    /// Similarity / relevance score — used for `ORDER BY similarity()` /
    /// `ORDER BY score`.
    pub score: f32,
    /// Payload — used for arbitrary column sorts.
    pub payload: Option<serde_json::Value>,
    /// The serialized row to return to the caller after sorting.
    pub row: QueryResultRow,
}

/// Sorts a row set in place according to the SELECT's ORDER BY clause.
///
/// Does nothing when the statement has no ORDER BY. Invalid expressions
/// (e.g. aggregate / arithmetic / similarity with explicit args) fall back
/// to stable no-op sort so the executor never fails silently on shape.
pub(crate) fn sort_rows(stmt: &SelectStatement, rows: &mut [SortableRow]) {
    let Some(specs) = stmt.order_by.as_ref() else {
        return;
    };
    if specs.is_empty() {
        return;
    }
    rows.sort_by(|a, b| compare_rows(a, b, specs));
}

/// Strict total order used by `sort_by` over a multi-key ORDER BY.
fn compare_rows(a: &SortableRow, b: &SortableRow, specs: &[SelectOrderBy]) -> Ordering {
    for spec in specs {
        let ord = compare_with_spec(a, b, spec);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn compare_with_spec(a: &SortableRow, b: &SortableRow, spec: &SelectOrderBy) -> Ordering {
    let ord = match &spec.expr {
        OrderByExpr::Field(name) => compare_field(a, b, name),
        OrderByExpr::SimilarityBare => compare_scores(a, b),
        OrderByExpr::Similarity(_) | OrderByExpr::Aggregate(_) | OrderByExpr::Arithmetic(_) => {
            // These expressions aren't materialized in WASM — treat them as
            // equal so the comparison degrades to a stable no-op rather than
            // producing a misleading order.
            Ordering::Equal
        }
        _ => Ordering::Equal,
    };
    if spec.descending {
        ord.reverse()
    } else {
        ord
    }
}

/// Compares a pair of rows by the given column. `id` and `score` are
/// resolved from their dedicated struct fields.
fn compare_field(a: &SortableRow, b: &SortableRow, name: &str) -> Ordering {
    if name == "id" {
        return a.id.cmp(&b.id);
    }
    if name == "score" {
        return compare_scores(a, b);
    }
    let va = extract_payload_field(a.payload.as_ref(), name);
    let vb = extract_payload_field(b.payload.as_ref(), name);
    compare_json_with_nulls(va.as_ref(), vb.as_ref())
}

/// Compares two f32 scores (NaN-safe, NaN sorts last in ASC).
fn compare_scores(a: &SortableRow, b: &SortableRow) -> Ordering {
    match (a.score.is_nan(), b.score.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater, // NaN last
        (false, true) => Ordering::Less,
        (false, false) => a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal),
    }
}

/// Pulls a column from the payload (supports dot-nested paths via filter).
fn extract_payload_field(
    payload: Option<&serde_json::Value>,
    column: &str,
) -> Option<serde_json::Value> {
    payload.and_then(|p| crate::filter::get_nested_field(p, column).cloned())
}

/// Postgres-style NULL ordering: null sorts AFTER non-null in ASC. Since we
/// always compute ASC first and then reverse for DESC via the outer
/// `compare_with_spec`, producing "null > non-null" here gives the correct
/// "NULLS LAST in ASC" / "NULLS FIRST in DESC" behaviour.
fn compare_json_with_nulls(
    left: Option<&serde_json::Value>,
    right: Option<&serde_json::Value>,
) -> Ordering {
    let left_null = left.is_none_or(serde_json::Value::is_null);
    let right_null = right.is_none_or(serde_json::Value::is_null);
    match (left_null, right_null) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            // Safe to unwrap: both are Some and non-null per the match arms.
            let l = left.expect("left is non-null here");
            let r = right.expect("right is non-null here");
            json_values_cmp(l, r).unwrap_or(Ordering::Equal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::velesql::{OrderByExpr, SelectOrderBy};

    fn mk(id: u64, score: f32, payload: serde_json::Value) -> SortableRow {
        SortableRow {
            id,
            score,
            payload: Some(payload),
            row: QueryResultRow::synthetic(serde_json::json!({"id": id})).expect("test: row"),
        }
    }

    fn by_field(name: &str, desc: bool) -> Vec<SelectOrderBy> {
        vec![SelectOrderBy {
            expr: OrderByExpr::Field(name.to_string()),
            descending: desc,
        }]
    }

    #[test]
    fn test_sort_by_id_asc() {
        let mut rows = vec![
            mk(3, 0.0, serde_json::json!({})),
            mk(1, 0.0, serde_json::json!({})),
            mk(2, 0.0, serde_json::json!({})),
        ];
        let mut stmt = velesdb_core::velesql::SelectStatement::empty();
        stmt.order_by = Some(by_field("id", false));
        sort_rows(&stmt, &mut rows);
        assert_eq!(rows[0].id, 1);
        assert_eq!(rows[2].id, 3);
    }

    #[test]
    fn test_sort_by_id_desc() {
        let mut rows = vec![
            mk(3, 0.0, serde_json::json!({})),
            mk(1, 0.0, serde_json::json!({})),
            mk(2, 0.0, serde_json::json!({})),
        ];
        let mut stmt = velesdb_core::velesql::SelectStatement::empty();
        stmt.order_by = Some(by_field("id", true));
        sort_rows(&stmt, &mut rows);
        assert_eq!(rows[0].id, 3);
        assert_eq!(rows[2].id, 1);
    }

    #[test]
    fn test_sort_by_payload_column() {
        let mut rows = vec![
            mk(1, 0.0, serde_json::json!({"price": 20})),
            mk(2, 0.0, serde_json::json!({"price": 10})),
            mk(3, 0.0, serde_json::json!({"price": 30})),
        ];
        let mut stmt = velesdb_core::velesql::SelectStatement::empty();
        stmt.order_by = Some(by_field("price", false));
        sort_rows(&stmt, &mut rows);
        assert_eq!(rows[0].id, 2);
        assert_eq!(rows[1].id, 1);
        assert_eq!(rows[2].id, 3);
    }

    #[test]
    fn test_sort_nulls_last_asc() {
        let mut rows = vec![
            mk(1, 0.0, serde_json::json!({"x": 5})),
            mk(2, 0.0, serde_json::json!({})),
            mk(3, 0.0, serde_json::json!({"x": 1})),
        ];
        let mut stmt = velesdb_core::velesql::SelectStatement::empty();
        stmt.order_by = Some(by_field("x", false));
        sort_rows(&stmt, &mut rows);
        assert_eq!(rows[0].id, 3); // x=1
        assert_eq!(rows[1].id, 1); // x=5
        assert_eq!(rows[2].id, 2); // null last
    }

    #[test]
    fn test_sort_nulls_first_desc() {
        let mut rows = vec![
            mk(1, 0.0, serde_json::json!({"x": 5})),
            mk(2, 0.0, serde_json::json!({})),
            mk(3, 0.0, serde_json::json!({"x": 1})),
        ];
        let mut stmt = velesdb_core::velesql::SelectStatement::empty();
        stmt.order_by = Some(by_field("x", true));
        sort_rows(&stmt, &mut rows);
        assert_eq!(rows[0].id, 2); // null first in DESC
    }

    #[test]
    fn test_sort_multi_key() {
        let mut rows = vec![
            mk(1, 0.0, serde_json::json!({"cat": "a", "p": 10})),
            mk(2, 0.0, serde_json::json!({"cat": "a", "p": 5})),
            mk(3, 0.0, serde_json::json!({"cat": "b", "p": 1})),
        ];
        let mut stmt = velesdb_core::velesql::SelectStatement::empty();
        stmt.order_by = Some(vec![
            SelectOrderBy {
                expr: OrderByExpr::Field("cat".to_string()),
                descending: false,
            },
            SelectOrderBy {
                expr: OrderByExpr::Field("p".to_string()),
                descending: true,
            },
        ]);
        sort_rows(&stmt, &mut rows);
        // cat=a first, then p DESC: (a, 10) < (a, 5) < (b, 1)
        assert_eq!(rows[0].id, 1);
        assert_eq!(rows[1].id, 2);
        assert_eq!(rows[2].id, 3);
    }

    #[test]
    fn test_sort_by_similarity_bare_desc() {
        let mut rows = vec![
            mk(1, 0.1, serde_json::json!({})),
            mk(2, 0.9, serde_json::json!({})),
            mk(3, 0.5, serde_json::json!({})),
        ];
        let mut stmt = velesdb_core::velesql::SelectStatement::empty();
        stmt.order_by = Some(vec![SelectOrderBy {
            expr: OrderByExpr::SimilarityBare,
            descending: true,
        }]);
        sort_rows(&stmt, &mut rows);
        assert_eq!(rows[0].id, 2);
        assert_eq!(rows[2].id, 1);
    }
}
