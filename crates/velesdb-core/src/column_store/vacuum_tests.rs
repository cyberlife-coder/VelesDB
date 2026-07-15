//! Tests for column-store vacuum, `should_vacuum()` threshold logic,
//! and `is_row_deleted_bitmap()`.

#![allow(clippy::cast_possible_wrap)]

use crate::column_store::{ColumnStore, ColumnType, ColumnValue, VacuumConfig};

/// Helper: builds a store with `count` rows, each having an Int `id` column.
fn store_with_id_rows(count: usize) -> ColumnStore {
    let mut store = ColumnStore::with_schema(&[("id", ColumnType::Int)]);
    for i in 0..count {
        store.push_row(&[("id", ColumnValue::Int(i as i64))]);
    }
    store
}

// ─────────────────────────────────────────────────────────────────────────────
// vacuum() removes deleted rows
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn vacuum_removes_tombstones() {
    let mut store = store_with_id_rows(10);

    // Soft-delete rows 3 and 7
    if let Ok(idx) = u32::try_from(3_usize) {
        store.deletion_bitmap.insert(idx);
    }
    if let Ok(idx) = u32::try_from(7_usize) {
        store.deletion_bitmap.insert(idx);
    }

    let stats = store.vacuum(VacuumConfig::default());
    assert!(stats.completed);
    assert_eq!(stats.tombstones_found, 2);
    assert_eq!(stats.tombstones_removed, 2);
    assert_eq!(store.row_count(), 8);
    assert_eq!(store.deleted_row_count(), 0);
}

#[test]
fn vacuum_no_tombstones_is_noop() {
    let mut store = store_with_id_rows(5);
    let stats = store.vacuum(VacuumConfig::default());
    assert!(stats.completed);
    assert_eq!(stats.tombstones_found, 0);
    assert_eq!(store.row_count(), 5);
}

#[test]
fn vacuum_preserves_remaining_data() {
    let mut store = store_with_id_rows(5);
    // Delete row at index 2 (id=2)
    if let Ok(idx) = u32::try_from(2_usize) {
        store.deletion_bitmap.insert(idx);
    }

    store.vacuum(VacuumConfig::default());

    // The remaining 4 rows should have ids 0,1,3,4 (compacted)
    let ids = store.filter_eq_int("id", 2);
    assert!(ids.is_empty(), "deleted id=2 must not appear after vacuum");

    let all_rows = store.row_count();
    assert_eq!(all_rows, 4);
}

// ─────────────────────────────────────────────────────────────────────────────
// should_vacuum() threshold logic
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn should_vacuum_below_threshold() {
    let mut store = store_with_id_rows(100);
    // Delete 10 out of 100 => 10%, threshold 20% => false
    for i in 0..10u32 {
        store.deletion_bitmap.insert(i);
    }
    assert!(!store.should_vacuum(0.20));
}

#[test]
fn should_vacuum_at_threshold() {
    let mut store = store_with_id_rows(100);
    // Delete 20 out of 100 => 20%, threshold 20% => true
    for i in 0..20u32 {
        store.deletion_bitmap.insert(i);
    }
    assert!(store.should_vacuum(0.20));
}

#[test]
fn should_vacuum_above_threshold() {
    let mut store = store_with_id_rows(100);
    for i in 0..50u32 {
        store.deletion_bitmap.insert(i);
    }
    assert!(store.should_vacuum(0.20));
}

#[test]
fn should_vacuum_empty_store() {
    let store = ColumnStore::new();
    assert!(!store.should_vacuum(0.20));
}

// ─────────────────────────────────────────────────────────────────────────────
// is_row_deleted_bitmap()
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn is_row_deleted_bitmap_true_for_deleted() {
    let mut store = store_with_id_rows(5);
    store.deletion_bitmap.insert(2);

    assert!(store.is_row_deleted_bitmap(2));
}

#[test]
fn is_row_deleted_bitmap_false_for_live() {
    let store = store_with_id_rows(5);
    assert!(!store.is_row_deleted_bitmap(0));
    assert!(!store.is_row_deleted_bitmap(4));
}

#[test]
fn live_row_indices_excludes_deleted() {
    let mut store = store_with_id_rows(5);
    store.deletion_bitmap.insert(1);
    store.deletion_bitmap.insert(3);

    let live: Vec<usize> = store.live_row_indices().collect();
    assert_eq!(live, vec![0, 2, 4]);
}
