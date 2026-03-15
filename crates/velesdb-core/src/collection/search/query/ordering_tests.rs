//! Tests for ORDER BY clause execution and JSON value comparison.

#[cfg(test)]
mod tests {
    use crate::collection::search::query::ordering::compare_json_values;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // compare_json_values — basic type comparisons
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_none_vs_none_is_equal() {
        assert_eq!(compare_json_values(None, None), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_compare_none_vs_some_is_less() {
        let v = json!(42);
        assert_eq!(
            compare_json_values(None, Some(&v)),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_some_vs_none_is_greater() {
        let v = json!("hello");
        assert_eq!(
            compare_json_values(Some(&v), None),
            std::cmp::Ordering::Greater
        );
    }

    // -----------------------------------------------------------------------
    // Same type: numbers
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_numbers_ascending() {
        let a = json!(10);
        let b = json!(20);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_numbers_equal() {
        let a = json!(42);
        let b = json!(42);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_compare_numbers_descending() {
        let a = json!(100);
        let b = json!(50);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Greater
        );
    }

    // -----------------------------------------------------------------------
    // Same type: strings
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_strings_alphabetical() {
        let a = json!("apple");
        let b = json!("banana");
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
    }

    // -----------------------------------------------------------------------
    // Same type: booleans
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_bools_false_less_than_true() {
        let a = json!(false);
        let b = json!(true);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
    }

    // -----------------------------------------------------------------------
    // Mixed types: type rank ordering
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_null_less_than_number() {
        let a = json!(null);
        let b = json!(42);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "null (rank 0) < number (rank 2)"
        );
    }

    #[test]
    fn test_compare_number_less_than_string() {
        let a = json!(99);
        let b = json!("99");
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "number (rank 2) < string (rank 3)"
        );
    }

    #[test]
    fn test_compare_string_less_than_array() {
        let a = json!("hello");
        let b = json!([1, 2, 3]);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "string (rank 3) < array (rank 4)"
        );
    }

    #[test]
    fn test_compare_array_less_than_object() {
        let a = json!([1]);
        let b = json!({"key": "value"});
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "array (rank 4) < object (rank 5)"
        );
    }

    // -----------------------------------------------------------------------
    // Integration: ORDER BY field via VelesQL
    // -----------------------------------------------------------------------

    #[cfg(feature = "persistence")]
    mod integration {
        use crate::collection::types::Collection;
        use crate::distance::DistanceMetric;
        use crate::point::Point;
        use crate::velesql::Parser;
        use std::collections::HashMap;
        use std::path::PathBuf;

        fn setup_ordered_collection() -> (tempfile::TempDir, Collection) {
            let dir = tempfile::tempdir().expect("temp dir");
            let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
                .expect("create collection");

            let points = vec![
                Point {
                    id: 1,
                    vector: vec![1.0, 0.0, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 3, "name": "charlie"})),
                    sparse_vectors: None,
                },
                Point {
                    id: 2,
                    vector: vec![0.9, 0.1, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 1, "name": "alpha"})),
                    sparse_vectors: None,
                },
                Point {
                    id: 3,
                    vector: vec![0.8, 0.2, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 2, "name": "bravo"})),
                    sparse_vectors: None,
                },
            ];
            col.upsert(points).expect("upsert");
            (dir, col)
        }

        #[test]
        fn test_order_by_field_asc() {
            let (_dir, col) = setup_ordered_collection();

            let query = "SELECT * FROM test ORDER BY priority ASC LIMIT 10";
            let parsed = Parser::parse(query).expect("parse");
            let params = HashMap::new();

            let results = col.execute_query(&parsed, &params).expect("execute");
            assert!(results.len() >= 2, "should have results to sort");

            // Verify ascending order on priority.
            for window in results.windows(2) {
                let p0 = window[0]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("priority"))
                    .and_then(serde_json::Value::as_i64);
                let p1 = window[1]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("priority"))
                    .and_then(serde_json::Value::as_i64);
                assert!(
                    p0 <= p1,
                    "priority should be ascending: {:?} <= {:?}",
                    p0,
                    p1
                );
            }
        }

        #[test]
        fn test_order_by_field_desc() {
            let (_dir, col) = setup_ordered_collection();

            let query = "SELECT * FROM test ORDER BY name DESC LIMIT 10";
            let parsed = Parser::parse(query).expect("parse");
            let params = HashMap::new();

            let results = col.execute_query(&parsed, &params).expect("execute");
            assert!(results.len() >= 2, "should have results");

            // Verify descending order on name.
            for window in results.windows(2) {
                let n0 = window[0]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let n1 = window[1]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                assert!(n0 >= n1, "name should be descending: {} >= {}", n0, n1);
            }
        }
    }
}
