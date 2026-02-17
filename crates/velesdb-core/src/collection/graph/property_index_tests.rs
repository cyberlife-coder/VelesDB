//! Tests for `PropertyIndex` and `CompositeGraphIndex`.

use super::property_index::{
    CompositeGraphIndex, CompositeIndexManager, CompositeIndexType, CompositeRangeIndex,
    EdgePropertyIndex, IndexAdvisor, IndexIntersection, PredicateType, PropertyIndex, QueryPattern,
    QueryPatternTracker,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_create_property_index() {
    let mut index = PropertyIndex::new();

    assert!(!index.has_index("Person", "email"));

    index.create_index("Person", "email");

    assert!(index.has_index("Person", "email"));
    assert!(!index.has_index("Person", "name"));
    assert!(!index.has_index("Company", "email"));
}

#[test]
fn test_insert_and_lookup() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");

    // Insert a value
    let inserted = index.insert("Person", "email", &json!("alice@example.com"), 1);
    assert!(inserted);

    // Lookup should find it
    let result = index.lookup("Person", "email", &json!("alice@example.com"));
    assert!(result.is_some());
    assert!(result.unwrap().contains(1));

    // Lookup different value should not find it
    let result2 = index.lookup("Person", "email", &json!("bob@example.com"));
    assert!(result2.is_none());
}

#[test]
fn test_lookup_unindexed_returns_none() {
    let index = PropertyIndex::new();

    // No index created - lookup should return None
    let result = index.lookup("Person", "email", &json!("alice@example.com"));
    assert!(result.is_none());
}

#[test]
fn test_insert_unindexed_returns_false() {
    let mut index = PropertyIndex::new();

    // No index created - insert should return false
    let inserted = index.insert("Person", "email", &json!("alice@example.com"), 1);
    assert!(!inserted);
}

#[test]
fn test_multiple_nodes_same_value() {
    let mut index = PropertyIndex::new();
    index.create_index("Document", "category");

    // Insert multiple nodes with same category
    index.insert("Document", "category", &json!("tech"), 1);
    index.insert("Document", "category", &json!("tech"), 2);
    index.insert("Document", "category", &json!("tech"), 3);
    index.insert("Document", "category", &json!("science"), 10);

    // Should find all tech documents
    let tech_docs = index
        .lookup("Document", "category", &json!("tech"))
        .unwrap();
    assert_eq!(tech_docs.len(), 3);
    assert!(tech_docs.contains(1));
    assert!(tech_docs.contains(2));
    assert!(tech_docs.contains(3));

    // Should find science document
    let science_docs = index
        .lookup("Document", "category", &json!("science"))
        .unwrap();
    assert_eq!(science_docs.len(), 1);
    assert!(science_docs.contains(10));
}

#[test]
fn test_remove_from_index() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");

    index.insert("Person", "email", &json!("alice@example.com"), 1);
    index.insert("Person", "email", &json!("alice@example.com"), 2);

    // Remove one node
    let removed = index.remove("Person", "email", &json!("alice@example.com"), 1);
    assert!(removed);

    // Should still find node 2
    let result = index
        .lookup("Person", "email", &json!("alice@example.com"))
        .unwrap();
    assert!(!result.contains(1));
    assert!(result.contains(2));

    // Remove node 2 - entry should be cleaned up
    index.remove("Person", "email", &json!("alice@example.com"), 2);
    let result2 = index.lookup("Person", "email", &json!("alice@example.com"));
    assert!(result2.is_none());
}

#[test]
fn test_indexed_properties() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");
    index.create_index("Person", "name");
    index.create_index("Company", "domain");

    let props = index.indexed_properties();
    assert_eq!(props.len(), 3);
}

#[test]
fn test_cardinality() {
    let mut index = PropertyIndex::new();
    index.create_index("Document", "category");

    // No values yet
    assert_eq!(index.cardinality("Document", "category"), Some(0));

    // Add values
    index.insert("Document", "category", &json!("tech"), 1);
    index.insert("Document", "category", &json!("science"), 2);
    index.insert("Document", "category", &json!("tech"), 3); // duplicate value

    // Should have 2 unique values
    assert_eq!(index.cardinality("Document", "category"), Some(2));
}

#[test]
fn test_drop_index() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");
    index.insert("Person", "email", &json!("alice@example.com"), 1);

    assert!(index.has_index("Person", "email"));

    let dropped = index.drop_index("Person", "email");
    assert!(dropped);
    assert!(!index.has_index("Person", "email"));

    // Drop non-existent index
    let dropped2 = index.drop_index("Person", "email");
    assert!(!dropped2);
}

#[test]
fn test_numeric_values() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "age");

    index.insert("Person", "age", &json!(25), 1);
    index.insert("Person", "age", &json!(30), 2);
    index.insert("Person", "age", &json!(25), 3);

    let age_25 = index.lookup("Person", "age", &json!(25)).unwrap();
    assert_eq!(age_25.len(), 2);
    assert!(age_25.contains(1));
    assert!(age_25.contains(3));
}

#[test]
fn test_boolean_values() {
    let mut index = PropertyIndex::new();
    index.create_index("User", "active");

    index.insert("User", "active", &json!(true), 1);
    index.insert("User", "active", &json!(false), 2);
    index.insert("User", "active", &json!(true), 3);

    let active = index.lookup("User", "active", &json!(true)).unwrap();
    assert_eq!(active.len(), 2);

    let inactive = index.lookup("User", "active", &json!(false)).unwrap();
    assert_eq!(inactive.len(), 1);
}

#[test]
fn test_memory_usage() {
    let mut index = PropertyIndex::new();
    let initial = index.memory_usage();

    index.create_index("Person", "email");
    index.insert("Person", "email", &json!("alice@example.com"), 1);

    let after = index.memory_usage();
    assert!(after > initial);
}

// =========================================================================
// Edge cases tests (Expert review)
// =========================================================================

#[test]
fn test_null_value_in_index() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "nickname");

    // Null values should be indexable
    index.insert("Person", "nickname", &json!(null), 1);
    index.insert("Person", "nickname", &json!(null), 2);

    let result = index.lookup("Person", "nickname", &json!(null));
    assert!(result.is_some());
    assert_eq!(result.unwrap().len(), 2);
}

#[test]
fn test_empty_string_value() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "middle_name");

    index.insert("Person", "middle_name", &json!(""), 1);

    let result = index.lookup("Person", "middle_name", &json!(""));
    assert!(result.is_some());
    assert!(result.unwrap().contains(1));
}

#[test]
fn test_clear_removes_all_indexes() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");
    index.create_index("Company", "domain");
    index.insert("Person", "email", &json!("test@test.com"), 1);

    index.clear();

    assert!(!index.has_index("Person", "email"));
    assert!(!index.has_index("Company", "domain"));
    assert!(index.indexed_properties().is_empty());
}

// =========================================================================
// Persistence tests (US-005)
// =========================================================================

#[test]
fn test_property_index_serialize_deserialize() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");
    index.create_index("Person", "age");
    index.insert("Person", "email", &json!("alice@example.com"), 1);
    index.insert("Person", "email", &json!("bob@example.com"), 2);
    index.insert("Person", "age", &json!(30), 1);

    // Serialize
    let bytes = index.to_bytes().expect("Serialization failed");
    assert!(!bytes.is_empty());

    // Deserialize
    let loaded = PropertyIndex::from_bytes(&bytes).expect("Deserialization failed");

    // Verify data integrity
    assert!(loaded.has_index("Person", "email"));
    assert!(loaded.has_index("Person", "age"));

    let result = loaded.lookup("Person", "email", &json!("alice@example.com"));
    assert!(result.is_some());
    assert!(result.unwrap().contains(1));

    let result2 = loaded.lookup("Person", "age", &json!(30));
    assert!(result2.is_some());
    assert!(result2.unwrap().contains(1));
}

#[test]
fn test_property_index_persist_to_file() {
    let mut index = PropertyIndex::new();
    index.create_index("Document", "category");
    index.insert("Document", "category", &json!("tech"), 1);
    index.insert("Document", "category", &json!("tech"), 2);
    index.insert("Document", "category", &json!("science"), 3);

    // Save to temp file
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_property_index.bin");

    index.save_to_file(&file_path).expect("Save failed");
    assert!(file_path.exists());

    // Load from file
    let loaded = PropertyIndex::load_from_file(&file_path).expect("Load failed");

    // Verify
    let tech_nodes = loaded.lookup("Document", "category", &json!("tech"));
    assert!(tech_nodes.is_some());
    assert_eq!(tech_nodes.unwrap().len(), 2);

    // Cleanup
    std::fs::remove_file(&file_path).ok();
}

#[test]
fn test_property_index_corrupted_data() {
    let corrupted = vec![0u8, 1, 2, 3, 255, 254];
    let result = PropertyIndex::from_bytes(&corrupted);
    assert!(result.is_err());
}

// =========================================================================
// Collection lifecycle persistence tests (EPIC-009 US-005)
// =========================================================================

#[test]
fn test_property_index_persists_across_collection_reopen() {
    use crate::collection::types::Collection;
    use crate::distance::DistanceMetric;

    let temp_dir = tempfile::tempdir().unwrap();
    let path = std::path::PathBuf::from(temp_dir.path());

    // Create collection and add property index
    {
        let collection = Collection::create(path.clone(), 4, DistanceMetric::Cosine).unwrap();

        // Create a property index
        collection
            .property_index
            .write()
            .create_index("Person", "email");
        collection
            .property_index
            .write()
            .insert("Person", "email", &json!("alice@example.com"), 1);
        collection
            .property_index
            .write()
            .insert("Person", "email", &json!("bob@example.com"), 2);

        // Flush to persist
        collection.flush().unwrap();
    }

    // Reopen collection and verify index is loaded
    {
        let collection = Collection::open(path).unwrap();

        // Verify index exists and data is preserved
        let index = collection.property_index.read();
        assert!(
            index.has_index("Person", "email"),
            "Property index should be loaded from disk"
        );

        let alice_nodes = index.lookup("Person", "email", &json!("alice@example.com"));
        assert!(
            alice_nodes.is_some_and(|b| b.contains(1)),
            "Alice should be in index after reopen"
        );

        let bob_nodes = index.lookup("Person", "email", &json!("bob@example.com"));
        assert!(
            bob_nodes.is_some_and(|b| b.contains(2)),
            "Bob should be in index after reopen"
        );
    }
}

// =========================================================================
// Maintenance hooks tests (US-002)
// =========================================================================

#[test]
fn test_on_add_node_indexes_properties() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");
    index.create_index("Person", "name");

    let mut properties = std::collections::HashMap::new();
    properties.insert("email".to_string(), json!("alice@example.com"));
    properties.insert("name".to_string(), json!("Alice"));
    properties.insert("age".to_string(), json!(30)); // Not indexed

    index.on_add_node("Person", 1, &properties);

    // Indexed properties should be found
    let email_result = index.lookup("Person", "email", &json!("alice@example.com"));
    assert!(email_result.is_some());
    assert!(email_result.unwrap().contains(1));

    let name_result = index.lookup("Person", "name", &json!("Alice"));
    assert!(name_result.is_some());
    assert!(name_result.unwrap().contains(1));

    // Non-indexed property (age) should not be in index
    assert!(!index.has_index("Person", "age"));
}

#[test]
fn test_on_remove_node_removes_from_index() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");

    let mut properties = std::collections::HashMap::new();
    properties.insert("email".to_string(), json!("alice@example.com"));

    // Add node
    index.on_add_node("Person", 1, &properties);
    assert!(index
        .lookup("Person", "email", &json!("alice@example.com"))
        .is_some());

    // Remove node
    index.on_remove_node("Person", 1, &properties);

    // Should no longer be in index
    let result = index.lookup("Person", "email", &json!("alice@example.com"));
    assert!(result.is_none());
}

#[test]
fn test_on_update_property_updates_index() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");

    // Add initial value
    index.insert("Person", "email", &json!("old@example.com"), 1);

    // Verify old value exists
    assert!(index
        .lookup("Person", "email", &json!("old@example.com"))
        .unwrap()
        .contains(1));

    // Update property
    index.on_update_property(
        "Person",
        1,
        "email",
        &json!("old@example.com"),
        &json!("new@example.com"),
    );

    // Old value should be gone
    let old_result = index.lookup("Person", "email", &json!("old@example.com"));
    assert!(old_result.is_none());

    // New value should exist
    let new_result = index.lookup("Person", "email", &json!("new@example.com"));
    assert!(new_result.is_some());
    assert!(new_result.unwrap().contains(1));
}

#[test]
fn test_on_update_non_indexed_property_noop() {
    let mut index = PropertyIndex::new();
    // No index created for "age"

    // Should not panic or error
    index.on_update_property("Person", 1, "age", &json!(25), &json!(30));

    // No index should exist
    assert!(!index.has_index("Person", "age"));
}

#[test]
fn test_index_consistency_after_multiple_mutations() {
    let mut index = PropertyIndex::new();
    index.create_index("Document", "category");

    let mut props1 = std::collections::HashMap::new();
    props1.insert("category".to_string(), json!("tech"));

    let mut props2 = std::collections::HashMap::new();
    props2.insert("category".to_string(), json!("tech"));

    let mut props3 = std::collections::HashMap::new();
    props3.insert("category".to_string(), json!("science"));

    // Add 3 documents
    index.on_add_node("Document", 1, &props1);
    index.on_add_node("Document", 2, &props2);
    index.on_add_node("Document", 3, &props3);

    // Verify counts
    assert_eq!(
        index
            .lookup("Document", "category", &json!("tech"))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        index
            .lookup("Document", "category", &json!("science"))
            .unwrap()
            .len(),
        1
    );

    // Remove one tech document
    index.on_remove_node("Document", 1, &props1);
    assert_eq!(
        index
            .lookup("Document", "category", &json!("tech"))
            .unwrap()
            .len(),
        1
    );

    // Update remaining tech to science
    index.on_update_property("Document", 2, "category", &json!("tech"), &json!("science"));

    // Tech should be empty now
    assert!(index
        .lookup("Document", "category", &json!("tech"))
        .is_none());

    // Science should have 2
    assert_eq!(
        index
            .lookup("Document", "category", &json!("science"))
            .unwrap()
            .len(),
        2
    );
}

// =============================================================================
// EPIC-047 US-001: Composite Graph Index Tests
// =============================================================================

#[test]
fn test_composite_index_create() {
    let index = CompositeGraphIndex::new(
        "Person",
        vec!["name".to_string(), "city".to_string()],
        CompositeIndexType::Hash,
    );

    assert_eq!(index.label(), "Person");
    assert_eq!(index.properties(), &["name", "city"]);
    assert_eq!(index.index_type(), CompositeIndexType::Hash);
}

#[test]
fn test_composite_index_insert_and_lookup() {
    let mut index = CompositeGraphIndex::new(
        "Person",
        vec!["name".to_string(), "city".to_string()],
        CompositeIndexType::Hash,
    );

    index.insert(1, &[json!("Alice"), json!("Paris")]);
    index.insert(2, &[json!("Bob"), json!("London")]);
    index.insert(3, &[json!("Alice"), json!("Paris")]); // Duplicate values

    let nodes = index.lookup(&[json!("Alice"), json!("Paris")]);
    assert_eq!(nodes.len(), 2);
    assert!(nodes.contains(&1));
    assert!(nodes.contains(&3));

    let nodes2 = index.lookup(&[json!("Bob"), json!("London")]);
    assert_eq!(nodes2, &[2]);

    // Non-existent combination
    let nodes3 = index.lookup(&[json!("Alice"), json!("London")]);
    assert!(nodes3.is_empty());
}

#[test]
fn test_composite_index_remove() {
    let mut index =
        CompositeGraphIndex::new("Person", vec!["name".to_string()], CompositeIndexType::Hash);

    index.insert(1, &[json!("Alice")]);
    index.insert(2, &[json!("Alice")]);

    assert_eq!(index.lookup(&[json!("Alice")]).len(), 2);

    let removed = index.remove(1, &[json!("Alice")]);
    assert!(removed);
    assert_eq!(index.lookup(&[json!("Alice")]).len(), 1);
    assert_eq!(index.lookup(&[json!("Alice")])[0], 2);
}

#[test]
fn test_composite_index_covers() {
    let index = CompositeGraphIndex::new(
        "Person",
        vec!["name".to_string(), "age".to_string()],
        CompositeIndexType::Hash,
    );

    assert!(index.covers("Person", &["name"]));
    assert!(index.covers("Person", &["age"]));
    assert!(index.covers("Person", &["name", "age"]));
    assert!(!index.covers("Company", &["name"]));
    assert!(!index.covers("Person", &["email"]));
}

#[test]
fn test_composite_index_cardinality() {
    let mut index =
        CompositeGraphIndex::new("Person", vec!["city".to_string()], CompositeIndexType::Hash);

    index.insert(1, &[json!("Paris")]);
    index.insert(2, &[json!("London")]);
    index.insert(3, &[json!("Paris")]);

    assert_eq!(index.cardinality(), 2); // Paris and London
    assert_eq!(index.node_count(), 3);
}

#[test]
fn test_composite_index_manager() {
    let mut manager = CompositeIndexManager::new();

    let created = manager.create_index(
        "idx_person_name",
        "Person",
        vec!["name".to_string()],
        CompositeIndexType::Hash,
    );
    assert!(created);

    // Duplicate name should fail
    let created2 = manager.create_index(
        "idx_person_name",
        "Person",
        vec!["email".to_string()],
        CompositeIndexType::Hash,
    );
    assert!(!created2);

    let index = manager.get_mut("idx_person_name").unwrap();
    index.insert(1, &[json!("Alice")]);

    let covering = manager.find_covering_indexes("Person", &["name"]);
    assert_eq!(covering, vec!["idx_person_name"]);

    let dropped = manager.drop_index("idx_person_name");
    assert!(dropped);
    assert!(manager.get("idx_person_name").is_none());
}

#[test]
fn test_composite_index_manager_hooks() {
    let mut manager = CompositeIndexManager::new();
    manager.create_index(
        "idx_person_name_city",
        "Person",
        vec!["name".to_string(), "city".to_string()],
        CompositeIndexType::Hash,
    );

    let mut props = HashMap::new();
    props.insert("name".to_string(), json!("Alice"));
    props.insert("city".to_string(), json!("Paris"));

    manager.on_add_node("Person", 1, &props);

    // Check after add
    {
        let index = manager.get("idx_person_name_city").unwrap();
        let nodes = index.lookup(&[json!("Alice"), json!("Paris")]);
        assert_eq!(nodes, &[1]);
    }

    manager.on_remove_node("Person", 1, &props);

    // Check after remove
    {
        let index = manager.get("idx_person_name_city").unwrap();
        let nodes2 = index.lookup(&[json!("Alice"), json!("Paris")]);
        assert!(nodes2.is_empty());
    }
}

// =============================================================================
// EPIC-047 US-002: Range Index Tests
// =============================================================================

#[test]
fn test_range_index_create() {
    let index = CompositeRangeIndex::new("Person", "age");
    assert_eq!(index.label(), "Person");
    assert_eq!(index.property(), "age");
}

#[test]
fn test_range_index_insert_and_lookup() {
    let mut index = CompositeRangeIndex::new("Person", "age");

    index.insert(1, &json!(25));
    index.insert(2, &json!(30));
    index.insert(3, &json!(35));
    index.insert(4, &json!(30)); // Duplicate value

    assert_eq!(index.lookup_exact(&json!(30)).len(), 2);
    assert_eq!(index.lookup_exact(&json!(25)), &[1]);
}

#[test]
fn test_range_index_range_lookup() {
    let mut index = CompositeRangeIndex::new("Person", "age");

    index.insert(1, &json!(20));
    index.insert(2, &json!(25));
    index.insert(3, &json!(30));
    index.insert(4, &json!(35));
    index.insert(5, &json!(40));

    // Range [25, 35]
    let result = index.lookup_range(Some(&json!(25)), Some(&json!(35)));
    assert_eq!(result.len(), 3);
    assert!(result.contains(&2));
    assert!(result.contains(&3));
    assert!(result.contains(&4));
}

#[test]
fn test_range_index_gt_lt() {
    let mut index = CompositeRangeIndex::new("Person", "age");

    index.insert(1, &json!(20));
    index.insert(2, &json!(30));
    index.insert(3, &json!(40));

    let gt_result = index.lookup_gt(&json!(25));
    assert_eq!(gt_result.len(), 2);

    let lt_result = index.lookup_lt(&json!(35));
    assert_eq!(lt_result.len(), 2);
}

// =============================================================================
// EPIC-047 US-003: Edge Property Index Tests
// =============================================================================

#[test]
fn test_edge_index_create() {
    let index = EdgePropertyIndex::new("KNOWS", "since");
    assert_eq!(index.rel_type(), "KNOWS");
    assert_eq!(index.property(), "since");
}

#[test]
fn test_edge_index_insert_and_lookup() {
    let mut index = EdgePropertyIndex::new("KNOWS", "since");

    index.insert(100, &json!(2020));
    index.insert(101, &json!(2021));
    index.insert(102, &json!(2020));

    assert_eq!(index.lookup_exact(&json!(2020)).len(), 2);
    assert_eq!(index.lookup_exact(&json!(2021)), &[101]);
}

#[test]
fn test_edge_index_range() {
    let mut index = EdgePropertyIndex::new("KNOWS", "since");

    index.insert(1, &json!(2018));
    index.insert(2, &json!(2020));
    index.insert(3, &json!(2022));

    let result = index.lookup_range(Some(&json!(2019)), Some(&json!(2021)));
    assert_eq!(result, vec![2]);
}

// =============================================================================
// EPIC-047 US-004: Index Intersection Tests
// =============================================================================

#[test]
fn test_intersect_two_sets() {
    let a = vec![1, 2, 3, 4, 5];
    let b = vec![3, 4, 5, 6, 7];

    let result = IndexIntersection::intersect_two(&a, &b);
    assert_eq!(result.len(), 3);
    assert!(result.contains(&3));
    assert!(result.contains(&4));
    assert!(result.contains(&5));
}

#[test]
fn test_intersect_vecs() {
    let a = vec![1u64, 2, 3, 4, 5];
    let b = vec![3u64, 4, 5, 6, 7];
    let c = vec![4u64, 5, 8, 9];

    let result = IndexIntersection::intersect_vecs(&[&a, &b, &c]);
    assert_eq!(result.len(), 2);
    assert!(result.contains(&4));
    assert!(result.contains(&5));
}

#[test]
fn test_intersect_empty() {
    let a = vec![1u64, 2, 3];
    let b = vec![4u64, 5, 6];

    let result = IndexIntersection::intersect_two(&a, &b);
    assert!(result.is_empty());
}

// =============================================================================
// EPIC-047 US-005: Auto-Index Suggestions Tests
// =============================================================================

#[test]
fn test_pattern_tracker_record() {
    let mut tracker = QueryPatternTracker::new();

    let pattern = QueryPattern {
        labels: vec!["Person".to_string()],
        properties: vec!["name".to_string()],
        predicates: vec![PredicateType::Equality],
    };

    tracker.record(pattern.clone(), 50);
    tracker.record(pattern.clone(), 100);

    let patterns = tracker.expensive_patterns();
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].1.count, 2);
    assert_eq!(patterns[0].1.total_time_ms, 150);
}

#[test]
fn test_pattern_tracker_slow_patterns() {
    let mut tracker = QueryPatternTracker::new();
    tracker.set_threshold(50);

    let fast_pattern = QueryPattern {
        labels: vec!["Fast".to_string()],
        properties: vec!["prop".to_string()],
        predicates: vec![PredicateType::Equality],
    };

    let slow_pattern = QueryPattern {
        labels: vec!["Slow".to_string()],
        properties: vec!["prop".to_string()],
        predicates: vec![PredicateType::Equality],
    };

    tracker.record(fast_pattern, 30);
    tracker.record(slow_pattern, 100);

    let slow = tracker.slow_patterns();
    assert_eq!(slow.len(), 1);
    assert_eq!(slow[0].0.labels[0], "Slow");
}

#[test]
fn test_index_advisor_suggest() {
    let mut tracker = QueryPatternTracker::new();

    let pattern = QueryPattern {
        labels: vec!["Person".to_string()],
        properties: vec!["email".to_string()],
        predicates: vec![PredicateType::Equality],
    };

    // Record multiple times to make it expensive
    for _ in 0..10 {
        tracker.record(pattern.clone(), 100);
    }

    let advisor = IndexAdvisor::new();
    let suggestions = advisor.suggest(&tracker);

    assert!(!suggestions.is_empty());
    assert!(suggestions[0].ddl.contains("email"));
    assert!(suggestions[0].estimated_improvement > 0.5);
}

#[test]
fn test_index_advisor_skip_existing() {
    let mut tracker = QueryPatternTracker::new();

    let pattern = QueryPattern {
        labels: vec!["Person".to_string()],
        properties: vec!["name".to_string()],
        predicates: vec![PredicateType::Equality],
    };

    tracker.record(pattern, 100);

    let mut advisor = IndexAdvisor::new();
    advisor.register_index("idx_person_name");

    let suggestions = advisor.suggest(&tracker);
    assert!(suggestions.is_empty());
}
