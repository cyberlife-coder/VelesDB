//! ORDER BY for a vector GROUP BY (`NEAR` + GROUP BY) in the WASM executor.
//!
//! Core returns a vector GROUP BY as search results, one per group: scored by
//! its best row (or by the average, see
//! [`vector_group_score_is_average`]), identified by that best row, carrying
//! the group row as payload, and sorted by the plain search ORDER BY once each
//! ORDER BY aggregate has become the field the group stores it in. This module
//! builds the same results from the WASM group rows and sorts them with the
//! WASM search ORDER BY (`velesql_orderby::sort_rows`), so `similarity()`,
//! arithmetic, fields and aggregates order the groups as they do in core.

use velesdb_core::point::{Point, SearchResult};
use velesdb_core::velesql::{vector_group_score_is_average, AggregateFunction, SelectStatement};

use crate::velesql_aggregate::ScannedRow;

/// Orders finished vector GROUP BY rows, each beside the scanned rows of its
/// group, by the statement's ORDER BY.
///
/// # Errors
///
/// Returns core's `V016` message for an ORDER BY aggregate the SELECT does
/// not list, and the WASM search ORDER BY's error for a form it cannot
/// evaluate.
pub(crate) fn order_groups(
    stmt: &SelectStatement,
    aggregates: &[AggregateFunction],
    groups: Vec<(serde_json::Value, Vec<ScannedRow<'_>>)>,
) -> Result<Vec<serde_json::Value>, String> {
    let average = vector_group_score_is_average(aggregates);
    let stmt = stmt
        .order_by_aggregates_as_fields()
        .map_err(|e| e.to_string())?;
    let mut results: Vec<SearchResult> = groups
        .into_iter()
        .map(|(row, rows)| {
            let (id, score) = group_score(&rows, average);
            SearchResult::new(Point::new(id, Vec::new(), Some(row)), score)
        })
        .collect();
    crate::velesql_orderby::sort_rows(&stmt, &mut results)?;
    Ok(results
        .into_iter()
        .map(|result| result.point.payload.unwrap_or(serde_json::Value::Null))
        .collect())
}

/// The id of a group's best row (the first, on a tie) and the group's score:
/// that row's, or the mean over the group when `average`. A group holds at
/// least the row that created it.
fn group_score(rows: &[ScannedRow<'_>], average: bool) -> (u64, f32) {
    let mut best = (0, f32::NEG_INFINITY);
    let mut sum = 0.0_f32;
    for &(id, score, _) in rows {
        sum += score;
        if score > best.1 {
            best = (id, score);
        }
    }
    if average {
        // Reason: a group's row count is bounded by the scanned result set.
        #[allow(clippy::cast_precision_loss)]
        let count = rows.len() as f32;
        (best.0, sum / count)
    } else {
        best
    }
}
