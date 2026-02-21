//! JOIN execution for cross-store queries (EPIC-031 US-005).
//!
//! This module implements JOIN execution between graph traversal results
//! and ColumnStore data with adaptive batch sizing.

#![allow(dead_code)]

use crate::column_store::ColumnStore;
use crate::error::{Error, Result};
use crate::point::{Point, SearchResult};
use crate::velesql::{ColumnRef, JoinClause, JoinCondition, JoinType};
use std::collections::{HashMap, HashSet};

/// Result of a JOIN operation, combining graph result with column data.
#[derive(Debug, Clone)]
pub struct JoinedResult {
    /// Original search result from graph/vector search.
    pub search_result: SearchResult,
    /// Joined column data from ColumnStore as JSON values.
    pub column_data: HashMap<String, serde_json::Value>,
}

impl JoinedResult {
    /// Creates a new JoinedResult by merging search result with column data.
    #[must_use]
    pub fn new(
        search_result: SearchResult,
        column_data: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            search_result,
            column_data,
        }
    }
}

/// Adaptive batch size thresholds.
const SMALL_BATCH_THRESHOLD: usize = 100;
const MEDIUM_BATCH_THRESHOLD: usize = 10_000;
const MEDIUM_BATCH_SIZE: usize = 1_000;
const LARGE_BATCH_SIZE: usize = 5_000;

/// Determines the optimal batch size based on the number of join keys.
#[must_use]
pub fn adaptive_batch_size(key_count: usize) -> usize {
    match key_count {
        0..=SMALL_BATCH_THRESHOLD => key_count.max(1),
        n if n <= MEDIUM_BATCH_THRESHOLD => MEDIUM_BATCH_SIZE,
        _ => LARGE_BATCH_SIZE,
    }
}

/// Extracts join keys from search results based on the join condition.
///
/// The join key is extracted from the search result's payload using
/// the right side of the join condition (e.g., `products.id`).
///
/// # Note
/// Point IDs > i64::MAX are filtered out to prevent overflow issues.
#[must_use]
pub fn extract_join_keys(results: &[SearchResult], condition: &JoinCondition) -> Vec<(usize, i64)> {
    let key_column = &condition.right.column;

    results
        .iter()
        .enumerate()
        .filter_map(|(idx, r)| {
            // Try to extract the join key from payload
            r.point
                .payload
                .as_ref()
                .and_then(|payload| {
                    payload.get(key_column).and_then(|v| {
                        // Support both integer and point ID
                        v.as_i64().or_else(|| {
                            // Fallback: use point.id if key_column is "id"
                            // Use try_from to safely convert u64 -> i64 without overflow
                            if key_column == "id" {
                                i64::try_from(r.point.id).ok()
                            } else {
                                None
                            }
                        })
                    })
                })
                .or_else(|| {
                    // If no payload, use point.id for "id" column
                    // Use try_from to safely convert u64 -> i64 without overflow
                    if key_column == "id" {
                        i64::try_from(r.point.id).ok()
                    } else {
                        None
                    }
                })
                .map(|key| (idx, key))
        })
        .collect()
}

/// Executes a JOIN between search results and a ColumnStore.
///
/// # Algorithm
///
/// 1. Validate that join condition's left column matches ColumnStore's primary key
/// 2. Extract join keys from search results
/// 3. Determine adaptive batch size
/// 4. Batch lookup in ColumnStore by primary key
/// 5. Merge matching rows with search results
///
/// # Arguments
///
/// * `results` - Search results from vector/graph query
/// * `join` - JOIN clause from parsed query
/// * `column_store` - ColumnStore to join with
///
/// # Returns
///
/// Vector of JoinedResults containing merged data.
/// Returns empty vector if the join condition's left column doesn't match the primary key.
///
/// # Errors
///
/// Returns an error when:
/// - the JOIN type is not supported at runtime,
/// - the JOIN condition is missing or invalid,
/// - the target `ColumnStore` has no primary key,
/// - the JOIN column does not match the target primary key.
#[allow(clippy::cognitive_complexity)] // Reason: Linear flow with early returns, splitting would reduce readability
pub fn execute_join(
    results: &[SearchResult],
    join: &JoinClause,
    column_store: &ColumnStore,
) -> Result<Vec<JoinedResult>> {
    let Some(condition) = resolve_join_condition(join) else {
        return Err(Error::Query(format!(
            "JOIN on table '{}' must use ON condition or USING(single_column).",
            join.table
        )));
    };

    // 1. Validate that join column matches ColumnStore's primary key
    // This prevents silent incorrect results when joining on non-PK columns
    let join_column = &condition.left.column;
    if let Some(pk_column) = column_store.primary_key_column() {
        if join_column != pk_column {
            return Err(Error::Query(format!(
                "JOIN on table '{}' requires primary key '{}', got '{}'.",
                join.table, pk_column, join_column
            )));
        }
    } else {
        return Err(Error::Query(format!(
            "JOIN target '{}' has no primary key configured.",
            join.table
        )));
    }

    // 2. Extract join keys from search results
    let join_keys = extract_join_keys(results, &condition);

    if join_keys.is_empty() {
        return Ok(Vec::new());
    }

    // 3. Determine adaptive batch size
    let batch_size = adaptive_batch_size(join_keys.len());

    // 4. Build result map: pk -> row_data and merge based on join semantics
    let mut joined_results = Vec::with_capacity(join_keys.len());
    let mut matched_left_indices = vec![false; results.len()];
    let mut matched_right_pks: HashSet<i64> = HashSet::with_capacity(join_keys.len());
    let null_row_data = build_null_row_data(column_store);

    // Process in batches
    for chunk in join_keys.chunks(batch_size) {
        // Extract just the keys for this batch
        let pks: Vec<i64> = chunk.iter().map(|(_, pk)| *pk).collect();

        // Batch lookup in ColumnStore
        let rows = batch_get_rows(column_store, &pks);

        // Build map of pk -> column data for this batch
        let row_map = rows;

        // Merge with search results
        for (result_idx, pk) in chunk {
            if let Some(column_data) = row_map.get(pk) {
                let search_result = results[*result_idx].clone();
                joined_results.push(JoinedResult::new(search_result, column_data.clone()));
                matched_left_indices[*result_idx] = true;
                matched_right_pks.insert(*pk);
            } else if matches!(join.join_type, JoinType::Left | JoinType::Full) {
                let search_result = results[*result_idx].clone();
                joined_results.push(JoinedResult::new(search_result, null_row_data.clone()));
                matched_left_indices[*result_idx] = true;
            }
        }
    }

    if matches!(join.join_type, JoinType::Right | JoinType::Full) {
        for row_idx in column_store.live_row_indices() {
            let Some(pk_value) = column_store.get_value_as_json(join_column, row_idx) else {
                continue;
            };
            let Some(pk) = pk_value.as_i64() else {
                continue;
            };
            if matched_right_pks.contains(&pk) {
                continue;
            }

            let Ok(point_id) = u64::try_from(pk) else {
                continue;
            };
            let row_data = row_as_json_map(column_store, row_idx);
            let synthetic_result =
                SearchResult::new(Point::metadata_only(point_id, serde_json::json!({})), 0.0);
            joined_results.push(JoinedResult::new(synthetic_result, row_data));
        }
    }

    // LEFT/FULL should include left rows that had no join key extraction at all.
    if matches!(join.join_type, JoinType::Left | JoinType::Full) {
        for (idx, left_result) in results.iter().enumerate() {
            if !matched_left_indices[idx] {
                joined_results.push(JoinedResult::new(
                    left_result.clone(),
                    null_row_data.clone(),
                ));
            }
        }
    }

    Ok(joined_results)
}

/// Resolves JOIN condition for execution from either `ON` or `USING` syntax.
///
/// Current runtime supports:
/// - `JOIN ... ON left = right`
/// - `JOIN ... USING (single_column)`
///
/// `USING` with multiple columns is currently not supported because execution
/// path relies on a single primary key lookup.
fn resolve_join_condition(join: &JoinClause) -> Option<JoinCondition> {
    if let Some(condition) = &join.condition {
        return Some(normalize_join_condition(condition, join));
    }

    let Some(using_columns) = &join.using_columns else {
        return None;
    };

    if using_columns.len() != 1 {
        return None;
    }

    let join_column = using_columns[0].clone();
    Some(JoinCondition {
        left: ColumnRef {
            table: Some(join.table.clone()),
            column: join_column.clone(),
        },
        right: ColumnRef {
            table: None,
            column: join_column,
        },
    })
}

/// Normalizes ON condition so that `left` refers to the joined table and `right`
/// refers to the current result set side.
fn normalize_join_condition(condition: &JoinCondition, join: &JoinClause) -> JoinCondition {
    let is_join_side = |table: Option<&str>| {
        table.is_some_and(|t| t == join.table || join.alias.as_deref().is_some_and(|a| a == t))
    };

    if is_join_side(condition.left.table.as_deref()) {
        return condition.clone();
    }

    if is_join_side(condition.right.table.as_deref()) {
        return JoinCondition {
            left: condition.right.clone(),
            right: condition.left.clone(),
        };
    }

    condition.clone()
}

/// Batch get rows from ColumnStore by primary keys.
///
/// Returns a map of pk -> column values (as JSON) for found rows.
fn batch_get_rows(
    column_store: &ColumnStore,
    pks: &[i64],
) -> HashMap<i64, HashMap<String, serde_json::Value>> {
    let mut result = HashMap::with_capacity(pks.len());

    for &pk in pks {
        if let Some(row_idx) = column_store.get_row_idx_by_pk(pk) {
            // Get all column values for this row
            let mut row_data = HashMap::new();
            for col_name in column_store.column_names() {
                if let Some(value) = column_store.get_value_as_json(col_name, row_idx) {
                    row_data.insert(col_name.to_string(), value);
                }
            }
            result.insert(pk, row_data);
        }
    }

    result
}

fn row_as_json_map(
    column_store: &ColumnStore,
    row_idx: usize,
) -> HashMap<String, serde_json::Value> {
    let mut row_data = HashMap::new();
    for col_name in column_store.column_names() {
        if let Some(value) = column_store.get_value_as_json(col_name, row_idx) {
            row_data.insert(col_name.to_string(), value);
        }
    }
    row_data
}

fn build_null_row_data(column_store: &ColumnStore) -> HashMap<String, serde_json::Value> {
    column_store
        .column_names()
        .map(|name| (name.to_string(), serde_json::Value::Null))
        .collect()
}

/// Converts JoinedResults back to SearchResults with merged payload.
///
/// This is useful when the query expects SearchResult format but
/// we want to include joined column data in the payload.
#[must_use]
pub fn joined_to_search_results(joined: Vec<JoinedResult>) -> Vec<SearchResult> {
    joined
        .into_iter()
        .map(|jr| {
            let mut result = jr.search_result;

            // Merge column data into payload
            let mut payload = result
                .point
                .payload
                .take()
                .and_then(|p| p.as_object().cloned())
                .unwrap_or_default();

            for (key, value) in &jr.column_data {
                payload.insert(key.clone(), value.clone());
            }

            result.point.payload = Some(serde_json::Value::Object(payload));
            result
        })
        .collect()
}

// Tests moved to join_tests.rs per project rules
