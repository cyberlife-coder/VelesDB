//! Tests for the `LabelIndex` graph node label index.

use super::label_index::LabelIndex;
use serde_json::json;

#[test]
fn test_new_index_is_empty() {
    let index = LabelIndex::new();
    assert!(index.is_empty());
    assert_eq!(index.label_count(), 0);
}

#[test]
fn test_index_from_payload_single_label() {
    let mut index = LabelIndex::new();
    let payload = json!({"_labels": ["Person"], "name": "Alice"});
    let count = index.index_from_payload(1, &payload);
    assert_eq!(count, 1);
    assert!(!index.is_empty());

    let bitmap = index.lookup("Person").expect("test: Person should exist");
    assert!(bitmap.contains(1));
    assert!(!bitmap.contains(2));
}

#[test]
fn test_index_from_payload_multiple_labels() {
    let mut index = LabelIndex::new();
    let payload = json!({"_labels": ["Person", "Employee"]});
    let count = index.index_from_payload(10, &payload);
    assert_eq!(count, 2);

    assert!(index.lookup("Person").expect("test: Person").contains(10));
    assert!(index
        .lookup("Employee")
        .expect("test: Employee")
        .contains(10));
}

#[test]
fn test_index_from_payload_no_labels() {
    let mut index = LabelIndex::new();
    let payload = json!({"name": "Alice"});
    let count = index.index_from_payload(1, &payload);
    assert_eq!(count, 0);
    assert!(index.is_empty());
}

#[test]
fn test_index_from_payload_empty_labels_array() {
    let mut index = LabelIndex::new();
    let payload = json!({"_labels": []});
    let count = index.index_from_payload(1, &payload);
    assert_eq!(count, 0);
}

#[test]
fn test_index_from_payload_non_string_labels_ignored() {
    let mut index = LabelIndex::new();
    let payload = json!({"_labels": ["Person", 42, null, "Company"]});
    let count = index.index_from_payload(1, &payload);
    assert_eq!(count, 2); // Only "Person" and "Company"
    assert!(index.lookup("Person").is_some());
    assert!(index.lookup("Company").is_some());
}

#[test]
fn test_insert_single() {
    let mut index = LabelIndex::new();
    assert!(index.insert("Document", 5));
    assert!(index
        .lookup("Document")
        .expect("test: Document")
        .contains(5));
}

#[test]
fn test_insert_duplicate_returns_false() {
    let mut index = LabelIndex::new();
    assert!(index.insert("Document", 5));
    assert!(!index.insert("Document", 5)); // Already exists
}

#[test]
fn test_insert_exceeds_u32_max_returns_false() {
    let mut index = LabelIndex::new();
    let large_id = u64::from(u32::MAX) + 1;
    assert!(!index.insert("Document", large_id));
    assert!(index.is_empty());
}

#[test]
fn test_remove_from_payload() {
    let mut index = LabelIndex::new();
    let payload = json!({"_labels": ["Person", "Employee"]});
    index.index_from_payload(1, &payload);

    index.remove_from_payload(1, &payload);
    assert!(index.lookup("Person").is_none());
    assert!(index.lookup("Employee").is_none());
    assert!(index.is_empty());
}

#[test]
fn test_remove_from_payload_preserves_other_nodes() {
    let mut index = LabelIndex::new();
    let payload1 = json!({"_labels": ["Person"]});
    let payload2 = json!({"_labels": ["Person"]});
    index.index_from_payload(1, &payload1);
    index.index_from_payload(2, &payload2);

    index.remove_from_payload(1, &payload1);
    let bitmap = index.lookup("Person").expect("test: Person still exists");
    assert!(!bitmap.contains(1));
    assert!(bitmap.contains(2));
}

#[test]
fn test_lookup_intersection_single_label() {
    let mut index = LabelIndex::new();
    index.index_from_payload(1, &json!({"_labels": ["Person"]}));
    index.index_from_payload(2, &json!({"_labels": ["Company"]}));

    let result = index
        .lookup_intersection(&["Person".to_string()])
        .expect("test: should match");
    assert!(result.contains(1));
    assert!(!result.contains(2));
}

#[test]
fn test_lookup_intersection_multiple_labels() {
    let mut index = LabelIndex::new();
    index.index_from_payload(1, &json!({"_labels": ["Person", "Employee"]}));
    index.index_from_payload(2, &json!({"_labels": ["Person"]}));
    index.index_from_payload(3, &json!({"_labels": ["Employee"]}));

    let result = index
        .lookup_intersection(&["Person".to_string(), "Employee".to_string()])
        .expect("test: should have intersection");
    assert!(result.contains(1)); // Has both labels
    assert!(!result.contains(2)); // Only Person
    assert!(!result.contains(3)); // Only Employee
}

#[test]
fn test_lookup_intersection_no_match() {
    let mut index = LabelIndex::new();
    index.index_from_payload(1, &json!({"_labels": ["Person"]}));

    let result = index.lookup_intersection(&["Unknown".to_string()]);
    assert!(result.is_none());
}

#[test]
fn test_lookup_intersection_empty_labels() {
    let index = LabelIndex::new();
    let result = index.lookup_intersection(&[]);
    assert!(result.is_none());
}

#[test]
fn test_clear() {
    let mut index = LabelIndex::new();
    index.index_from_payload(1, &json!({"_labels": ["Person"]}));
    assert!(!index.is_empty());

    index.clear();
    assert!(index.is_empty());
    assert_eq!(index.label_count(), 0);
}

#[test]
fn test_memory_usage_increases_with_data() {
    let mut index = LabelIndex::new();
    let baseline = index.memory_usage();

    index.index_from_payload(1, &json!({"_labels": ["Person"]}));
    assert!(index.memory_usage() > baseline);
}

#[test]
fn test_index_from_payload_u32_max_boundary() {
    let mut index = LabelIndex::new();
    let payload = json!({"_labels": ["Edge"]});

    // Exactly u32::MAX should be indexable.
    let count = index.index_from_payload(u64::from(u32::MAX), &payload);
    assert_eq!(count, 1);

    // u32::MAX + 1 should be rejected.
    let count = index.index_from_payload(u64::from(u32::MAX) + 1, &payload);
    assert_eq!(count, 0);
}
