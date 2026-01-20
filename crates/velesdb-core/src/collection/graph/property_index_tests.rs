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
