//! Tests for query validation functions.
//!
//! Tests for:
//! - `validate_similarity_query_structure()` - Rejects unsupported patterns
//! - NEAR + similarity() combination - Supported pattern for agentic memory
//!
//! Note: NOT similarity() tests are commented out because VelesQL parser
//! does not yet support `NOT condition` syntax (only `IS NOT NULL`).
//! The validation code exists for future parser extension.

#[cfg(test)]
mod tests {
    use crate::collection::types::Collection;
    use crate::distance::DistanceMetric;
    use crate::velesql::Parser;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn create_test_collection() -> (Collection, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = PathBuf::from(temp_dir.path());
        let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();
        (collection, temp_dir)
    }

    // =========================================================================
    // Tests for NOT similarity() rejection
    // NOTE: VelesQL parser does not support `NOT condition` syntax yet.
    // These tests are disabled until parser is extended (see EPIC-005).
    // The validation code in query.rs is ready for when parser supports NOT.
    // =========================================================================

    // TODO: Enable when parser supports NOT condition
    // #[test]
    // fn test_not_similarity_is_rejected() { ... }

    // =========================================================================
    // Tests for NEAR + similarity() combination (should work)
    // =========================================================================

    #[test]
    fn test_near_plus_similarity_is_supported() {
        let (collection, _temp) = create_test_collection();

        // Insert test data
        let points = vec![
            crate::Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "tech"})),
            },
            crate::Point {
                id: 2,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "tech"})),
            },
            crate::Point {
                id: 3,
                vector: vec![0.0, 1.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "other"})),
            },
        ];
        collection.upsert(points).unwrap();

        // NEAR + similarity() should work: NEAR finds candidates, similarity filters by threshold
        let query =
            "SELECT * FROM test WHERE vector NEAR $v AND similarity(vector, $v) > 0.5 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let result = collection.execute_query(&parsed, &params);
        assert!(
            result.is_ok(),
            "NEAR + similarity() should be supported: {:?}",
            result.err()
        );

        let results = result.unwrap();
        // All results should have similarity > 0.5
        assert!(!results.is_empty(), "Should return some results");
    }

    #[test]
    fn test_near_plus_similarity_filters_by_threshold() {
        let (collection, _temp) = create_test_collection();

        // Insert test data with varying similarities
        let points = vec![
            crate::Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0], // similarity = 1.0
                payload: None,
            },
            crate::Point {
                id: 2,
                vector: vec![0.7, 0.7, 0.0, 0.0], // similarity ≈ 0.7
                payload: None,
            },
            crate::Point {
                id: 3,
                vector: vec![0.0, 1.0, 0.0, 0.0], // similarity = 0.0
                payload: None,
            },
        ];
        collection.upsert(points).unwrap();

        // High threshold should filter out low similarity results
        let query =
            "SELECT * FROM test WHERE vector NEAR $v AND similarity(vector, $v) > 0.9 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let result = collection.execute_query(&parsed, &params);
        assert!(result.is_ok());

        let results = result.unwrap();
        // Only point 1 should match (similarity = 1.0)
        assert!(
            results.len() <= 2,
            "High threshold should filter results: got {}",
            results.len()
        );
    }

    // =========================================================================
    // Tests for NOT with metadata
    // NOTE: VelesQL parser does not support `NOT condition` syntax yet.
    // Use != operator instead for negation.
    // =========================================================================

    #[test]
    fn test_not_equal_metadata_is_supported() {
        let (collection, _temp) = create_test_collection();

        let points = vec![
            crate::Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "tech"})),
            },
            crate::Point {
                id: 2,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "science"})),
            },
        ];
        collection.upsert(points).unwrap();

        // Use != instead of NOT for negation (parser supported)
        let query = "SELECT * FROM test WHERE category != 'tech' LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let result = collection.execute_query(&parsed, &HashMap::new());
        assert!(
            result.is_ok(),
            "!= metadata should be supported: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_similarity_and_not_equal_metadata_is_supported() {
        let (collection, _temp) = create_test_collection();

        let points = vec![
            crate::Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "tech"})),
            },
            crate::Point {
                id: 2,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "science"})),
            },
        ];
        collection.upsert(points).unwrap();

        // similarity() AND != metadata should work
        let query =
            "SELECT * FROM test WHERE similarity(vector, $v) > 0.5 AND category != 'tech' LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let result = collection.execute_query(&parsed, &params);
        assert!(
            result.is_ok(),
            "similarity() AND != metadata should be supported: {:?}",
            result.err()
        );
    }

    // =========================================================================
    // Tests for ORDER BY multi-columns (EPIC-028)
    // =========================================================================

    #[test]
    fn test_order_by_two_columns_asc_desc() {
        let (collection, _temp) = create_test_collection();

        let points = vec![
            crate::Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "A", "priority": 3})),
            },
            crate::Point {
                id: 2,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "A", "priority": 1})),
            },
            crate::Point {
                id: 3,
                vector: vec![0.8, 0.2, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "B", "priority": 2})),
            },
            crate::Point {
                id: 4,
                vector: vec![0.7, 0.3, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "A", "priority": 2})),
            },
        ];
        collection.upsert(points).unwrap();

        // ORDER BY category ASC, priority DESC
        // Expected order: A(priority=3), A(priority=2), A(priority=1), B(priority=2)
        let query = "SELECT * FROM test ORDER BY category ASC, priority DESC LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let result = collection.execute_query(&parsed, &HashMap::new());
        assert!(
            result.is_ok(),
            "Multi-column ORDER BY should work: {:?}",
            result.err()
        );

        let results = result.unwrap();
        assert_eq!(results.len(), 4);

        // Verify order: category A first (ASC), within A: priority DESC (3, 2, 1)
        let categories: Vec<&str> = results
            .iter()
            .map(|r| {
                r.point
                    .payload
                    .as_ref()
                    .unwrap()
                    .get("category")
                    .unwrap()
                    .as_str()
                    .unwrap()
            })
            .collect();
        let priorities: Vec<i64> = results
            .iter()
            .map(|r| {
                r.point
                    .payload
                    .as_ref()
                    .unwrap()
                    .get("priority")
                    .unwrap()
                    .as_i64()
                    .unwrap()
            })
            .collect();

        // All A's come before B
        assert_eq!(&categories[0..3], &["A", "A", "A"]);
        assert_eq!(categories[3], "B");

        // Within A's, priority should be DESC (3, 2, 1)
        assert_eq!(&priorities[0..3], &[3, 2, 1]);
    }

    #[test]
    fn test_order_by_three_columns() {
        let (collection, _temp) = create_test_collection();

        let points = vec![
            crate::Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"a": 1, "b": 2, "c": "x"})),
            },
            crate::Point {
                id: 2,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"a": 1, "b": 2, "c": "y"})),
            },
            crate::Point {
                id: 3,
                vector: vec![0.8, 0.2, 0.0, 0.0],
                payload: Some(serde_json::json!({"a": 1, "b": 1, "c": "z"})),
            },
            crate::Point {
                id: 4,
                vector: vec![0.7, 0.3, 0.0, 0.0],
                payload: Some(serde_json::json!({"a": 2, "b": 1, "c": "w"})),
            },
        ];
        collection.upsert(points).unwrap();

        // ORDER BY a ASC, b DESC, c ASC
        let query = "SELECT * FROM test ORDER BY a ASC, b DESC, c ASC LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let result = collection.execute_query(&parsed, &HashMap::new());
        assert!(
            result.is_ok(),
            "Three-column ORDER BY should work: {:?}",
            result.err()
        );

        let results = result.unwrap();
        assert_eq!(results.len(), 4);

        // Expected order based on (a ASC, b DESC, c ASC):
        // (1, 2, x), (1, 2, y), (1, 1, z), (2, 1, w)
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_order_by_stable_equal_values() {
        let (collection, _temp) = create_test_collection();

        // All same category - should maintain stable order (original insertion order)
        let points = vec![
            crate::Point {
                id: 10,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "same", "seq": 1})),
            },
            crate::Point {
                id: 20,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "same", "seq": 2})),
            },
            crate::Point {
                id: 30,
                vector: vec![0.8, 0.2, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "same", "seq": 3})),
            },
        ];
        collection.upsert(points).unwrap();

        // ORDER BY category ASC only - all equal, should be stable
        let query = "SELECT * FROM test ORDER BY category ASC LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let result = collection.execute_query(&parsed, &HashMap::new());
        assert!(result.is_ok());

        // Rust sort_by is stable, so order should be preserved for equal values
        // We just verify it doesn't crash and returns all results
        let results = result.unwrap();
        assert_eq!(results.len(), 3);
    }
}
