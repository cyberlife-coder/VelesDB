//! Set operation execution for compound queries (UNION, INTERSECT, EXCEPT).
//!
//! Implements SQL-standard set semantics on `SearchResult` vectors, keyed by
//! point ID. Each operator follows the scoring rules documented below.

use std::collections::HashMap;

use crate::point::SearchResult;
use crate::velesql::SetOperator;

/// Applies a set operator to two result sets, bounding the output at `limit`.
///
/// Scoring rules per operator:
/// - **Union**: deduplicate by point ID, keep highest score.
/// - **`UnionAll`**: concatenate without deduplication.
/// - **Intersect**: keep only IDs present in both; take the higher score.
/// - **Except**: keep left-side IDs that do not appear in the right side.
///
/// Results are returned sorted by score descending and truncated to `limit`.
/// Because the final result is score-ranked then capped, only the top `limit`
/// rows are ever observable — so truncating here drops nothing within the
/// requested window. Operands are expected to already be capped by the caller
/// (`MAX_LIMIT`), which bounds buffering on the smaller-side scan for INTERSECT.
pub(crate) fn apply_set_operation(
    left: Vec<SearchResult>,
    right: Vec<SearchResult>,
    operator: SetOperator,
    limit: usize,
) -> Vec<SearchResult> {
    let mut results = match operator {
        SetOperator::Union => union_dedup(left, right),
        SetOperator::UnionAll => union_all(left, right),
        SetOperator::Intersect => intersect(left, &right),
        SetOperator::Except => except(left, &right),
    };

    results.sort_unstable_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results.truncate(limit);
    results
}

/// UNION: merge both sides, deduplicate by point ID (keep highest score).
fn union_dedup(left: Vec<SearchResult>, right: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut map: HashMap<u64, SearchResult> = HashMap::with_capacity(left.len() + right.len());

    for result in left {
        map.insert(result.point.id, result);
    }

    for result in right {
        match map.entry(result.point.id) {
            std::collections::hash_map::Entry::Occupied(mut existing) => {
                if result.score > existing.get().score {
                    existing.insert(result);
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(result);
            }
        }
    }

    map.into_values().collect()
}

/// UNION ALL: concatenate without deduplication.
fn union_all(mut left: Vec<SearchResult>, right: Vec<SearchResult>) -> Vec<SearchResult> {
    left.extend(right);
    left
}

/// INTERSECT: keep only IDs present in both sides; take the higher score.
fn intersect(left: Vec<SearchResult>, right: &[SearchResult]) -> Vec<SearchResult> {
    let right_map: HashMap<u64, &SearchResult> = right.iter().map(|r| (r.point.id, r)).collect();

    left.into_iter()
        .filter_map(|l| {
            right_map
                .get(&l.point.id)
                .map(|r| if r.score > l.score { (*r).clone() } else { l })
        })
        .collect()
}

/// EXCEPT: keep left-side results whose IDs do not appear in the right side.
fn except(left: Vec<SearchResult>, right: &[SearchResult]) -> Vec<SearchResult> {
    let right_ids: std::collections::HashSet<u64> = right.iter().map(|r| r.point.id).collect();

    left.into_iter()
        .filter(|l| !right_ids.contains(&l.point.id))
        .collect()
}

#[cfg(test)]
#[path = "set_operations_tests.rs"]
mod tests;
