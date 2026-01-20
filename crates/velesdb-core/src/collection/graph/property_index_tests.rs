//! Tests for PropertyIndex.

use super::property_index::PropertyIndex;
use serde_json::json;

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
