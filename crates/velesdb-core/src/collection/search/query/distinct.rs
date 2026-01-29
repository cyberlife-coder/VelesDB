//! DISTINCT deduplication for query results (EPIC-061/US-003 refactoring).
//!
//! Extracted from mod.rs to reduce file size and improve modularity.

use crate::point::SearchResult;
use crate::velesql::SelectColumns;
use rustc_hash::FxHashSet;

/// Apply DISTINCT deduplication to results based on selected columns (EPIC-052 US-001).
///
/// Uses HashSet for O(n) complexity and preserves insertion order.
pub fn apply_distinct(results: Vec<SearchResult>, columns: &SelectColumns) -> Vec<SearchResult> {
    // If SELECT *, deduplicate by all payload fields
    let column_names: Vec<String> = match columns {
        SelectColumns::Columns(cols) => cols.iter().map(|c| c.name.clone()).collect(),
        SelectColumns::Mixed { columns: cols, .. } => cols.iter().map(|c| c.name.clone()).collect(),
        // All or Aggregations: use full payload or no deduplication
        SelectColumns::All | SelectColumns::Aggregations(_) => Vec::new(),
    };

    let mut seen: FxHashSet<String> = FxHashSet::default();
    results
        .into_iter()
        .filter(|r| {
            let key = compute_distinct_key(r, &column_names);
            seen.insert(key)
        })
        .collect()
}

/// Compute a unique key for DISTINCT deduplication.
pub fn compute_distinct_key(result: &SearchResult, columns: &[String]) -> String {
    let payload = result.point.payload.as_ref();

    if columns.is_empty() {
        // SELECT * or SELECT DISTINCT *: use full payload as key
        payload.map_or_else(|| "null".to_string(), ToString::to_string)
    } else {
        // SELECT DISTINCT col1, col2: use specific columns
        columns
            .iter()
            .map(|col| {
                payload
                    .and_then(|p| p.get(col))
                    .map_or_else(|| "null".to_string(), ToString::to_string)
            })
            .collect::<Vec<_>>()
            .join("\x1F") // ASCII Unit Separator
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Point;

    fn make_result(id: u64, payload: serde_json::Value) -> SearchResult {
        SearchResult {
            point: Point {
                id,
                vector: vec![0.0; 4], // Dummy vector for tests
                payload: Some(payload),
            },
            score: 1.0,
        }
    }

    #[test]
    fn test_apply_distinct_removes_duplicates() {
        let results = vec![
            make_result(1, serde_json::json!({"name": "Alice"})),
            make_result(2, serde_json::json!({"name": "Alice"})),
            make_result(3, serde_json::json!({"name": "Bob"})),
        ];

        let columns = SelectColumns::Columns(vec![crate::velesql::Column {
            name: "name".to_string(),
            alias: None,
        }]);

        let distinct = apply_distinct(results, &columns);
        assert_eq!(distinct.len(), 2);
    }

    #[test]
    fn test_compute_distinct_key_empty_columns() {
        let result = make_result(1, serde_json::json!({"a": 1, "b": 2}));
        let key = compute_distinct_key(&result, &[]);
        assert!(key.contains('1')); // Full payload serialized
    }

    #[test]
    fn test_compute_distinct_key_specific_columns() {
        let result = make_result(1, serde_json::json!({"name": "Alice", "age": 30}));
        let key = compute_distinct_key(&result, &["name".to_string()]);
        assert!(key.contains("Alice"));
        assert!(!key.contains("30"));
    }
}
