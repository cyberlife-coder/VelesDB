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
    let (column_names, include_score) = match columns {
        SelectColumns::Columns(cols) => (cols.iter().map(|c| c.name.clone()).collect(), false),
        SelectColumns::Mixed {
            columns: cols,
            similarity_scores,
            ..
        } => (
            cols.iter().map(|c| c.name.clone()).collect(),
            !similarity_scores.is_empty(),
        ),
        SelectColumns::SimilarityScore(_) => (Vec::new(), true),
        // All, Aggregations, QualifiedWildcard: use full payload or no dedup
        SelectColumns::All
        | SelectColumns::Aggregations(_)
        | SelectColumns::QualifiedWildcard(_) => (Vec::new(), false),
    };

    let mut seen: FxHashSet<String> = FxHashSet::default();
    results
        .into_iter()
        .filter(|r| {
            let key = compute_distinct_key(r, &column_names, include_score);
            seen.insert(key)
        })
        .collect()
}

/// Compute a unique key for DISTINCT deduplication.
///
/// Uses canonical JSON representation with sorted keys to ensure
/// logically equal objects produce identical keys.
/// When `include_score` is true, the similarity score is appended to the key
/// so rows differing only by score are not collapsed.
pub fn compute_distinct_key(
    result: &SearchResult,
    columns: &[String],
    include_score: bool,
) -> String {
    let payload = result.point.payload.as_ref();

    let mut key = if columns.is_empty() {
        // SELECT * or SELECT DISTINCT *: use full payload as key
        payload.map_or_else(|| "null".to_string(), canonical_json_string)
    } else {
        // SELECT DISTINCT col1, col2: use specific columns
        columns
            .iter()
            .map(|col| {
                payload
                    .and_then(|p| p.get(col))
                    .map_or_else(|| "null".to_string(), canonical_json_string)
            })
            .collect::<Vec<_>>()
            .join("\x1F") // ASCII Unit Separator
    };

    if include_score {
        key.push('\x1F');
        key.push_str(&result.score.to_string());
    }

    key
}

/// Produce a canonical JSON string with sorted object keys.
///
/// This ensures that logically equal JSON objects produce identical strings,
/// regardless of the original key order (e.g., `{"a":1,"b":2}` == `{"b":2,"a":1}`).
fn canonical_json_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            // Sort keys alphabetically for deterministic output
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let pairs: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", k, canonical_json_string(&map[*k])))
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json_string).collect();
            format!("[{}]", items.join(","))
        }
        // Primitives: use standard JSON representation
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Point;

    fn make_result(id: u64, payload: serde_json::Value) -> SearchResult {
        make_result_with_score(id, payload, 1.0)
    }

    fn make_result_with_score(id: u64, payload: serde_json::Value, score: f32) -> SearchResult {
        SearchResult::new(
            Point {
                id,
                vector: vec![0.0; 4],
                payload: Some(payload),
                sparse_vectors: None,
            },
            score,
        )
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
        let key = compute_distinct_key(&result, &[], false);
        assert!(key.contains('1')); // Full payload serialized
    }

    #[test]
    fn test_compute_distinct_key_specific_columns() {
        let result = make_result(1, serde_json::json!({"name": "Alice", "age": 30}));
        let key = compute_distinct_key(&result, &["name".to_string()], false);
        assert!(key.contains("Alice"));
        assert!(!key.contains("30"));
    }

    #[test]
    fn test_canonical_json_key_order_independent() {
        // Two payloads with same content but different key order
        let result1 = make_result(1, serde_json::json!({"a": 1, "b": 2}));
        let result2 = make_result(2, serde_json::json!({"b": 2, "a": 1}));

        let key1 = compute_distinct_key(&result1, &[], false);
        let key2 = compute_distinct_key(&result2, &[], false);

        // Keys should be identical regardless of original key order
        assert_eq!(
            key1, key2,
            "Canonical JSON should produce same key for logically equal objects"
        );
    }

    #[test]
    fn test_canonical_json_nested_objects() {
        let result1 = make_result(1, serde_json::json!({"outer": {"z": 1, "a": 2}}));
        let result2 = make_result(2, serde_json::json!({"outer": {"a": 2, "z": 1}}));

        let key1 = compute_distinct_key(&result1, &[], false);
        let key2 = compute_distinct_key(&result2, &[], false);

        assert_eq!(key1, key2, "Nested objects should also be canonicalized");
    }

    #[test]
    fn test_distinct_mixed_with_similarity_preserves_different_scores() {
        let results = vec![
            make_result_with_score(1, serde_json::json!({"title": "Rust"}), 0.95),
            make_result_with_score(2, serde_json::json!({"title": "Rust"}), 0.80),
            make_result_with_score(3, serde_json::json!({"title": "Go"}), 0.70),
        ];

        let columns = SelectColumns::Mixed {
            columns: vec![crate::velesql::Column {
                name: "title".to_string(),
                alias: None,
            }],
            aggregations: vec![],
            similarity_scores: vec![crate::velesql::SimilarityScoreExpr {
                alias: Some("score".to_string()),
            }],
            qualified_wildcards: vec![],
        };

        let distinct = apply_distinct(results, &columns);
        // Both "Rust" rows should be kept because scores differ
        assert_eq!(distinct.len(), 3);
    }

    #[test]
    fn test_distinct_mixed_without_similarity_collapses_same_payload() {
        let results = vec![
            make_result_with_score(1, serde_json::json!({"title": "Rust"}), 0.95),
            make_result_with_score(2, serde_json::json!({"title": "Rust"}), 0.80),
        ];

        let columns = SelectColumns::Mixed {
            columns: vec![crate::velesql::Column {
                name: "title".to_string(),
                alias: None,
            }],
            aggregations: vec![],
            similarity_scores: vec![],
            qualified_wildcards: vec![],
        };

        let distinct = apply_distinct(results, &columns);
        // Without similarity in SELECT, same title → collapsed
        assert_eq!(distinct.len(), 1);
    }
}
