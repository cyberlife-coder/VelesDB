//! Tests for `RangeIndex`.

use super::range_index::{OrderedFloat, OrderedValue, RangeIndex};
use serde_json::json;

#[test]
fn test_create_range_index() {
    let mut index = RangeIndex::new();

    assert!(!index.has_index("Event", "timestamp"));

    index.create_index("Event", "timestamp");

    assert!(index.has_index("Event", "timestamp"));
    assert!(!index.has_index("Event", "name"));
}

#[test]
fn test_insert_and_basic_operations() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    // Insert values
    assert!(index.insert("Event", "timestamp", &json!(100), 1));
    assert!(index.insert("Event", "timestamp", &json!(200), 2));
    assert!(index.insert("Event", "timestamp", &json!(300), 3));

    // Insert to non-existent index returns false
    assert!(!index.insert("Other", "field", &json!(1), 1));
}

#[test]
fn test_range_greater_than() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &json!(100), 1);
    index.insert("Event", "timestamp", &json!(200), 2);
    index.insert("Event", "timestamp", &json!(300), 3);
    index.insert("Event", "timestamp", &json!(400), 4);

    // timestamp > 200 should return nodes 3, 4
    let result = index.range_greater_than("Event", "timestamp", &json!(200));
    assert_eq!(result.len(), 2);
    assert!(!result.contains(1));
    assert!(!result.contains(2));
    assert!(result.contains(3));
    assert!(result.contains(4));
}

#[test]
fn test_range_greater_or_equal() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &json!(100), 1);
    index.insert("Event", "timestamp", &json!(200), 2);
    index.insert("Event", "timestamp", &json!(300), 3);

    // timestamp >= 200 should return nodes 2, 3
    let result = index.range_greater_or_equal("Event", "timestamp", &json!(200));
    assert_eq!(result.len(), 2);
    assert!(!result.contains(1));
    assert!(result.contains(2));
    assert!(result.contains(3));
}

#[test]
fn test_range_less_than() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &json!(100), 1);
    index.insert("Event", "timestamp", &json!(200), 2);
    index.insert("Event", "timestamp", &json!(300), 3);

    // timestamp < 200 should return node 1
    let result = index.range_less_than("Event", "timestamp", &json!(200));
    assert_eq!(result.len(), 1);
    assert!(result.contains(1));
    assert!(!result.contains(2));
    assert!(!result.contains(3));
}

#[test]
fn test_range_less_or_equal() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &json!(100), 1);
    index.insert("Event", "timestamp", &json!(200), 2);
    index.insert("Event", "timestamp", &json!(300), 3);

    // timestamp <= 200 should return nodes 1, 2
    let result = index.range_less_or_equal("Event", "timestamp", &json!(200));
    assert_eq!(result.len(), 2);
    assert!(result.contains(1));
    assert!(result.contains(2));
    assert!(!result.contains(3));
}

#[test]
fn test_range_between() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &json!(100), 1);
    index.insert("Event", "timestamp", &json!(200), 2);
    index.insert("Event", "timestamp", &json!(300), 3);
    index.insert("Event", "timestamp", &json!(400), 4);
    index.insert("Event", "timestamp", &json!(500), 5);

    // 200 <= timestamp <= 400 should return nodes 2, 3, 4
    let result = index.range_between("Event", "timestamp", &json!(200), &json!(400));
    assert_eq!(result.len(), 3);
    assert!(!result.contains(1));
    assert!(result.contains(2));
    assert!(result.contains(3));
    assert!(result.contains(4));
    assert!(!result.contains(5));
}

#[test]
fn test_range_with_floats() {
    let mut index = RangeIndex::new();
    index.create_index("Measurement", "value");

    index.insert("Measurement", "value", &json!(1.5), 1);
    index.insert("Measurement", "value", &json!(2.5), 2);
    index.insert("Measurement", "value", &json!(3.5), 3);

    // value > 2.0 should return nodes 2, 3
    let result = index.range_greater_than("Measurement", "value", &json!(2.0));
    assert_eq!(result.len(), 2);
    assert!(result.contains(2));
    assert!(result.contains(3));
}

#[test]
fn test_range_with_strings() {
    let mut index = RangeIndex::new();
    index.create_index("Person", "name");

    index.insert("Person", "name", &json!("Alice"), 1);
    index.insert("Person", "name", &json!("Bob"), 2);
    index.insert("Person", "name", &json!("Charlie"), 3);

    // name > "Bob" should return Charlie (lexicographic)
    let result = index.range_greater_than("Person", "name", &json!("Bob"));
    assert_eq!(result.len(), 1);
    assert!(result.contains(3));

    // name <= "Bob" should return Alice, Bob
    let result2 = index.range_less_or_equal("Person", "name", &json!("Bob"));
    assert_eq!(result2.len(), 2);
    assert!(result2.contains(1));
    assert!(result2.contains(2));
}

#[test]
fn test_remove_from_range_index() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &json!(100), 1);
    index.insert("Event", "timestamp", &json!(100), 2);

    // Both nodes should be in range
    let result = index.range_greater_or_equal("Event", "timestamp", &json!(100));
    assert_eq!(result.len(), 2);

    // Remove one
    assert!(index.remove("Event", "timestamp", &json!(100), 1));

    // Only one should remain
    let result2 = index.range_greater_or_equal("Event", "timestamp", &json!(100));
    assert_eq!(result2.len(), 1);
    assert!(result2.contains(2));
}

#[test]
fn test_drop_range_index() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");
    index.insert("Event", "timestamp", &json!(100), 1);

    assert!(index.has_index("Event", "timestamp"));

    let dropped = index.drop_index("Event", "timestamp");
    assert!(dropped);
    assert!(!index.has_index("Event", "timestamp"));
}

#[test]
fn test_range_empty_result() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &json!(100), 1);

    // timestamp > 1000 should return empty
    let result = index.range_greater_than("Event", "timestamp", &json!(1000));
    assert!(result.is_empty());
}

#[test]
fn test_range_non_existent_index() {
    let index = RangeIndex::new();

    // Query on non-existent index returns empty
    let result = index.range_greater_than("Event", "timestamp", &json!(100));
    assert!(result.is_empty());
}

#[test]
fn test_ordered_value_comparison() {
    // Null < Integer < Float < String
    assert!(OrderedValue::Null < OrderedValue::Integer(0));
    assert!(OrderedValue::Integer(100) < OrderedValue::Integer(200));
    assert!(OrderedValue::Integer(100) < OrderedValue::Float(OrderedFloat(100.5)));
    assert!(OrderedValue::Float(OrderedFloat(1.0)) < OrderedValue::String("a".to_string()));
}

#[test]
fn test_memory_usage() {
    let mut index = RangeIndex::new();
    let initial = index.memory_usage();

    index.create_index("Event", "timestamp");
    index.insert("Event", "timestamp", &json!(100), 1);

    let after = index.memory_usage();
    assert!(after > initial);
}

// =========================================================================
// Persistence tests (US-005)
// =========================================================================

#[test]
fn test_range_index_serialize_deserialize() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");
    index.insert("Event", "timestamp", &json!(100), 1);
    index.insert("Event", "timestamp", &json!(200), 2);
    index.insert("Event", "timestamp", &json!(300), 3);

    // Serialize
    let bytes = index.to_bytes().expect("Serialization failed");
    assert!(!bytes.is_empty());

    // Deserialize
    let loaded = RangeIndex::from_bytes(&bytes).expect("Deserialization failed");

    // Verify range queries work after deserialization
    let result = loaded.range_greater_than("Event", "timestamp", &json!(150));
    assert_eq!(result.len(), 2);
    assert!(result.contains(2));
    assert!(result.contains(3));
}

#[test]
fn test_range_index_persist_to_file() {
    let mut index = RangeIndex::new();
    index.create_index("Measurement", "value");
    index.insert("Measurement", "value", &json!(1.5), 1);
    index.insert("Measurement", "value", &json!(2.5), 2);
    index.insert("Measurement", "value", &json!(3.5), 3);

    // Save to temp file
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_range_index.bin");

    index.save_to_file(&file_path).expect("Save failed");
    assert!(file_path.exists());

    // Load from file
    let loaded = RangeIndex::load_from_file(&file_path).expect("Load failed");

    // Verify range query works
    let result = loaded.range_between("Measurement", "value", &json!(2.0), &json!(3.0));
    assert_eq!(result.len(), 1);
    assert!(result.contains(2));

    // Cleanup
    std::fs::remove_file(&file_path).ok();
}

#[test]
fn test_range_index_corrupted_data() {
    let corrupted = vec![0u8, 1, 2, 3, 255, 254];
    let result = RangeIndex::from_bytes(&corrupted);
    assert!(result.is_err());
}
