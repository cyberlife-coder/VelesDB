//! JOIN execution for cross-store queries (EPIC-031 US-005).
//!
//! This module implements JOIN execution between graph traversal results
//! and ColumnStore data with adaptive batch sizing.
//!
//! Note: Functions in this module are tested but not yet integrated into
//! execute_query. Integration is planned for future work.

#![allow(dead_code)]

use crate::column_store::ColumnStore;
use crate::point::SearchResult;
use crate::velesql::{JoinClause, JoinCondition};
use std::collections::HashMap;

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
pub fn execute_join(
    results: &[SearchResult],
    join: &JoinClause,
    column_store: &ColumnStore,
) -> Vec<JoinedResult> {
    // EPIC-040 US-003: Handle Option<JoinCondition> - USING clause not yet supported for execution
    let condition = match &join.condition {
        Some(cond) => cond,
        None => {
            // USING clause - not yet implemented for execution
            // Would need to resolve column from both tables
            return Vec::new();
        }
    };

    // 1. Validate that join column matches ColumnStore's primary key
    // This prevents silent incorrect results when joining on non-PK columns
    let join_column = &condition.left.column;
    if let Some(pk_column) = column_store.primary_key_column() {
        if join_column != pk_column {
            // Cannot join on non-primary-key column - return empty results
            // In the future, this could use secondary indexes
            return Vec::new();
        }
    } else {
        // ColumnStore has no primary key configured - cannot perform PK-based join
        return Vec::new();
    }

    // 2. Extract join keys from search results
    let join_keys = extract_join_keys(results, condition);

    if join_keys.is_empty() {
        return Vec::new();
    }

    // 3. Determine adaptive batch size
    let batch_size = adaptive_batch_size(join_keys.len());

    // 4. Build result map: pk -> (result_idx, row_data)
    let mut joined_results = Vec::with_capacity(join_keys.len());

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
            }
            // Inner JOIN: skip results without matching column data
        }
    }

    joined_results
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

/// Converts JoinedResults back to SearchResults with merged payload.
///
/// This is useful when the query expects SearchResult format but
/// we want to include joined column data in the payload.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column_store::{ColumnType, ColumnValue};
    use crate::point::Point;
    use crate::velesql::ColumnRef;

    fn make_search_result(id: u64, payload_id: i64) -> SearchResult {
        SearchResult {
            point: Point {
                id,
                vector: vec![0.1, 0.2, 0.3],
                payload: Some(
                    serde_json::json!({"id": payload_id, "name": format!("item_{}", id)}),
                ),
            },
            score: 0.9,
        }
    }

    fn make_column_store() -> ColumnStore {
        let mut store = ColumnStore::with_primary_key(
            &[
                ("product_id", ColumnType::Int),
                ("price", ColumnType::Float),
                ("available", ColumnType::Bool),
            ],
            "product_id",
        );

        // Insert test rows
        store
            .insert_row(&[
                ("product_id", ColumnValue::Int(1)),
                ("price", ColumnValue::Float(99.99)),
                ("available", ColumnValue::Bool(true)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("product_id", ColumnValue::Int(2)),
                ("price", ColumnValue::Float(149.99)),
                ("available", ColumnValue::Bool(false)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("product_id", ColumnValue::Int(3)),
                ("price", ColumnValue::Float(49.99)),
                ("available", ColumnValue::Bool(true)),
            ])
            .unwrap();

        store
    }

    fn make_join_clause() -> JoinClause {
        JoinClause {
            join_type: crate::velesql::JoinType::Inner,
            table: "prices".to_string(),
            alias: None,
            condition: Some(JoinCondition {
                left: ColumnRef {
                    table: Some("prices".to_string()),
                    column: "product_id".to_string(),
                },
                right: ColumnRef {
                    table: Some("products".to_string()),
                    column: "id".to_string(),
                },
            }),
            using_columns: None,
        }
    }

    #[test]
    fn test_adaptive_batch_size_small() {
        assert_eq!(adaptive_batch_size(50), 50);
        assert_eq!(adaptive_batch_size(100), 100);
    }

    #[test]
    fn test_adaptive_batch_size_medium() {
        assert_eq!(adaptive_batch_size(101), 1000);
        assert_eq!(adaptive_batch_size(5000), 1000);
        assert_eq!(adaptive_batch_size(10000), 1000);
    }

    #[test]
    fn test_adaptive_batch_size_large() {
        assert_eq!(adaptive_batch_size(10001), 5000);
        assert_eq!(adaptive_batch_size(100_000), 5000);
    }

    #[test]
    fn test_extract_join_keys() {
        let results = vec![
            make_search_result(1, 1),
            make_search_result(2, 2),
            make_search_result(3, 3),
        ];
        let join = make_join_clause();

        let keys = extract_join_keys(&results, join.condition.as_ref().unwrap());

        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0], (0, 1));
        assert_eq!(keys[1], (1, 2));
        assert_eq!(keys[2], (2, 3));
    }

    #[test]
    fn test_execute_join_basic() {
        let results = vec![
            make_search_result(1, 1),
            make_search_result(2, 2),
            make_search_result(3, 3),
        ];
        let column_store = make_column_store();
        let join = make_join_clause();

        let joined = execute_join(&results, &join, &column_store);

        assert_eq!(joined.len(), 3);

        // Check first result has price data
        assert!(joined[0].column_data.contains_key("price"));
        let price = joined[0]
            .column_data
            .get("price")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((price - 99.99).abs() < 0.01);
    }

    #[test]
    fn test_execute_join_inner_skips_missing() {
        // Only product_id 1 and 2 exist in column store
        let results = vec![
            make_search_result(1, 1),
            make_search_result(2, 99), // No matching row
            make_search_result(3, 3),
        ];
        let column_store = make_column_store();
        let join = make_join_clause();

        let joined = execute_join(&results, &join, &column_store);

        // Inner JOIN: only 2 results (id=1 and id=3 match)
        assert_eq!(joined.len(), 2);
    }

    #[test]
    fn test_joined_to_search_results() {
        let results = vec![make_search_result(1, 1)];
        let column_store = make_column_store();
        let join = make_join_clause();

        let joined = execute_join(&results, &join, &column_store);
        let search_results = joined_to_search_results(joined);

        assert_eq!(search_results.len(), 1);

        // Check payload contains merged data
        let payload = search_results[0].point.payload.as_ref().unwrap();
        assert!(payload.get("price").is_some());
        assert!(payload.get("available").is_some());
    }

    // ========== REGRESSION TESTS FOR PR #85 BUGS ==========

    /// Regression test for BUG-1: u64 to i64 overflow
    /// Point IDs > i64::MAX should not cause incorrect join keys
    #[test]
    fn test_extract_join_keys_u64_overflow_safety() {
        // Create a search result with a very large u64 ID (> i64::MAX)
        let large_id = u64::MAX;
        let result = SearchResult {
            point: Point {
                id: large_id,
                vector: vec![0.1, 0.2, 0.3],
                payload: None, // No payload, will try to use point.id
            },
            score: 0.9,
        };

        let condition = JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        };

        let keys = extract_join_keys(&[result], &condition);

        // Should return empty or valid positive key, not a negative overflow value
        // Before fix: would return (0, -1) due to overflow
        // After fix: should return empty (None) since u64::MAX doesn't fit in i64
        assert!(
            keys.is_empty() || keys.iter().all(|(_, k)| *k >= 0),
            "Large u64 IDs should not produce negative join keys: {:?}",
            keys
        );
    }

    /// Regression test for BUG-1: Boundary case at i64::MAX
    #[test]
    fn test_extract_join_keys_i64_max_boundary() {
        // i64::MAX should work fine
        let max_safe_id = i64::MAX as u64;
        let result = SearchResult {
            point: Point {
                id: max_safe_id,
                vector: vec![0.1, 0.2, 0.3],
                payload: None,
            },
            score: 0.9,
        };

        let condition = JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        };

        let keys = extract_join_keys(&[result], &condition);

        // i64::MAX as u64 should safely convert back to i64::MAX
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].1, i64::MAX);
    }

    /// Regression test for BUG-1: Value just above i64::MAX
    #[test]
    fn test_extract_join_keys_just_above_i64_max() {
        // i64::MAX + 1 should not produce incorrect results
        let just_over = (i64::MAX as u64) + 1;
        let result = SearchResult {
            point: Point {
                id: just_over,
                vector: vec![0.1, 0.2, 0.3],
                payload: None,
            },
            score: 0.9,
        };

        let condition = JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        };

        let keys = extract_join_keys(&[result], &condition);

        // Should be empty (filtered out) rather than overflow to negative
        assert!(
            keys.is_empty(),
            "IDs > i64::MAX should be filtered out, got: {:?}",
            keys
        );
    }

    /// Regression test for BUG-2: JOIN must validate PK column
    #[test]
    fn test_execute_join_validates_pk_column() {
        let results = vec![make_search_result(1, 1)];
        let column_store = make_column_store(); // PK is "product_id"

        // Create join with WRONG left column (category_id instead of product_id)
        let wrong_join = JoinClause {
            join_type: crate::velesql::JoinType::Inner,
            table: "prices".to_string(),
            alias: None,
            condition: Some(JoinCondition {
                left: ColumnRef {
                    table: Some("prices".to_string()),
                    column: "category_id".to_string(), // NOT the PK!
                },
                right: ColumnRef {
                    table: Some("products".to_string()),
                    column: "id".to_string(),
                },
            }),
            using_columns: None,
        };

        let joined = execute_join(&results, &wrong_join, &column_store);

        // Before fix: would silently use product_id as PK anyway
        // After fix: should return empty (PK mismatch) or error
        // For now, we expect empty results since we can't join on non-PK
        assert!(
            joined.is_empty(),
            "JOIN on non-PK column should not return results silently"
        );
    }

    /// Regression test for BUG-2: JOIN with correct PK column should work
    #[test]
    fn test_execute_join_correct_pk_column_works() {
        let results = vec![make_search_result(1, 1)];
        let column_store = make_column_store(); // PK is "product_id"

        // Correct join with actual PK column
        let correct_join = JoinClause {
            join_type: crate::velesql::JoinType::Inner,
            table: "prices".to_string(),
            alias: None,
            condition: Some(JoinCondition {
                left: ColumnRef {
                    table: Some("prices".to_string()),
                    column: "product_id".to_string(), // Correct PK
                },
                right: ColumnRef {
                    table: Some("products".to_string()),
                    column: "id".to_string(),
                },
            }),
            using_columns: None,
        };

        let joined = execute_join(&results, &correct_join, &column_store);

        // Should work correctly
        assert_eq!(joined.len(), 1);
    }
}
