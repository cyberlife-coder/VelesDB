//! Window function evaluation engine (Issue #386).
//!
//! Evaluates window functions (`ROW_NUMBER`, `RANK`, `DENSE_RANK`) over
//! a result set, partitioning and sorting as specified by the `OVER` clause.
//!
//! ## Pipeline position
//!
//! Window evaluation happens **after** DISTINCT and **before** ORDER BY/LIMIT.
//! This is an intentional deviation from SQL standard (which runs windows
//! before DISTINCT) tailored to the vector-search use case; see the design
//! note on [`crate::collection::search::query::select_dispatch::QueryExecutor::apply_select_postprocessing`]
//! for the full rationale.
//!
//! ## Design: Global snapshot-first evaluation
//!
//! Rankings are computed from snapshots of ORDER BY values AND PARTITION BY
//! keys taken **up-front for every window function**, before any single
//! injection runs. This prevents three classes of corruption:
//!
//! 1. **Intra-function**: If the window alias matches an ORDER BY column
//!    (e.g., `RANK() OVER (ORDER BY score DESC) AS score`), injecting the
//!    rank mid-loop would overwrite the original sort value. Fixed by the
//!    per-function three-phase (snapshot → compute → inject) path.
//!
//! 2. **Inter-function on ORDER BY**: When multiple window functions are
//!    evaluated sequentially and an earlier function's alias collides with
//!    a later function's ORDER BY column (e.g.
//!    `ROW_NUMBER() ... AS score, RANK() OVER (ORDER BY score DESC) AS rnk`),
//!    the later function would read the injected ranks instead of the
//!    original payload. Fixed by snapshotting ORDER BY values for **every**
//!    window function before any injection.
//!
//! 3. **Inter-function on PARTITION BY**: Same contamination path but via
//!    a partition key rather than a sort key. Fixed by snapshotting partition
//!    keys alongside ORDER BY values, up-front.

use crate::velesql::{WindowFunction, WindowFunctionType, WindowOrderBy};
use crate::SearchResult;
use std::collections::BTreeMap;

/// Pre-computed per-row snapshot of the inputs a single window function
/// reads from the result set.
///
/// Captured in [`evaluate`] **before any window function injects values**
/// so sequential injection cannot contaminate later functions' inputs.
struct WindowSnapshot {
    /// For each row (parallel index to `results`), the value of each
    /// ORDER BY column in the `OVER` clause, in declaration order.
    order_by_values: Vec<Vec<serde_json::Value>>,
    /// For each row, the pre-computed PARTITION BY key string.
    partition_keys: Vec<String>,
}

impl WindowSnapshot {
    fn capture(results: &[SearchResult], wf: &WindowFunction) -> Self {
        let order_by_values = results
            .iter()
            .map(|r| {
                wf.over_clause
                    .order_by
                    .iter()
                    .map(|ob| extract_sort_value(r, &ob.column))
                    .collect()
            })
            .collect();
        let partition_keys = results
            .iter()
            .map(|r| partition_key(r, &wf.over_clause.partition_by))
            .collect();
        Self {
            order_by_values,
            partition_keys,
        }
    }
}

/// Evaluates window functions on a mutable result set.
///
/// Pre-snapshots every window function's ORDER BY values and PARTITION BY
/// keys **before** any injection, then computes rankings and injects
/// results one function at a time. This prevents both intra-function
/// corruption (alias collides with its own ORDER BY column) and
/// inter-function contamination (an earlier function's alias collides
/// with a later function's ORDER BY or PARTITION BY column).
///
/// Returns `Ok(())` unconditionally; the `Result` signature is kept for
/// pipeline consistency with the execution engine.
///
/// # Errors
///
/// Currently infallible — the signature keeps `Result` so future window
/// functions that can fail (e.g. numeric overflow in `ROW_NUMBER` past
/// `u64::MAX`, or a user-provided ORDER BY reference that cannot be
/// resolved) can propagate errors without rippling through every caller.
pub fn evaluate(
    results: &mut [SearchResult],
    window_functions: &[WindowFunction],
) -> crate::Result<()> {
    // Phase 0: snapshot EVERY window function's inputs before any injection.
    // This is the fix for inter-function contamination — once any window
    // function injects values into payloads, later snapshots would read the
    // injected values instead of the originals.
    let snapshots: Vec<WindowSnapshot> = window_functions
        .iter()
        .map(|wf| WindowSnapshot::capture(results, wf))
        .collect();

    for (wf, snapshot) in window_functions.iter().zip(snapshots.iter()) {
        apply_single_window(results, wf, snapshot);
    }
    Ok(())
}

/// Applies a single window function across all partitions, using a
/// pre-captured snapshot of its inputs.
///
/// Uses a three-phase approach to prevent payload corruption:
/// 1. **Snapshot** captured up-front by [`evaluate`] (read-only from here).
/// 2. **Compute** rankings from the snapshot.
/// 3. **Inject** ranking values into payloads (write-only).
fn apply_single_window(
    results: &mut [SearchResult],
    wf: &WindowFunction,
    snapshot: &WindowSnapshot,
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

    // Phase 2: Build partitions from the pre-snapshotted partition keys
    // and compute rankings (read-only from snapshots).
    let partitions = build_partitions_from_snapshot(&snapshot.partition_keys);
    let mut all_rankings: Vec<(usize, u64)> = Vec::new();

    for indices in partitions.values() {
        let sorted = sort_partition_from_snapshots(
            indices,
            &snapshot.order_by_values,
            &wf.over_clause.order_by,
        );
        let rankings = compute_rankings(
            &sorted,
            &snapshot.order_by_values,
            &wf.over_clause.order_by,
            wf.function_type,
        );
        all_rankings.extend(rankings);
    }

    // Phase 3: Inject all rankings (write-only, no more reads).
    for (idx, value) in all_rankings {
        inject_ranking(&mut results[idx], alias, value);
    }
}

/// Groups row indices by their pre-computed `PARTITION BY` key.
///
/// Uses `BTreeMap` for deterministic partition ordering in tests.
/// An empty `PARTITION BY` clause maps every row to the same key (`""`),
/// producing a single partition that covers the whole result set.
fn build_partitions_from_snapshot(partition_keys: &[String]) -> BTreeMap<String, Vec<usize>> {
    let mut partitions: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, key) in partition_keys.iter().enumerate() {
        partitions.entry(key.clone()).or_default().push(i);
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

/// Sorts indices within a partition using pre-snapshotted sort values.
///
/// This avoids reading from payloads during sorting, preventing any
/// corruption from prior injections.
fn sort_partition_from_snapshots(
    indices: &[usize],
    snapshots: &[Vec<serde_json::Value>],
    order_by: &[WindowOrderBy],
) -> Vec<usize> {
    let mut sorted = indices.to_vec();
    sorted.sort_by(|&a, &b| {
        for (col_idx, ob) in order_by.iter().enumerate() {
            let cmp = compare_json_values(&snapshots[a][col_idx], &snapshots[b][col_idx]);
            let cmp = if ob.descending { cmp.reverse() } else { cmp };
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    });
    sorted
}

/// Computes ranking values from pre-snapshotted sort values.
///
/// Dispatches to a variant-specific helper so each helper only maintains
/// the state its ranking function actually uses (no dead `rank` / `dense_rank`
/// initialisation when the other variant is selected).
///
/// Returns a list of `(result_index, ranking_value)` pairs.
fn compute_rankings(
    sorted_indices: &[usize],
    snapshots: &[Vec<serde_json::Value>],
    order_by: &[WindowOrderBy],
    fn_type: WindowFunctionType,
) -> Vec<(usize, u64)> {
    match fn_type {
        WindowFunctionType::RowNumber => compute_row_numbers(sorted_indices),
        WindowFunctionType::Rank => compute_rank(sorted_indices, snapshots, order_by),
        WindowFunctionType::DenseRank => compute_dense_rank(sorted_indices, snapshots, order_by),
    }
}

/// `ROW_NUMBER()`: assign `1..=n` by sort position, ignoring ties.
fn compute_row_numbers(sorted_indices: &[usize]) -> Vec<(usize, u64)> {
    sorted_indices
        .iter()
        .enumerate()
        .map(|(position, &idx)| (idx, (position as u64) + 1))
        .collect()
}

/// `RANK()`: gaps after ties (1, 2, 2, 4).
///
/// When the current row's snapshot ties with the previous row's, the rank
/// is carried forward; otherwise it jumps to `position + 1`.
fn compute_rank(
    sorted_indices: &[usize],
    snapshots: &[Vec<serde_json::Value>],
    order_by: &[WindowOrderBy],
) -> Vec<(usize, u64)> {
    let mut rankings = Vec::with_capacity(sorted_indices.len());
    let mut rank: u64 = 1;
    for (position, &idx) in sorted_indices.iter().enumerate() {
        if is_new_group(position, sorted_indices, snapshots, order_by, idx) {
            rank = (position as u64) + 1;
        }
        rankings.push((idx, rank));
    }
    rankings
}

/// `DENSE_RANK()`: no gaps after ties (1, 2, 2, 3).
fn compute_dense_rank(
    sorted_indices: &[usize],
    snapshots: &[Vec<serde_json::Value>],
    order_by: &[WindowOrderBy],
) -> Vec<(usize, u64)> {
    let mut rankings = Vec::with_capacity(sorted_indices.len());
    let mut dense_rank: u64 = 1;
    for (position, &idx) in sorted_indices.iter().enumerate() {
        if is_new_group(position, sorted_indices, snapshots, order_by, idx) {
            dense_rank += 1;
        }
        rankings.push((idx, dense_rank));
    }
    rankings
}

/// Returns `true` if the row at `position` starts a new tie group, i.e.
/// its ORDER BY snapshot differs from the previous row's. The first row
/// (`position == 0`) is never "new" — it anchors the initial group.
fn is_new_group(
    position: usize,
    sorted_indices: &[usize],
    snapshots: &[Vec<serde_json::Value>],
    order_by: &[WindowOrderBy],
    idx: usize,
) -> bool {
    if position == 0 {
        return false;
    }
    let prev_idx = sorted_indices[position - 1];
    !snapshots_tied(&snapshots[idx], &snapshots[prev_idx], order_by)
}

/// Returns `true` if two snapshot rows have identical ORDER BY values.
fn snapshots_tied(
    snap_a: &[serde_json::Value],
    snap_b: &[serde_json::Value],
    order_by: &[WindowOrderBy],
) -> bool {
    for (col_idx, _ob) in order_by.iter().enumerate() {
        if compare_json_values(&snap_a[col_idx], &snap_b[col_idx]) != std::cmp::Ordering::Equal {
            return false;
        }
    }
    true
}

/// Extracts a sortable value from a result for comparison.
///
/// Special-cases `"similarity"` to use the search score, consistent with
/// the main ORDER BY system in `ordering.rs`.
fn extract_sort_value(result: &SearchResult, column: &str) -> serde_json::Value {
    if column == "similarity" {
        return serde_json::Value::from(f64::from(result.score));
    }
    extract_nested_value(result, column)
}

/// Extracts a potentially nested payload value (e.g., `"metadata.source"`).
fn extract_nested_value(result: &SearchResult, column: &str) -> serde_json::Value {
    let Some(payload) = result.point.payload.as_ref() else {
        return serde_json::Value::Null;
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

/// Extracts a canonical-JSON string representation of a payload value for
/// partition keys.
///
/// Uses `serde_json`'s `Display` (`value.to_string()`) rather than extracting
/// the inner string for `Value::String`. This preserves the JSON type
/// discriminator (quotes around strings, bare digits for numbers, `null`
/// literal for NULL) so rows with different types cannot collide into the
/// same partition.
///
/// Examples of collisions that this prevents:
/// - `Value::Null` vs `Value::String("null")` → `null` vs `"null"` (distinct)
/// - `Value::Number(1)` vs `Value::String("1")` → `1` vs `"1"` (distinct)
/// - `Value::Bool(true)` vs `Value::String("true")` → `true` vs `"true"`
///
/// Consistent with `distinct::canonical_json_string` which applies the same
/// rule for DISTINCT deduplication keys.
fn extract_payload_value(result: &SearchResult, column: &str) -> String {
    extract_nested_value(result, column).to_string()
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
