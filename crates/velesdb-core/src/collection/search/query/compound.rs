//! Compound query execution (UNION/INTERSECT/EXCEPT).
//!
//! Applies set operations to combine results from two SELECT statements.
//! Full implementation in Plan 08-03.

use crate::point::SearchResult;
use crate::velesql::SetOperator;
use std::collections::HashSet;

/// Applies a set operation to two result sets.
///
/// # Set Operations
///
/// - `Union`: merge + deduplicate by `point.id`
/// - `UnionAll`: concatenate (no dedup)
/// - `Intersect`: keep only IDs present in both
/// - `Except`: remove second set IDs from first
#[must_use]
pub fn apply_set_operation(
    left: Vec<SearchResult>,
    right: Vec<SearchResult>,
    operator: SetOperator,
) -> Vec<SearchResult> {
    match operator {
        SetOperator::Union => {
            let mut seen = HashSet::new();
            let mut results = Vec::with_capacity(left.len() + right.len());
            for r in left.into_iter().chain(right) {
                if seen.insert(r.point.id) {
                    results.push(r);
                }
            }
            results
        }
        SetOperator::UnionAll => {
            let mut results = left;
            results.extend(right);
            results
        }
        SetOperator::Intersect => {
            let right_ids: HashSet<u64> = right.iter().map(|r| r.point.id).collect();
            left.into_iter()
                .filter(|r| right_ids.contains(&r.point.id))
                .collect()
        }
        SetOperator::Except => {
            let right_ids: HashSet<u64> = right.iter().map(|r| r.point.id).collect();
            left.into_iter()
                .filter(|r| !right_ids.contains(&r.point.id))
                .collect()
        }
    }
}
