//! Vector-search GROUP BY post-processing (Issue #511).
//!
//! Groups the results of a vector search by a parent field: one result per
//! group, carrying its best row's id and vector, the group score, and the
//! group row (GROUP BY columns and SELECT aggregates) as payload. The grouping
//! itself — group set, aggregates, HAVING, default order — is the
//! engine-shared [`crate::velesql::Grouper`], which the WASM executor uses
//! too; this module only maps search results in and out of it.

use crate::point::{Point, SearchResult};
use crate::velesql::{GroupInput, Grouper, HavingClause, SelectStatement};
use std::collections::HashMap;

/// Groups `results` for `stmt`, a vector GROUP BY whose HAVING is `having`
/// with its parameters bound, best group first under the metric's
/// direction (`higher_is_better`).
///
/// # Errors
///
/// None in practice: the group count is uncapped (`usize::MAX`), the only
/// error `Grouper::push` reports.
pub(crate) fn group_search_results(
    stmt: &SelectStatement,
    results: &[SearchResult],
    having: Option<&HavingClause>,
    higher_is_better: bool,
) -> crate::error::Result<Vec<SearchResult>> {
    let mut grouper = Grouper::new(stmt, having, higher_is_better, usize::MAX);
    for result in results {
        let row = GroupInput {
            id: result.point.id,
            score: result.score,
            payload: result.point.payload.as_ref(),
        };
        grouper.push(row).map_err(crate::error::Error::Config)?;
    }
    let by_id: HashMap<u64, &SearchResult> = results.iter().map(|r| (r.point.id, r)).collect();
    Ok(grouper
        .finish()
        .into_iter()
        .map(|group| {
            let mut point = by_id.get(&group.id).map_or_else(
                || Point::new(group.id, Vec::new(), None),
                |r| r.point.clone(),
            );
            point.payload = Some(group.row);
            SearchResult {
                point,
                score: group.score,
                component_scores: None,
            }
        })
        .collect())
}

#[cfg(test)]
#[path = "vector_group_by_tests.rs"]
mod tests;
