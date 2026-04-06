//! Tests for index_management.rs (EPIC-041 US-001)

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

    // =========================================================================
    // Bitmap pre-filter tests
    // =========================================================================

    #[test]
    fn test_build_prefilter_bitmap_returns_none_without_index() {
        let (collection, _temp) = create_test_collection();

        // Filter on a non-indexed field
        let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
            field: "category".to_string(),
            value: serde_json::Value::String("tech".to_string()),
        });

        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_none(), "no secondary index => no bitmap");
    }

    #[test]
    fn test_build_prefilter_bitmap_returns_bitmap_with_index() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        // Manually populate the secondary index
        {
            let indexes = collection.secondary_indexes.read();
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("tech".to_string()),
                    vec![1, 5, 10],
                );
            }
        }

        let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
            field: "category".to_string(),
            value: serde_json::Value::String("tech".to_string()),
        });

        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_some(), "indexed field should produce a bitmap");
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(1));
        assert!(bm.contains(5));
        assert!(bm.contains(10));
    }

    #[test]
    fn test_build_prefilter_bitmap_and_intersection() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        collection
            .create_index("status")
            .expect("test: index creation");

        // Populate both indexes
        {
            let indexes = collection.secondary_indexes.read();
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("tech".to_string()),
                    vec![1, 5, 10, 20],
                );
            }
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("status") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("active".to_string()),
                    vec![5, 10, 30],
                );
            }
        }

        // AND condition: category = "tech" AND status = "active"
        let filter = crate::filter::Filter::new(crate::filter::Condition::And {
            conditions: vec![
                crate::filter::Condition::Eq {
                    field: "category".to_string(),
                    value: serde_json::Value::String("tech".to_string()),
                },
                crate::filter::Condition::Eq {
                    field: "status".to_string(),
                    value: serde_json::Value::String("active".to_string()),
                },
            ],
        });

        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(
            bitmap.is_some(),
            "AND of indexed fields should produce bitmap"
        );
        let bm = bitmap.unwrap();
        // Intersection of {1,5,10,20} & {5,10,30} = {5,10}
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(5));
        assert!(bm.contains(10));
    }

    // =========================================================================
    // Range pre-filter tests (Gt, Gte, Lt, Lte, Between)
    // =========================================================================

    /// Populates a "price" secondary index with values 10, 20, 30, 40, 50
    /// mapped to point IDs 1, 2, 3, 4, 5 respectively.
    fn populate_price_index(collection: &Collection) {
        use crate::index::secondary::F64Key;
        let indexes = collection.secondary_indexes.read();
        if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("price") {
            let mut t = tree.write();
            for (price, id) in [(10, 1u64), (20, 2), (30, 3), (40, 4), (50, 5)] {
                t.insert(
                    crate::index::JsonValue::Number(F64Key::from(f64::from(price))),
                    vec![id],
                );
            }
        }
    }

    #[test]
    fn test_build_prefilter_bitmap_gt() {
        // GIVEN: collection with secondary index on "price", values 10..50
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("price")
            .expect("test: index creation");
        populate_price_index(&collection);

        // WHEN: build_prefilter_bitmap for price > 30
        let filter = crate::filter::Filter::new(crate::filter::Condition::Gt {
            field: "price".to_string(),
            value: serde_json::json!(30),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: bitmap contains IDs with price 40, 50
        assert!(
            bitmap.is_some(),
            "Gt on indexed field should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(4));
        assert!(bm.contains(5));
    }

    #[test]
    fn test_build_prefilter_bitmap_gte() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("price")
            .expect("test: index creation");
        populate_price_index(&collection);

        // price >= 30 => IDs 3, 4, 5
        let filter = crate::filter::Filter::new(crate::filter::Condition::Gte {
            field: "price".to_string(),
            value: serde_json::json!(30),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        assert!(
            bitmap.is_some(),
            "Gte on indexed field should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(3));
        assert!(bm.contains(4));
        assert!(bm.contains(5));
    }

    #[test]
    fn test_build_prefilter_bitmap_lt() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("price")
            .expect("test: index creation");
        populate_price_index(&collection);

        // price < 30 => IDs 1, 2
        let filter = crate::filter::Filter::new(crate::filter::Condition::Lt {
            field: "price".to_string(),
            value: serde_json::json!(30),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        assert!(
            bitmap.is_some(),
            "Lt on indexed field should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
    }

    #[test]
    fn test_build_prefilter_bitmap_lte() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("price")
            .expect("test: index creation");
        populate_price_index(&collection);

        // price <= 30 => IDs 1, 2, 3
        let filter = crate::filter::Filter::new(crate::filter::Condition::Lte {
            field: "price".to_string(),
            value: serde_json::json!(30),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        assert!(
            bitmap.is_some(),
            "Lte on indexed field should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
        assert!(bm.contains(3));
    }

    #[test]
    fn test_build_prefilter_bitmap_between_via_and() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("price")
            .expect("test: index creation");
        populate_price_index(&collection);

        // BETWEEN 20 AND 40 => AND(Gte(20), Lte(40)) => IDs 2, 3, 4
        let filter = crate::filter::Filter::new(crate::filter::Condition::And {
            conditions: vec![
                crate::filter::Condition::Gte {
                    field: "price".to_string(),
                    value: serde_json::json!(20),
                },
                crate::filter::Condition::Lte {
                    field: "price".to_string(),
                    value: serde_json::json!(40),
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        assert!(
            bitmap.is_some(),
            "BETWEEN on indexed field should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(2));
        assert!(bm.contains(3));
        assert!(bm.contains(4));
    }

    #[test]
    fn test_build_prefilter_bitmap_range_no_index() {
        let (collection, _temp) = create_test_collection();

        // No index on "price" => None fallback
        let filter = crate::filter::Filter::new(crate::filter::Condition::Gt {
            field: "price".to_string(),
            value: serde_json::json!(100),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_none(), "no index => None for range condition");
    }

    #[test]
    fn test_build_prefilter_bitmap_range_empty_result() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("price")
            .expect("test: index creation");
        populate_price_index(&collection);

        // price > 999 => no matches => empty bitmap
        let filter = crate::filter::Filter::new(crate::filter::Condition::Gt {
            field: "price".to_string(),
            value: serde_json::json!(999),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_some(), "indexed field should produce bitmap");
        let bm = bitmap.unwrap();
        assert!(bm.is_empty(), "no matches => empty bitmap");
    }

    // =========================================================================
    // OR pre-filter tests
    // =========================================================================

    #[test]
    fn test_build_prefilter_bitmap_or_union() {
        // GIVEN: collection with index on "category"
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        {
            let indexes = collection.secondary_indexes.read();
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("tech".to_string()),
                    vec![1, 5],
                );
                t.insert(
                    crate::index::JsonValue::String("science".to_string()),
                    vec![3, 7],
                );
            }
        }

        // WHEN: build_prefilter_bitmap for category='tech' OR category='science'
        let filter = crate::filter::Filter::new(crate::filter::Condition::Or {
            conditions: vec![
                crate::filter::Condition::Eq {
                    field: "category".to_string(),
                    value: serde_json::Value::String("tech".to_string()),
                },
                crate::filter::Condition::Eq {
                    field: "category".to_string(),
                    value: serde_json::Value::String("science".to_string()),
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: bitmap is union of both sets
        assert!(
            bitmap.is_some(),
            "OR of indexed fields should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 4);
        assert!(bm.contains(1));
        assert!(bm.contains(3));
        assert!(bm.contains(5));
        assert!(bm.contains(7));
    }

    #[test]
    fn test_build_prefilter_bitmap_or_with_non_indexed_child() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        {
            let indexes = collection.secondary_indexes.read();
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("tech".to_string()),
                    vec![1, 5],
                );
            }
        }

        // OR where one child has no index => entire OR returns None
        let filter = crate::filter::Filter::new(crate::filter::Condition::Or {
            conditions: vec![
                crate::filter::Condition::Eq {
                    field: "category".to_string(),
                    value: serde_json::Value::String("tech".to_string()),
                },
                crate::filter::Condition::Eq {
                    field: "unindexed_field".to_string(),
                    value: serde_json::Value::String("value".to_string()),
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(
            bitmap.is_none(),
            "OR with non-indexed child must return None"
        );
    }

    // =========================================================================
    // NOT / Neq pre-filter tests (documented: return None)
    // =========================================================================

    #[test]
    fn test_build_prefilter_bitmap_not_returns_none() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        let filter = crate::filter::Filter::new(crate::filter::Condition::Not {
            condition: Box::new(crate::filter::Condition::Eq {
                field: "category".to_string(),
                value: serde_json::Value::String("tech".to_string()),
            }),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_none(), "NOT returns None (needs universe bitmap)");
    }

    #[test]
    fn test_build_prefilter_bitmap_neq_uses_universe_subtraction() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        // Manually populate: 3 tech (IDs 1,5,10) + 2 science (IDs 2,7)
        {
            let indexes = collection.secondary_indexes.read();
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("tech".to_string()),
                    vec![1, 5, 10],
                );
                t.insert(
                    crate::index::JsonValue::String("science".to_string()),
                    vec![2, 7],
                );
            }
        }

        let filter = crate::filter::Filter::new(crate::filter::Condition::Neq {
            field: "category".to_string(),
            value: serde_json::Value::String("tech".to_string()),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_some(), "Neq should return Some (universe - eq)");
        let bm = bitmap.unwrap();
        // Universe = {1,2,5,7,10}, eq("tech") = {1,5,10}, NEQ = {2,7}
        assert_eq!(bm.len(), 2, "Neq should exclude tech points");
        assert!(bm.contains(2));
        assert!(bm.contains(7));
    }

    // =========================================================================
    // Mixed compound tests (AND + OR + range)
    // =========================================================================

    #[test]
    fn test_build_prefilter_bitmap_and_with_range_and_eq() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("price")
            .expect("test: index creation");
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_price_index(&collection);

        {
            let indexes = collection.secondary_indexes.read();
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("tech".to_string()),
                    vec![1, 2, 3, 4, 5],
                );
            }
        }

        // AND(price > 20, category = 'tech') => intersection of {3,4,5} & {1,2,3,4,5} = {3,4,5}
        let filter = crate::filter::Filter::new(crate::filter::Condition::And {
            conditions: vec![
                crate::filter::Condition::Gt {
                    field: "price".to_string(),
                    value: serde_json::json!(20),
                },
                crate::filter::Condition::Eq {
                    field: "category".to_string(),
                    value: serde_json::Value::String("tech".to_string()),
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        assert!(
            bitmap.is_some(),
            "AND of indexed range+eq should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(3));
        assert!(bm.contains(4));
        assert!(bm.contains(5));
    }

    #[test]
    fn test_build_prefilter_bitmap_empty_for_missing_value() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        // Index exists but the value "nonexistent" has no entries
        let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
            field: "category".to_string(),
            value: serde_json::Value::String("nonexistent".to_string()),
        });

        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_some(), "indexed field should produce a bitmap");
        let bm = bitmap.unwrap();
        assert!(bm.is_empty(), "no entries => empty bitmap");
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
        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("IndexInfo"));
    }

    // =========================================================================
    // IN bitmap pre-filter tests (Issue #512)
    // =========================================================================

    /// Helper: populates a "category" secondary index with tech=[1,5,10],
    /// science=[2,7], art=[3].
    fn populate_category_index(collection: &Collection) {
        let indexes = collection.secondary_indexes.read();
        if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
            let mut t = tree.write();
            t.insert(
                crate::index::JsonValue::String("tech".to_string()),
                vec![1, 5, 10],
            );
            t.insert(
                crate::index::JsonValue::String("science".to_string()),
                vec![2, 7],
            );
            t.insert(crate::index::JsonValue::String("art".to_string()), vec![3]);
        }
    }

    #[test]
    fn test_in_bitmap_empty_list_returns_empty() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        let filter = crate::filter::Filter::new(crate::filter::Condition::In {
            field: "category".to_string(),
            values: vec![],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_some(), "empty IN list should return Some(empty)");
        assert!(bitmap.unwrap().is_empty(), "empty IN list => empty bitmap");
    }

    #[test]
    fn test_in_bitmap_no_index_returns_none() {
        let (collection, _temp) = create_test_collection();

        // No index on "category"
        let filter = crate::filter::Filter::new(crate::filter::Condition::In {
            field: "category".to_string(),
            values: vec![serde_json::Value::String("tech".to_string())],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_none(), "no secondary index => None");
    }

    #[test]
    fn test_in_bitmap_nonexistent_values_skipped() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // IN list with values not in the index
        let filter = crate::filter::Filter::new(crate::filter::Condition::In {
            field: "category".to_string(),
            values: vec![
                serde_json::Value::String("nonexistent".to_string()),
                serde_json::Value::String("also_missing".to_string()),
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(
            bitmap.is_some(),
            "indexed field should produce bitmap even with missing values"
        );
        assert!(
            bitmap.unwrap().is_empty(),
            "nonexistent values => empty bitmap"
        );
    }

    #[test]
    fn test_in_bitmap_single_value_matches_eq() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // IN with single value should match Eq bitmap
        let in_filter = crate::filter::Filter::new(crate::filter::Condition::In {
            field: "category".to_string(),
            values: vec![serde_json::Value::String("tech".to_string())],
        });
        let eq_filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
            field: "category".to_string(),
            value: serde_json::Value::String("tech".to_string()),
        });

        let in_bm = collection.build_prefilter_bitmap(&in_filter).unwrap();
        let eq_bm = collection.build_prefilter_bitmap(&eq_filter).unwrap();
        assert_eq!(in_bm, eq_bm, "IN(single) should equal Eq bitmap");
    }

    #[test]
    fn test_in_bitmap_multiple_values_union() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // IN('tech', 'science') => union of {1,5,10} | {2,7} = {1,2,5,7,10}
        let filter = crate::filter::Filter::new(crate::filter::Condition::In {
            field: "category".to_string(),
            values: vec![
                serde_json::Value::String("tech".to_string()),
                serde_json::Value::String("science".to_string()),
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(
            bitmap.is_some(),
            "IN on indexed field should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 5);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
        assert!(bm.contains(5));
        assert!(bm.contains(7));
        assert!(bm.contains(10));
    }

    #[test]
    fn test_in_bitmap_mixed_existing_missing_values() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // IN('tech', 'nonexistent', 'art') => union of {1,5,10} | {} | {3}
        let filter = crate::filter::Filter::new(crate::filter::Condition::In {
            field: "category".to_string(),
            values: vec![
                serde_json::Value::String("tech".to_string()),
                serde_json::Value::String("nonexistent".to_string()),
                serde_json::Value::String("art".to_string()),
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_some());
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 4);
        assert!(bm.contains(1));
        assert!(bm.contains(3));
        assert!(bm.contains(5));
        assert!(bm.contains(10));
    }

    #[test]
    fn test_in_bitmap_with_u64_overflow_ids() {
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");

        let large_id = u64::from(u32::MAX) + 1;
        {
            let indexes = collection.secondary_indexes.read();
            if let Some(crate::index::SecondaryIndex::BTree(tree)) = indexes.get("category") {
                let mut t = tree.write();
                t.insert(
                    crate::index::JsonValue::String("tech".to_string()),
                    vec![1, large_id, 5],
                );
            }
        }

        let filter = crate::filter::Filter::new(crate::filter::Condition::In {
            field: "category".to_string(),
            values: vec![serde_json::Value::String("tech".to_string())],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_some());
        let bm = bitmap.unwrap();
        // large_id should be silently skipped
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(1));
        assert!(bm.contains(5));
    }

    // =========================================================================
    // NOT IN bitmap pre-filter tests (Issue #512 — Requirement 2)
    // =========================================================================

    #[test]
    fn test_not_in_bitmap_returns_universe_minus_in() {
        // GIVEN: index with tech=[1,5,10], science=[2,7], art=[3]
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // WHEN: NOT IN ('tech', 'science')
        let filter = crate::filter::Filter::new(crate::filter::Condition::Not {
            condition: Box::new(crate::filter::Condition::In {
                field: "category".to_string(),
                values: vec![
                    serde_json::Value::String("tech".to_string()),
                    serde_json::Value::String("science".to_string()),
                ],
            }),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: universe={1,2,3,5,7,10}, IN={1,2,5,7,10}, NOT IN={3}
        assert!(
            bitmap.is_some(),
            "NOT IN on indexed field should produce bitmap"
        );
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 1);
        assert!(bm.contains(3), "only art ID=3 should remain");
    }

    #[test]
    fn test_not_in_bitmap_no_index_returns_none() {
        let (collection, _temp) = create_test_collection();

        // No index on "category"
        let filter = crate::filter::Filter::new(crate::filter::Condition::Not {
            condition: Box::new(crate::filter::Condition::In {
                field: "category".to_string(),
                values: vec![serde_json::Value::String("tech".to_string())],
            }),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_none(), "no secondary index => None");
    }

    #[test]
    fn test_not_in_empty_list_returns_universe() {
        // GIVEN: index with tech=[1,5,10], science=[2,7], art=[3]
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // WHEN: NOT IN () — empty exclusion list
        let filter = crate::filter::Filter::new(crate::filter::Condition::Not {
            condition: Box::new(crate::filter::Condition::In {
                field: "category".to_string(),
                values: vec![],
            }),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: universe - empty = full universe {1,2,3,5,7,10}
        assert!(bitmap.is_some(), "NOT IN () should return full universe");
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 6);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
        assert!(bm.contains(3));
        assert!(bm.contains(5));
        assert!(bm.contains(7));
        assert!(bm.contains(10));
    }

    #[test]
    fn test_not_in_all_values_returns_empty() {
        // GIVEN: index with tech=[1,5,10], science=[2,7], art=[3]
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // WHEN: NOT IN (all values) — exclude everything
        let filter = crate::filter::Filter::new(crate::filter::Condition::Not {
            condition: Box::new(crate::filter::Condition::In {
                field: "category".to_string(),
                values: vec![
                    serde_json::Value::String("tech".to_string()),
                    serde_json::Value::String("science".to_string()),
                    serde_json::Value::String("art".to_string()),
                ],
            }),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: universe - universe = empty
        assert!(bitmap.is_some(), "NOT IN (all) should return Some(empty)");
        assert!(
            bitmap.unwrap().is_empty(),
            "excluding all values => empty bitmap"
        );
    }

    #[test]
    fn test_not_wrapping_non_in_returns_none() {
        // Not { Eq } should still return None (only Not { In } is supported)
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        let filter = crate::filter::Filter::new(crate::filter::Condition::Not {
            condition: Box::new(crate::filter::Condition::Eq {
                field: "category".to_string(),
                value: serde_json::Value::String("tech".to_string()),
            }),
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);
        assert!(bitmap.is_none(), "Not wrapping Eq should return None");
    }

    // =========================================================================
    // AND/OR composition with IN bitmaps (Issue #512 — Requirement 4)
    // =========================================================================

    #[test]
    fn test_and_with_in_and_eq_intersects() {
        // GIVEN: category index with tech=[1,5,10], science=[2,7], art=[3]
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // WHEN: And(In('tech','science'), Eq('art'))
        // IN('tech','science') = {1,5,10} | {2,7} = {1,2,5,7,10}
        // Eq('art') = {3}
        // Intersection = empty (no overlap)
        let filter = crate::filter::Filter::new(crate::filter::Condition::And {
            conditions: vec![
                crate::filter::Condition::In {
                    field: "category".to_string(),
                    values: vec![
                        serde_json::Value::String("tech".to_string()),
                        serde_json::Value::String("science".to_string()),
                    ],
                },
                crate::filter::Condition::Eq {
                    field: "category".to_string(),
                    value: serde_json::Value::String("art".to_string()),
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: intersection is empty — no ID is in both sets
        assert!(bitmap.is_some(), "AND(In, Eq) on indexed field => Some");
        assert!(
            bitmap.unwrap().is_empty(),
            "no overlap between IN and Eq => empty bitmap"
        );
    }

    #[test]
    fn test_and_with_in_and_range_intersects() {
        // GIVEN: category index + price index
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        collection
            .create_index("price")
            .expect("test: index creation");
        populate_category_index(&collection);
        populate_price_index(&collection);

        // WHEN: And(In('tech','science'), Gte(price, 5))
        // IN('tech','science') on category = {1,2,5,7,10}
        // Gte(price, 5) => price index has 10→1, 20→2, 30→3, 40→4, 50→5
        //   all prices >= 5, so Gte(5) = {1,2,3,4,5}
        // Intersection = {1,2,5}
        let filter = crate::filter::Filter::new(crate::filter::Condition::And {
            conditions: vec![
                crate::filter::Condition::In {
                    field: "category".to_string(),
                    values: vec![
                        serde_json::Value::String("tech".to_string()),
                        serde_json::Value::String("science".to_string()),
                    ],
                },
                crate::filter::Condition::Gte {
                    field: "price".to_string(),
                    value: serde_json::json!(5),
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: intersection of {1,2,5,7,10} & {1,2,3,4,5} = {1,2,5}
        assert!(bitmap.is_some(), "AND(In, Gte) on indexed fields => Some");
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
        assert!(bm.contains(5));
    }

    #[test]
    fn test_or_with_in_and_eq_unions() {
        // GIVEN: category index with tech=[1,5,10], science=[2,7], art=[3]
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // WHEN: Or(In('tech'), Eq('science'))
        // IN('tech') = {1,5,10}
        // Eq('science') = {2,7}
        // Union = {1,2,5,7,10}
        let filter = crate::filter::Filter::new(crate::filter::Condition::Or {
            conditions: vec![
                crate::filter::Condition::In {
                    field: "category".to_string(),
                    values: vec![serde_json::Value::String("tech".to_string())],
                },
                crate::filter::Condition::Eq {
                    field: "category".to_string(),
                    value: serde_json::Value::String("science".to_string()),
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: union of {1,5,10} | {2,7} = {1,2,5,7,10}
        assert!(bitmap.is_some(), "OR(In, Eq) on indexed field => Some");
        let bm = bitmap.unwrap();
        assert_eq!(bm.len(), 5);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
        assert!(bm.contains(5));
        assert!(bm.contains(7));
        assert!(bm.contains(10));
    }

    #[test]
    fn test_or_with_in_unindexed_returns_none() {
        // GIVEN: category index exists, but "tags" has no index
        let (collection, _temp) = create_test_collection();
        collection
            .create_index("category")
            .expect("test: index creation");
        populate_category_index(&collection);

        // WHEN: Or(In(category, ['tech']), In(tags, ['rust']))
        // category IN is indexed => Some, but tags IN is unindexed => None
        // OR requires ALL children to resolve => entire OR returns None
        let filter = crate::filter::Filter::new(crate::filter::Condition::Or {
            conditions: vec![
                crate::filter::Condition::In {
                    field: "category".to_string(),
                    values: vec![serde_json::Value::String("tech".to_string())],
                },
                crate::filter::Condition::In {
                    field: "tags".to_string(),
                    values: vec![serde_json::Value::String("rust".to_string())],
                },
            ],
        });
        let bitmap = collection.build_prefilter_bitmap(&filter);

        // THEN: None because OR with unindexed child cannot produce complete bitmap
        assert!(
            bitmap.is_none(),
            "OR with unindexed IN child must return None"
        );
    }
}
