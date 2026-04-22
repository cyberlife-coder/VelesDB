//! Window function evaluation engine (Issue #386).
//!
//! Evaluates window functions (`ROW_NUMBER`, `RANK`, `DENSE_RANK`) over
//! a result set, partitioning and sorting as specified by the `OVER` clause.
//!
//! Window evaluation happens **after** DISTINCT but **before** ORDER BY/LIMIT
//! in the query pipeline, matching SQL standard semantics.

use crate::velesql::{OverClause, WindowFunction, WindowFunctionType, WindowOrderBy};
use crate::SearchResult;
use std::collections::BTreeMap;

/// Evaluates window functions on a mutable result set.
///
/// For each window function, partitions the results, sorts within each
/// partition, computes the ranking value, and injects it into the
/// `SearchResult`'s payload as a new JSON field.
///
/// Returns `Ok(())` unconditionally; the `Result` signature is kept for
/// pipeline consistency with the execution engine.
pub fn evaluate(
    results: &mut [SearchResult],
    window_functions: &[WindowFunction],
) -> crate::Result<()> {
    for wf in window_functions {
        apply_single_window(results, wf);
    }
    Ok(())
}

/// Applies a single window function across all partitions.
fn apply_single_window(
    results: &mut [SearchResult],
    wf: &WindowFunction,
) {
    let alias = wf
        .alias
        .as_deref()
        .unwrap_or(wf.function_type.default_alias());

    // Warn when ORDER BY is empty — ranking is non-deterministic without it.
    if wf.over_clause.order_by.is_empty() {
        tracing::warn!(
            function = alias,
            "Window function OVER clause has no ORDER BY; ranking order is non-deterministic"
        );
    }

    // Step 1: Build partition groups (row indices grouped by partition key).
    let partitions = build_partitions(results, &wf.over_clause);

    // Step 2: For each partition, sort indices and assign rankings.
    for indices in partitions.values() {
        let sorted = sort_partition(results, indices, &wf.over_clause.order_by);
        assign_rankings(results, &sorted, &wf.over_clause.order_by, wf.function_type, alias);
    }
}

/// Groups result indices by their `PARTITION BY` key values.
///
/// Uses `BTreeMap` for deterministic partition ordering in tests.
/// Empty `partition_by` → single partition containing all indices.
fn build_partitions(
    results: &[SearchResult],
    over_clause: &OverClause,
) -> BTreeMap<String, Vec<usize>> {
    let mut partitions: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, result) in results.iter().enumerate() {
        let key = partition_key(result, &over_clause.partition_by);
        partitions.entry(key).or_default().push(i);
    }
    partitions
}

/// Computes a partition key from the result's payload fields.
///
/// Empty `columns` → single partition key (`""`).
/// Multiple columns are joined with `\x1F` (ASCII Unit Separator) to avoid
/// collisions when values contain common delimiters.
fn partition_key(result: &SearchResult, columns: &[String]) -> String {
    if columns.is_empty() {
        return String::new();
    }
    columns
        .iter()
        .map(|col| extract_payload_value(result, col))
        .collect::<Vec<_>>()
        .join("\x1F")
}

/// Sorts indices within a partition according to the window `ORDER BY` spec.
fn sort_partition(
    results: &[SearchResult],
    indices: &[usize],
    order_by: &[WindowOrderBy],
) -> Vec<usize> {
    let mut sorted = indices.to_vec();
    sorted.sort_by(|&a, &b| compare_rows(results, a, b, order_by));
    sorted
}

/// Compares two result rows by the window `ORDER BY` columns.
fn compare_rows(
    results: &[SearchResult],
    a: usize,
    b: usize,
    order_by: &[WindowOrderBy],
) -> std::cmp::Ordering {
    for ob in order_by {
        let val_a = extract_sort_value(&results[a], &ob.column);
        let val_b = extract_sort_value(&results[b], &ob.column);
        let cmp = compare_json_values(&val_a, &val_b);
        let cmp = if ob.descending { cmp.reverse() } else { cmp };
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
    }
    std::cmp::Ordering::Equal
}

/// Extracts a sortable value from a result for comparison.
///
/// Special-cases `"similarity"` and `"score"` to use the search score.
fn extract_sort_value(result: &SearchResult, column: &str) -> serde_json::Value {
    if column == "similarity" || column == "score" {
        return serde_json::Value::from(f64::from(result.score));
    }
    extract_nested_value(result, column)
}

/// Extracts a potentially nested payload value (e.g., `"metadata.source"`).
fn extract_nested_value(result: &SearchResult, column: &str) -> serde_json::Value {
    let payload = match result.point.payload.as_ref() {
        Some(p) => p,
        None => return serde_json::Value::Null,
    };

    if column.contains('.') {
        // Navigate nested path: "metadata.source" → payload["metadata"]["source"]
        let parts: Vec<&str> = column.split('.').collect();
        let mut current = payload;
        for part in &parts {
            match current.get(*part) {
                Some(v) => current = v,
                None => return serde_json::Value::Null,
            }
        }
        current.clone()
    } else {
        payload
            .get(column)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
}

/// Extracts a string representation of a payload value for partition keys.
fn extract_payload_value(result: &SearchResult, column: &str) -> String {
    let value = extract_nested_value(result, column);
    match value {
        serde_json::Value::Null => "__null__".to_string(),
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }
}

/// Compares two JSON values for sorting.
///
/// Numeric values are compared numerically; everything else is compared
/// as strings. `Null` sorts last (after all non-null values).
fn compare_json_values(a: &serde_json::Value, b: &serde_json::Value) -> std::cmp::Ordering {
    match (a, b) {
        (serde_json::Value::Null, serde_json::Value::Null) => std::cmp::Ordering::Equal,
        (serde_json::Value::Null, _) => std::cmp::Ordering::Greater, // NULLs sort last
        (_, serde_json::Value::Null) => std::cmp::Ordering::Less,
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(fa), Some(fb)) => fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal),
            _ => a.to_string().cmp(&b.to_string()),
        },
    }
}

/// Assigns ranking values to each result in the sorted partition.
///
/// - `ROW_NUMBER`: sequential 1..N, no ties.
/// - `RANK`: same rank for ties, gaps after ties (1, 2, 2, 4).
/// - `DENSE_RANK`: same rank for ties, no gaps (1, 2, 2, 3).
fn assign_rankings(
    results: &mut [SearchResult],
    sorted_indices: &[usize],
    order_by: &[WindowOrderBy],
    fn_type: WindowFunctionType,
    alias: &str,
) {
    let mut rank: u64 = 1;
    let mut dense_rank: u64 = 1;

    for (position, &idx) in sorted_indices.iter().enumerate() {
        let is_new_group = is_new_ranking_group(results, sorted_indices, position, order_by);

        let value = match fn_type {
            WindowFunctionType::RowNumber => (position + 1) as u64,
            WindowFunctionType::Rank => {
                if is_new_group {
                    rank = (position + 1) as u64;
                }
                rank
            }
            WindowFunctionType::DenseRank => {
                if is_new_group {
                    dense_rank += 1;
                }
                dense_rank
            }
        };

        inject_ranking(&mut results[idx], alias, value);
    }
}

/// Returns `true` if this position starts a new ranking group (not tied with predecessor).
///
/// The first position (0) is never a "new group" — it starts at rank 1 by default.
fn is_new_ranking_group(
    results: &[SearchResult],
    sorted_indices: &[usize],
    position: usize,
    order_by: &[WindowOrderBy],
) -> bool {
    if position == 0 {
        return false;
    }
    let idx = sorted_indices[position];
    let prev_idx = sorted_indices[position - 1];
    !rows_tied(results, idx, prev_idx, order_by)
}

/// Returns `true` if two rows have identical values for all ORDER BY columns.
fn rows_tied(
    results: &[SearchResult],
    a: usize,
    b: usize,
    order_by: &[WindowOrderBy],
) -> bool {
    for ob in order_by {
        let val_a = extract_sort_value(&results[a], &ob.column);
        let val_b = extract_sort_value(&results[b], &ob.column);
        if compare_json_values(&val_a, &val_b) != std::cmp::Ordering::Equal {
            return false;
        }
    }
    true
}

/// Injects a ranking value into the result's payload as a new JSON field.
fn inject_ranking(result: &mut SearchResult, alias: &str, value: u64) {
    let payload = result
        .point
        .payload
        .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(map) = payload {
        map.insert(alias.to_string(), serde_json::json!(value));
    }
}
