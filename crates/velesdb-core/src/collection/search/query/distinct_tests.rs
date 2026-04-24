//! Additional tests for DISTINCT deduplication logic.
//!
//! The core unit tests live inline in `distinct.rs`. These tests exercise
//! edge cases: null payloads, mixed types, and `SelectColumns::All`.

#[cfg(test)]
mod tests {
    use crate::collection::search::query::distinct::{apply_distinct, compute_distinct_key};
    use crate::point::{Point, SearchResult};
    use crate::velesql::{Column, SelectColumns};

    fn make_result(id: u64, payload: Option<serde_json::Value>) -> SearchResult {
        SearchResult::new(
            Point {
                id,
                vector: vec![0.0; 4],
                payload,
                sparse_vectors: None,
            },
            1.0,
        )
    }

    // -----------------------------------------------------------------------
    // Null / missing payload handling
    // -----------------------------------------------------------------------

    #[test]
    fn test_distinct_null_payloads_collapse_to_one() {
        let results = vec![
            make_result(1, None),
            make_result(2, None),
            make_result(3, None),
        ];
        let columns = SelectColumns::All;
        let distinct = apply_distinct(results, &columns);

        // All three have null payload -> same key -> only first survives.
        assert_eq!(distinct.len(), 1, "null payloads should deduplicate to one");
        assert_eq!(distinct[0].point.id, 1, "first inserted wins");
    }

    #[test]
    fn test_distinct_null_vs_some_are_different() {
        let results = vec![
            make_result(1, None),
            make_result(2, Some(serde_json::json!({"name": "Alice"}))),
        ];
        let columns = SelectColumns::All;
        let distinct = apply_distinct(results, &columns);

        assert_eq!(distinct.len(), 2, "null vs Some should be distinct");
    }

    // -----------------------------------------------------------------------
    // SelectColumns::All deduplication
    // -----------------------------------------------------------------------

    #[test]
    fn test_distinct_select_all_uses_full_payload() {
        let results = vec![
            make_result(1, Some(serde_json::json!({"a": 1, "b": 2}))),
            make_result(2, Some(serde_json::json!({"a": 1, "b": 2}))),
            make_result(3, Some(serde_json::json!({"a": 1, "b": 3}))),
        ];
        let columns = SelectColumns::All;
        let distinct = apply_distinct(results, &columns);

        // id=1 and id=2 are identical payloads -> collapse.
        assert_eq!(distinct.len(), 2);
    }

    // -----------------------------------------------------------------------
    // compute_distinct_key with missing column
    // -----------------------------------------------------------------------

    #[test]
    fn test_distinct_key_missing_column_is_null() {
        let r = make_result(1, Some(serde_json::json!({"name": "Alice"})));
        let key = compute_distinct_key(&r, &["missing_field".to_string()], false);
        assert_eq!(key, "null", "missing column should produce 'null' key");
    }

    // -----------------------------------------------------------------------
    // Mixed column types produce unique keys
    // -----------------------------------------------------------------------

    #[test]
    fn test_distinct_mixed_column_types() {
        let results = vec![
            make_result(1, Some(serde_json::json!({"val": 42}))),
            make_result(2, Some(serde_json::json!({"val": "42"}))),
        ];

        let columns = SelectColumns::Columns(vec![Column {
            name: "val".to_string(),
            alias: None,
        }]);
        let distinct = apply_distinct(results, &columns);

        // Number 42 vs string "42" should be distinct (different JSON repr).
        assert_eq!(distinct.len(), 2, "number vs string should be distinct");
    }

    // -----------------------------------------------------------------------
    // Mixed with qualified_wildcards must dedup by FULL payload, not just
    // the explicit `columns` list. Regression for the Devin finding:
    // `SELECT DISTINCT ctx.*, title FROM docs` must collapse rows only when
    // ALL payload fields match, not only when `title` matches.
    // -----------------------------------------------------------------------

    #[test]
    fn test_distinct_mixed_with_qualified_wildcard_dedupes_by_full_payload() {
        // Two rows share `title` but differ in a wildcard-expanded field
        // (`author`). Without the fix, dedup would collapse them (only
        // `title` considered). With the fix, dedup uses the full payload,
        // so both survive.
        let results = vec![
            make_result(
                1,
                Some(serde_json::json!({"title": "T1", "author": "Alice"})),
            ),
            make_result(2, Some(serde_json::json!({"title": "T1", "author": "Bob"}))),
            // Third row is an exact duplicate of row 1 → should collapse.
            make_result(
                3,
                Some(serde_json::json!({"title": "T1", "author": "Alice"})),
            ),
        ];

        let columns = SelectColumns::Mixed {
            columns: vec![Column {
                name: "title".to_string(),
                alias: None,
            }],
            aggregations: Vec::new(),
            similarity_scores: Vec::new(),
            qualified_wildcards: vec!["ctx".to_string()],
            window_functions: Vec::new(),
        };

        let distinct = apply_distinct(results, &columns);
        assert_eq!(
            distinct.len(),
            2,
            "rows 1 and 2 differ (Alice vs Bob) so both survive; row 3 is an \
             exact duplicate of row 1 and is dropped"
        );
        // Preserves insertion order: row 1 and row 2 survive.
        assert_eq!(distinct[0].point.id, 1);
        assert_eq!(distinct[1].point.id, 2);
    }

    /// Control case: same query shape but WITHOUT qualified_wildcards —
    /// dedup must still use only the explicit `columns` list (current
    /// behaviour preserved for backward compatibility).
    #[test]
    fn test_distinct_mixed_without_qualified_wildcard_dedupes_by_columns_only() {
        let results = vec![
            make_result(
                1,
                Some(serde_json::json!({"title": "T1", "author": "Alice"})),
            ),
            make_result(2, Some(serde_json::json!({"title": "T1", "author": "Bob"}))),
        ];

        let columns = SelectColumns::Mixed {
            columns: vec![Column {
                name: "title".to_string(),
                alias: None,
            }],
            aggregations: Vec::new(),
            similarity_scores: Vec::new(),
            qualified_wildcards: Vec::new(),
            window_functions: Vec::new(),
        };

        let distinct = apply_distinct(results, &columns);
        assert_eq!(
            distinct.len(),
            1,
            "no wildcard → dedup by `title` only → both rows collapse"
        );
        assert_eq!(distinct[0].point.id, 1);
    }
}
