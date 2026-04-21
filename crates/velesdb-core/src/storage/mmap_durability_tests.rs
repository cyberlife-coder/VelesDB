#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::float_cmp
)]
//! Tests for Issue #423 Component 4: `DurabilityMode` on `MmapStorage`.
//!
//! Verifies that `DurabilityMode::None` skips WAL writes entirely for bulk
//! import scenarios, while the default `Fsync` mode preserves existing behavior.

use super::mmap::MmapStorage;
use super::traits::VectorStorage;
use crate::storage::DurabilityMode;
use tempfile::tempdir;

// =========================================================================
// Default mode is Fsync (backward compatible)
// =========================================================================

#[test]
fn test_default_durability_is_fsync() {
    let temp = tempdir().expect("temp dir");
    let storage = MmapStorage::new(temp.path(), 4).expect("create storage");
    assert_eq!(
        storage.durability(),
        DurabilityMode::Fsync,
        "default durability mode should be Fsync"
    );
}

// =========================================================================
// DurabilityMode::None skips WAL writes
// =========================================================================

#[test]
fn test_durability_none_skips_wal_writes() {
    let temp = tempdir().expect("temp dir");
    let mut storage =
        MmapStorage::new_with_durability(temp.path(), 4, DurabilityMode::None).expect("create");

    // Record WAL size before store
    let wal_path = temp.path().join("vectors.wal");
    let wal_size_before = std::fs::metadata(&wal_path).map_or(0, |m| m.len());

    // Store a vector
    storage
        .store(1, &[1.0, 2.0, 3.0, 4.0])
        .expect("store should succeed");

    // Flush to ensure any buffered writes are committed
    storage.flush().expect("flush");

    // WAL should NOT have grown (no WAL writes in None mode)
    let wal_size_after = std::fs::metadata(&wal_path).map_or(0, |m| m.len());
    assert_eq!(
        wal_size_before, wal_size_after,
        "WAL should not grow in DurabilityMode::None"
    );
}

#[test]
fn test_durability_none_store_batch_skips_wal() {
    let temp = tempdir().expect("temp dir");
    let mut storage =
        MmapStorage::new_with_durability(temp.path(), 4, DurabilityMode::None).expect("create");

    let wal_path = temp.path().join("vectors.wal");
    let wal_size_before = std::fs::metadata(&wal_path).map_or(0, |m| m.len());

    // Store a batch
    let vectors: Vec<(u64, &[f32])> = vec![
        (1, &[1.0, 2.0, 3.0, 4.0]),
        (2, &[5.0, 6.0, 7.0, 8.0]),
        (3, &[9.0, 10.0, 11.0, 12.0]),
    ];
    let count = storage.store_batch(&vectors).expect("store_batch");
    assert_eq!(count, 3, "should store all 3 vectors");

    storage.flush().expect("flush");

    let wal_size_after = std::fs::metadata(&wal_path).map_or(0, |m| m.len());
    assert_eq!(
        wal_size_before, wal_size_after,
        "WAL should not grow in DurabilityMode::None for store_batch"
    );
}

// =========================================================================
// Data is still readable after DurabilityMode::None flush (via mmap)
// =========================================================================

#[test]
fn test_durability_none_data_readable_via_mmap() {
    let temp = tempdir().expect("temp dir");
    let mut storage =
        MmapStorage::new_with_durability(temp.path(), 4, DurabilityMode::None).expect("create");

    storage.store(1, &[1.0, 2.0, 3.0, 4.0]).expect("store");
    storage.flush().expect("flush");

    // Data should be readable from mmap even without WAL
    let retrieved = storage
        .retrieve(1)
        .expect("retrieve")
        .expect("should exist");
    assert_eq!(
        retrieved,
        vec![1.0, 2.0, 3.0, 4.0],
        "vector should be readable from mmap"
    );
}

// =========================================================================
// Fsync mode writes to WAL (unchanged behavior)
// =========================================================================

#[test]
fn test_fsync_mode_writes_to_wal() {
    let temp = tempdir().expect("temp dir");
    let mut storage =
        MmapStorage::new_with_durability(temp.path(), 4, DurabilityMode::Fsync).expect("create");

    let wal_path = temp.path().join("vectors.wal");
    let wal_size_before = std::fs::metadata(&wal_path).map_or(0, |m| m.len());

    storage.store(1, &[1.0, 2.0, 3.0, 4.0]).expect("store");
    storage.flush().expect("flush");

    let wal_size_after = std::fs::metadata(&wal_path).map_or(0, |m| m.len());
    assert!(
        wal_size_after > wal_size_before,
        "WAL should grow in Fsync mode (before={wal_size_before}, after={wal_size_after})"
    );
}

// =========================================================================
// set_durability_mode allows runtime switching
// =========================================================================

#[test]
fn test_set_durability_mode_runtime_switch() {
    let temp = tempdir().expect("temp dir");
    let mut storage = MmapStorage::new(temp.path(), 4).expect("create");

    assert_eq!(storage.durability(), DurabilityMode::Fsync);

    storage.set_durability_mode(DurabilityMode::None);
    assert_eq!(storage.durability(), DurabilityMode::None);

    // Store in None mode — no WAL growth
    let wal_path = temp.path().join("vectors.wal");
    let wal_size_before = std::fs::metadata(&wal_path).map_or(0, |m| m.len());

    storage.store(1, &[1.0, 2.0, 3.0, 4.0]).expect("store");
    storage.flush().expect("flush");

    let wal_size_after = std::fs::metadata(&wal_path).map_or(0, |m| m.len());
    assert_eq!(
        wal_size_before, wal_size_after,
        "WAL should not grow after switching to None mode"
    );
}

// =========================================================================
// FlushOnly mode: WAL writes happen but no fsync
// =========================================================================

#[test]
fn test_flush_only_mode_writes_to_wal() {
    let temp = tempdir().expect("temp dir");
    let mut storage = MmapStorage::new_with_durability(temp.path(), 4, DurabilityMode::FlushOnly)
        .expect("create");

    let wal_path = temp.path().join("vectors.wal");
    let wal_size_before = std::fs::metadata(&wal_path).map_or(0, |m| m.len());

    storage.store(1, &[1.0, 2.0, 3.0, 4.0]).expect("store");
    storage.flush().expect("flush");

    let wal_size_after = std::fs::metadata(&wal_path).map_or(0, |m| m.len());
    assert!(
        wal_size_after > wal_size_before,
        "WAL should grow in FlushOnly mode"
    );
}
