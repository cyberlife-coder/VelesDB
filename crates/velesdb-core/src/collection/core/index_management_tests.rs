//! Tests for `index_management.rs` (EPIC-041 US-001)

#[cfg(test)]
mod tests {
    use crate::collection::types::Collection;
    use crate::DistanceMetric;
    use tempfile::TempDir;

    fn create_test_collection() -> (Collection, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let collection =
            Collection::create(temp_dir.path().to_path_buf(), 128, DistanceMetric::Cosine)
                .expect("Failed to create collection");
        (collection, temp_dir)
    }

    #[test]
    fn test_create_property_index_success() {
        let (collection, _temp) = create_test_collection();

        let result = collection.create_property_index("Person", "email");
        assert!(result.is_ok());

        // Verify index exists
        assert!(collection.has_property_index("Person", "email"));
    }

    #[test]
    fn test_create_property_index_idempotent() {
        let (collection, _temp) = create_test_collection();

        // Create same index twice
        collection.create_property_index("Person", "email").unwrap();
        let result = collection.create_property_index("Person", "email");

        // Should succeed (idempotent)
        assert!(result.is_ok());
        assert!(collection.has_property_index("Person", "email"));
    }

    #[test]
    fn test_create_range_index_success() {
        let (collection, _temp) = create_test_collection();

        let result = collection.create_range_index("Event", "timestamp");
        assert!(result.is_ok());

        // Verify index exists
        assert!(collection.has_range_index("Event", "timestamp"));
    }

    #[test]
    fn test_create_range_index_idempotent() {
        let (collection, _temp) = create_test_collection();

        // Create same index twice
        collection.create_range_index("Event", "timestamp").unwrap();
        let result = collection.create_range_index("Event", "timestamp");

        // Should succeed (idempotent)
        assert!(result.is_ok());
        assert!(collection.has_range_index("Event", "timestamp"));
    }

    #[test]
    fn test_has_property_index_false_when_not_exists() {
        let (collection, _temp) = create_test_collection();

        assert!(!collection.has_property_index("NonExistent", "field"));
    }

    #[test]
    fn test_has_range_index_false_when_not_exists() {
        let (collection, _temp) = create_test_collection();

        assert!(!collection.has_range_index("NonExistent", "field"));
    }

    #[test]
    fn test_list_indexes_empty_initially() {
        let (collection, _temp) = create_test_collection();

        let indexes = collection.list_indexes();
        assert!(indexes.is_empty());
    }

    #[test]
    fn test_list_indexes_with_property_index() {
        let (collection, _temp) = create_test_collection();

        collection.create_property_index("Person", "email").unwrap();

        let indexes = collection.list_indexes();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].label, "Person");
        assert_eq!(indexes[0].property, "email");
        assert_eq!(indexes[0].index_type, "hash");
    }

    #[test]
    fn test_list_indexes_with_range_index() {
        let (collection, _temp) = create_test_collection();

        collection.create_range_index("Event", "timestamp").unwrap();

        let indexes = collection.list_indexes();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].label, "Event");
        assert_eq!(indexes[0].property, "timestamp");
        assert_eq!(indexes[0].index_type, "range");
    }

    #[test]
    fn test_list_indexes_mixed() {
        let (collection, _temp) = create_test_collection();

        collection.create_property_index("Person", "email").unwrap();
        collection.create_range_index("Event", "timestamp").unwrap();

        let indexes = collection.list_indexes();
        assert_eq!(indexes.len(), 2);

        // Check both index types are present
        let has_hash = indexes.iter().any(|i| i.index_type == "hash");
        let has_range = indexes.iter().any(|i| i.index_type == "range");
        assert!(has_hash);
        assert!(has_range);
    }

    #[test]
    fn test_drop_index_property_success() {
        let (collection, _temp) = create_test_collection();

        collection.create_property_index("Person", "email").unwrap();
        assert!(collection.has_property_index("Person", "email"));

        let result = collection.drop_index("Person", "email");
        assert!(result.is_ok());
        assert!(result.unwrap()); // Returns true when dropped

        assert!(!collection.has_property_index("Person", "email"));
    }

    #[test]
    fn test_drop_index_range_success() {
        let (collection, _temp) = create_test_collection();

        collection.create_range_index("Event", "timestamp").unwrap();
        assert!(collection.has_range_index("Event", "timestamp"));

        let result = collection.drop_index("Event", "timestamp");
        assert!(result.is_ok());
        assert!(result.unwrap()); // Returns true when dropped

        assert!(!collection.has_range_index("Event", "timestamp"));
    }

    #[test]
    fn test_drop_index_returns_false_when_not_exists() {
        let (collection, _temp) = create_test_collection();

        let result = collection.drop_index("NonExistent", "field");
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Returns false when no index existed
    }

    #[test]
    fn test_indexes_memory_usage_initial() {
        let (collection, _temp) = create_test_collection();

        // Memory usage should be minimal initially
        let memory = collection.indexes_memory_usage();
        // Memory usage returns usize, just verify it doesn't panic
        let _ = memory;
    }

    #[test]
    fn test_indexes_memory_usage_after_creation() {
        let (collection, _temp) = create_test_collection();

        let initial_memory = collection.indexes_memory_usage();

        collection.create_property_index("Person", "email").unwrap();
        collection.create_range_index("Event", "timestamp").unwrap();

        let after_memory = collection.indexes_memory_usage();
        // Memory should be at least the same (could be more with index structures)
        assert!(after_memory >= initial_memory);
    }

    #[test]
    fn test_index_info_struct() {
        use crate::collection::core::index_management::IndexInfo;

        let info = IndexInfo {
            label: "Test".to_string(),
            property: "field".to_string(),
            index_type: "hash".to_string(),
            cardinality: 100,
            memory_bytes: 1024,
        };

        assert_eq!(info.label, "Test");
        assert_eq!(info.property, "field");
        assert_eq!(info.index_type, "hash");
        assert_eq!(info.cardinality, 100);
        assert_eq!(info.memory_bytes, 1024);

        // Test Clone
        let cloned = info.clone();
        assert_eq!(cloned.label, info.label);

        // Test Debug
        let debug_str = format!("{info:?}");
        assert!(debug_str.contains("IndexInfo"));
    }
}
