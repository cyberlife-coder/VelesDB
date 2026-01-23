//! Integration tests for similarity() function execution.

#[cfg(test)]
mod tests {
    use crate::collection::types::Collection;
    use crate::distance::DistanceMetric;
    use crate::velesql::Parser;
    use crate::Point;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn create_test_collection_with_data() -> (Collection, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = PathBuf::from(temp_dir.path());
        let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();

        // Insert points with different vectors for similarity testing
        let points = vec![
            Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"name": "point_1"})),
            },
            Point {
                id: 2,
                vector: vec![0.0, 1.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"name": "point_2"})),
            },
            Point {
                id: 3,
                vector: vec![0.7, 0.7, 0.0, 0.0],
                payload: Some(serde_json::json!({"name": "point_3"})),
            },
            Point {
                id: 4,
                vector: vec![0.5, 0.5, 0.5, 0.5],
                payload: Some(serde_json::json!({"name": "point_4"})),
            },
            Point {
                id: 5,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"name": "point_5"})),
            },
        ];

        collection.upsert(points).unwrap();
        (collection, temp_dir)
    }

    #[test]
    fn test_similarity_greater_than_threshold() {
        let (collection, _temp) = create_test_collection_with_data();

        // Query: find points with similarity > 0.8 to [1, 0, 0, 0]
        let query = "SELECT * FROM test_similarity WHERE similarity(vector, $v) > 0.8 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // Points 1 (identical), 3, 5 should have high similarity
        assert!(
            !results.is_empty(),
            "Should return results with high similarity"
        );

        // Verify all returned results have score > 0.8
        for r in &results {
            assert!(r.score > 0.8, "Score {} should be > 0.8", r.score);
        }
    }

    #[test]
    fn test_similarity_greater_than_or_equal() {
        let (collection, _temp) = create_test_collection_with_data();

        let query = "SELECT * FROM test_similarity WHERE similarity(vector, $v) >= 0.99 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // Only point 1 (identical) should have similarity >= 0.99
        for r in &results {
            assert!(r.score >= 0.99, "Score {} should be >= 0.99", r.score);
        }
    }

    #[test]
    fn test_similarity_less_than_threshold() {
        let (collection, _temp) = create_test_collection_with_data();

        let query = "SELECT * FROM test_similarity WHERE similarity(vector, $v) < 0.5 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // All returned results should have score < 0.5
        for r in &results {
            assert!(r.score < 0.5, "Score {} should be < 0.5", r.score);
        }
    }

    #[test]
    fn test_similarity_with_literal_vector() {
        let (collection, _temp) = create_test_collection_with_data();

        // Query with literal vector instead of parameter
        let query =
            "SELECT * FROM test_similarity WHERE similarity(vector, [1.0, 0.0, 0.0, 0.0]) > 0.9 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let params = HashMap::new();
        let results = collection.execute_query(&parsed, &params).unwrap();

        // Should find points with high similarity to [1, 0, 0, 0]
        for r in &results {
            assert!(r.score > 0.9, "Score {} should be > 0.9", r.score);
        }
    }

    #[test]
    fn test_similarity_no_results_when_threshold_too_high() {
        let (collection, _temp) = create_test_collection_with_data();

        // No point has similarity > 1.5 (impossible for normalized vectors)
        let query = "SELECT * FROM test_similarity WHERE similarity(vector, $v) > 1.5 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();
        assert!(
            results.is_empty(),
            "Should return no results for impossible threshold"
        );
    }

    #[test]
    fn test_similarity_missing_parameter_error() {
        let (collection, _temp) = create_test_collection_with_data();

        let query =
            "SELECT * FROM test_similarity WHERE similarity(vector, $missing) > 0.8 LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let params = HashMap::new(); // Empty params - $missing not provided
        let result = collection.execute_query(&parsed, &params);

        assert!(result.is_err(), "Should error on missing parameter");
    }

    /// Regression test: similarity() with additional filter conditions.
    ///
    /// Bug: similarity() queries were ignoring additional filter conditions
    /// in the WHERE clause (e.g., `AND category = 'tech'`).
    #[test]
    fn test_similarity_with_metadata_filter_applied() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = PathBuf::from(temp_dir.path());
        let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();

        // Insert points with category metadata
        let points = vec![
            Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "tech", "name": "tech_1"})),
            },
            Point {
                id: 2,
                vector: vec![0.95, 0.05, 0.0, 0.0], // Very similar to query vector
                payload: Some(serde_json::json!({"category": "sports", "name": "sports_1"})),
            },
            Point {
                id: 3,
                vector: vec![0.9, 0.1, 0.0, 0.0], // Also similar
                payload: Some(serde_json::json!({"category": "tech", "name": "tech_2"})),
            },
            Point {
                id: 4,
                vector: vec![0.5, 0.5, 0.5, 0.5], // Less similar
                payload: Some(serde_json::json!({"category": "tech", "name": "tech_3"})),
            },
        ];

        collection.upsert(points).unwrap();

        // Query: similarity > 0.8 AND category = 'tech'
        // Should only return tech items with high similarity (id=1, id=3)
        // Should NOT return id=2 (sports) even though it has high similarity
        let query =
            "SELECT * FROM test WHERE similarity(vector, $v) > 0.8 AND category = 'tech' LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // All results should be category = 'tech'
        for result in &results {
            let payload = result.point.payload.as_ref().expect("Should have payload");
            let category = payload.get("category").and_then(|v| v.as_str());
            assert_eq!(
                category,
                Some("tech"),
                "All results should have category='tech', but got {:?} for id={}",
                category,
                result.point.id
            );
        }

        // id=2 (sports) should NOT be in results even though it has high similarity
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert!(
            !ids.contains(&2),
            "id=2 (sports) should be filtered out, but got ids: {:?}",
            ids
        );
    }

    // =========================================================================
    // ORDER BY similarity() Tests (EPIC-008 US-008)
    // =========================================================================

    #[test]
    fn test_order_by_similarity_desc() {
        let (collection, _temp_dir) = create_test_collection_with_data();

        // Query with ORDER BY similarity DESC (highest first)
        let query = "SELECT * FROM test ORDER BY similarity(vector, $v) DESC LIMIT 5";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // Verify results are sorted by similarity DESC
        assert!(!results.is_empty(), "Should return results");

        for i in 1..results.len() {
            assert!(
                results[i - 1].score >= results[i].score,
                "Results should be sorted DESC: {} >= {} at position {}",
                results[i - 1].score,
                results[i].score,
                i
            );
        }

        // First result should be the most similar (point 1 with [1,0,0,0])
        assert_eq!(results[0].point.id, 1, "Most similar point should be first");
    }

    #[test]
    fn test_order_by_similarity_asc() {
        let (collection, _temp_dir) = create_test_collection_with_data();

        // Query with ORDER BY similarity ASC (lowest first)
        let query = "SELECT * FROM test ORDER BY similarity(vector, $v) ASC LIMIT 5";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // Verify results are sorted by similarity ASC
        assert!(!results.is_empty(), "Should return results");

        for i in 1..results.len() {
            assert!(
                results[i - 1].score <= results[i].score,
                "Results should be sorted ASC: {} <= {} at position {}",
                results[i - 1].score,
                results[i].score,
                i
            );
        }
    }

    #[test]
    fn test_order_by_similarity_with_filter() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = PathBuf::from(temp_dir.path());
        let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();

        // Insert points with category metadata
        let points = vec![
            Point {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "tech", "name": "tech_1"})),
            },
            Point {
                id: 2,
                vector: vec![0.9, 0.1, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "tech", "name": "tech_2"})),
            },
            Point {
                id: 3,
                vector: vec![0.8, 0.2, 0.0, 0.0],
                payload: Some(serde_json::json!({"category": "sports", "name": "sports_1"})),
            },
        ];
        collection.upsert(points).unwrap();

        // Query with WHERE filter AND ORDER BY similarity
        let query =
            "SELECT * FROM test WHERE category = 'tech' ORDER BY similarity(vector, $v) DESC LIMIT 10";
        let parsed = Parser::parse(query).unwrap();

        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // All results should be category = 'tech' AND sorted by similarity
        for result in &results {
            let payload = result.point.payload.as_ref().expect("Should have payload");
            let category = payload
                .get("category")
                .and_then(|v: &serde_json::Value| v.as_str());
            assert_eq!(
                category,
                Some("tech"),
                "All results should have category='tech'"
            );
        }

        // Verify sorted DESC
        for i in 1..results.len() {
            assert!(
                results[i - 1].score >= results[i].score,
                "Results should be sorted DESC"
            );
        }
    }
}
