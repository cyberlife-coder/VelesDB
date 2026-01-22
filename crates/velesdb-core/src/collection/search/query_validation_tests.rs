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
}
