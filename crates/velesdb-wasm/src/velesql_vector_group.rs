//! ORDER BY for a vector GROUP BY (`NEAR` + GROUP BY) in the WASM executor.
//!
//! Core's engine-shared `Grouper` already made each group what core's vector
//! GROUP BY returns: its best row's id, the group score, and the group row,
//! best group first. Core then sorts those search results with its search
//! ORDER BY once each ORDER BY aggregate has become the field the group
//! stores it in; this module does the same with the WASM search ORDER BY
//! (`velesql_orderby::sort_rows`), so `similarity()`, arithmetic, fields and
//! aggregates order the groups as they do in core, and each row keeps the
//! group's id and score.

use velesdb_core::point::{Point, SearchResult};
use velesdb_core::velesql::{GroupOutput, SelectStatement};

use crate::velesql_result::QueryResultRow;

/// Orders finished vector GROUP BY groups by the statement's ORDER BY, under
/// the collection metric's direction (`higher_is_better`).
///
/// # Errors
///
/// Returns core's `V016` message for an ORDER BY aggregate the SELECT does
/// not list, and the WASM search ORDER BY's error for a form it cannot
/// evaluate.
pub(crate) fn order_groups(
    stmt: &SelectStatement,
    groups: Vec<GroupOutput>,
    higher_is_better: bool,
) -> Result<Vec<QueryResultRow>, String> {
    let stmt = stmt
        .order_by_aggregates_as_fields()
        .map_err(|e| e.to_string())?;
    let mut results: Vec<SearchResult> = groups
        .into_iter()
        .map(|group| {
            SearchResult::new(
                Point::new(group.id, Vec::new(), Some(group.row)),
                group.score,
            )
        })
        .collect();
    crate::velesql_orderby::sort_rows(&stmt, &mut results, higher_is_better)?;
    results
        .iter()
        .map(|result| {
            let row = result
                .point
                .payload
                .as_ref()
                .unwrap_or(&serde_json::Value::Null);
            QueryResultRow::from_projected(result.point.id, result.score, row)
        })
        .collect()
}
