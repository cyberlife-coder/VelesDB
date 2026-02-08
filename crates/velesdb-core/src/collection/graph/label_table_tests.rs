#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for `label_table` module - Label interning table.

use super::label_table::*;

#[test]
fn test_label_table_intern_returns_same_id() {
    let mut table = LabelTable::new();

    let id1 = table.intern("Person").unwrap();
    let id2 = table.intern("Person").unwrap();
    let id3 = table.intern("Company").unwrap();

    assert_eq!(id1, id2, "Same label should return same ID");
    assert_ne!(id1, id3, "Different labels should return different IDs");
}

#[test]
fn test_label_table_resolve_returns_original() {
    let mut table = LabelTable::new();

    let id = table.intern("Person").unwrap();
    assert_eq!(table.resolve(id), Some("Person"));

    let invalid_id = LabelId::from_u32(999);
    assert_eq!(table.resolve(invalid_id), None);
}

#[test]
fn test_label_table_len_and_is_empty() {
    let mut table = LabelTable::new();

    assert!(table.is_empty());
    assert_eq!(table.len(), 0);

    table.intern("A").unwrap();
    table.intern("B").unwrap();
    table.intern("A").unwrap();

    assert!(!table.is_empty());
    assert_eq!(table.len(), 2);
}

#[test]
fn test_label_table_get_id_without_intern() {
    let mut table = LabelTable::new();

    assert_eq!(table.get_id("Person"), None);

    let id = table.intern("Person").unwrap();
    assert_eq!(table.get_id("Person"), Some(id));
    assert_eq!(table.get_id("Company"), None);
}

#[test]
fn test_label_table_iter() {
    let mut table = LabelTable::new();

    table.intern("A").unwrap();
    table.intern("B").unwrap();
    table.intern("C").unwrap();

    let labels: Vec<_> = table.iter().collect();
    assert_eq!(labels.len(), 3);
    assert_eq!(labels[0].1, "A");
    assert_eq!(labels[1].1, "B");
    assert_eq!(labels[2].1, "C");
}

#[test]
fn test_label_table_with_capacity() {
    let table = LabelTable::with_capacity(100);
    assert!(table.is_empty());
}

#[test]
fn test_label_id_as_u32_and_from_u32() {
    let id = LabelId::from_u32(42);
    assert_eq!(id.as_u32(), 42);
}

#[test]
fn test_label_table_contains() {
    let mut table = LabelTable::new();

    assert!(!table.contains("Person"));
    table.intern("Person").unwrap();
    assert!(table.contains("Person"));
    assert!(!table.contains("Company"));
}

#[test]
fn test_label_table_many_labels() {
    let mut table = LabelTable::new();

    for i in 0..1000 {
        let label = format!("Label{}", i);
        let id = table.intern(&label).unwrap();
        assert_eq!(id.as_u32(), i as u32);
    }

    assert_eq!(table.len(), 1000);

    for i in 0..1000 {
        let label = format!("Label{}", i);
        let id = LabelId::from_u32(i as u32);
        assert_eq!(table.resolve(id), Some(label.as_str()));
    }
}
