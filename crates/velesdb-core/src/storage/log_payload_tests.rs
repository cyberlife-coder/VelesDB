//! Tests for `log_payload` module

use super::log_payload::{LogPayloadStorage, SNAPSHOT_MAGIC, SNAPSHOT_VERSION};
use super::traits::PayloadStorage;

use serde_json::json;
use tempfile::TempDir;

// -------------------------------------------------------------------------
// Helper functions
// -------------------------------------------------------------------------

fn create_test_storage() -> (LogPayloadStorage, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = LogPayloadStorage::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

// -------------------------------------------------------------------------
// Basic functionality tests (existing behavior)
// -------------------------------------------------------------------------

#[test]
fn test_store_and_retrieve_payload() {
    // Arrange
    let (mut storage, _temp) = create_test_storage();
    let payload = json!({"name": "test", "value": 42});

    // Act
    storage.store(1, &payload).expect("Store failed");
    let retrieved = storage.retrieve(1).expect("Retrieve failed");

    // Assert
    assert_eq!(retrieved, Some(payload));
}

#[test]
fn test_delete_payload() {
    // Arrange
    let (mut storage, _temp) = create_test_storage();
    let payload = json!({"key": "value"});
    storage.store(1, &payload).expect("Store failed");

    // Act
    storage.delete(1).expect("Delete failed");
    let retrieved = storage.retrieve(1).expect("Retrieve failed");

    // Assert
    assert_eq!(retrieved, None);
}

#[test]
fn test_ids_returns_all_stored_ids() {
    // Arrange
    let (mut storage, _temp) = create_test_storage();
    for i in 1..=5 {
        storage.store(i, &json!({"id": i})).expect("Store failed");
    }

    // Act
    let mut ids = storage.ids();
    ids.sort_unstable();

    // Assert
    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
}

// -------------------------------------------------------------------------
// TDD: Snapshot creation tests
// -------------------------------------------------------------------------

#[test]
fn test_create_snapshot_creates_file() {
    // Arrange
    let (mut storage, temp) = create_test_storage();
    for i in 1..=10 {
        storage.store(i, &json!({"id": i})).expect("Store failed");
    }

    // Act
    storage.create_snapshot().expect("Snapshot creation failed");

    // Assert
    let snapshot_path = temp.path().join("payloads.snapshot");
    assert!(snapshot_path.exists(), "Snapshot file should exist");
}

#[test]
fn test_create_snapshot_has_correct_magic() {
    // Arrange
    let (mut storage, temp) = create_test_storage();
    storage
        .store(1, &json!({"test": true}))
        .expect("Store failed");

    // Act
    storage.create_snapshot().expect("Snapshot creation failed");

    // Assert
    let snapshot_path = temp.path().join("payloads.snapshot");
    let data = std::fs::read(&snapshot_path).expect("Read snapshot failed");
    assert_eq!(&data[0..4], SNAPSHOT_MAGIC, "Magic bytes mismatch");
}

#[test]
fn test_create_snapshot_has_correct_version() {
    // Arrange
    let (mut storage, temp) = create_test_storage();
    storage
        .store(1, &json!({"test": true}))
        .expect("Store failed");

    // Act
    storage.create_snapshot().expect("Snapshot creation failed");

    // Assert
    let snapshot_path = temp.path().join("payloads.snapshot");
    let data = std::fs::read(&snapshot_path).expect("Read snapshot failed");
    assert_eq!(data[4], SNAPSHOT_VERSION, "Version mismatch");
}

// -------------------------------------------------------------------------
// TDD: Snapshot loading tests
// -------------------------------------------------------------------------

#[test]
fn test_load_from_snapshot_restores_index() {
    // Arrange
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create storage, add data, snapshot
    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create failed");
        for i in 1..=100 {
            storage.store(i, &json!({"id": i})).expect("Store failed");
        }
        storage.create_snapshot().expect("Snapshot failed");
    }

    // Act - Reopen storage (should load from snapshot)
    let storage = LogPayloadStorage::new(temp.path()).expect("Reopen failed");

    // Assert
    assert_eq!(storage.ids().len(), 100);
    for i in 1..=100 {
        let payload = storage.retrieve(i).expect("Retrieve failed");
        assert!(payload.is_some(), "Payload {i} should exist");
    }
}

#[test]
fn test_load_from_snapshot_plus_delta_wal() {
    // Arrange
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Phase 1: Create storage, add data, snapshot
    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create failed");
        for i in 1..=50 {
            storage
                .store(i, &json!({"id": i, "phase": 1}))
                .expect("Store failed");
        }
        storage.create_snapshot().expect("Snapshot failed");

        // Phase 2: Add more data AFTER snapshot (delta)
        for i in 51..=100 {
            storage
                .store(i, &json!({"id": i, "phase": 2}))
                .expect("Store failed");
        }
        storage.flush().expect("Flush failed");
    }

    // Act - Reopen storage (should load snapshot + replay delta)
    let storage = LogPayloadStorage::new(temp.path()).expect("Reopen failed");

    // Assert - All 100 entries should be present
    assert_eq!(storage.ids().len(), 100);

    // Check phase 1 data
    let p1 = storage.retrieve(25).expect("Retrieve failed").unwrap();
    assert_eq!(p1["phase"], 1);

    // Check phase 2 data (delta)
    let p2 = storage.retrieve(75).expect("Retrieve failed").unwrap();
    assert_eq!(p2["phase"], 2);
}

#[test]
fn test_load_from_snapshot_with_deletes_in_delta() {
    // Arrange
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Phase 1: Create, add data, snapshot
    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create failed");
        for i in 1..=50 {
            storage.store(i, &json!({"id": i})).expect("Store failed");
        }
        storage.create_snapshot().expect("Snapshot failed");

        // Phase 2: Delete some entries after snapshot
        for i in 1..=10 {
            storage.delete(i).expect("Delete failed");
        }
        storage.flush().expect("Flush failed");
    }

    // Act - Reopen storage
    let storage = LogPayloadStorage::new(temp.path()).expect("Reopen failed");

    // Assert - Only 40 entries should remain (50 - 10 deleted)
    assert_eq!(storage.ids().len(), 40);

    // Deleted entries should not exist
    for i in 1..=10 {
        assert!(storage.retrieve(i).expect("Retrieve failed").is_none());
    }

    // Remaining entries should exist
    for i in 11..=50 {
        assert!(storage.retrieve(i).expect("Retrieve failed").is_some());
    }
}

// -------------------------------------------------------------------------
// TDD: Snapshot heuristics tests
// -------------------------------------------------------------------------

#[test]
fn test_should_create_snapshot_false_when_fresh() {
    // Arrange
    let (storage, _temp) = create_test_storage();

    // Act & Assert
    assert!(!storage.should_create_snapshot());
}

#[test]
fn test_should_create_snapshot_true_after_threshold() {
    // Arrange
    let (mut storage, _temp) = create_test_storage();

    // Add enough data to exceed threshold (simulate large WAL)
    // Each payload ~100 bytes, need ~100k payloads for 10MB
    // For test, we'll use a smaller threshold
    let large_payload = json!({"data": "x".repeat(10000)});
    for i in 1..=1100 {
        storage.store(i, &large_payload).expect("Store failed");
    }

    // Act & Assert - Should recommend snapshot after ~11MB of writes
    assert!(storage.should_create_snapshot());
}

#[test]
fn test_should_create_snapshot_false_after_recent_snapshot() {
    // Arrange
    let (mut storage, _temp) = create_test_storage();

    // Add data and snapshot
    for i in 1..=100 {
        storage.store(i, &json!({"id": i})).expect("Store failed");
    }
    storage.create_snapshot().expect("Snapshot failed");

    // Act & Assert - Just snapshotted, should not recommend another
    assert!(!storage.should_create_snapshot());
}

// -------------------------------------------------------------------------
// TDD: Snapshot integrity tests
// -------------------------------------------------------------------------

#[test]
fn test_snapshot_crc_validation() {
    // Arrange
    let temp = TempDir::new().expect("Failed to create temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create failed");
        storage
            .store(1, &json!({"test": true}))
            .expect("Store failed");
        storage.create_snapshot().expect("Snapshot failed");
    }

    // Act - Corrupt the snapshot file
    let snapshot_path = temp.path().join("payloads.snapshot");
    let mut data = std::fs::read(&snapshot_path).expect("Read failed");
    if let Some(last) = data.last_mut() {
        *last ^= 0xFF; // Flip bits in CRC
    }
    std::fs::write(&snapshot_path, &data).expect("Write failed");

    // Assert - Should fall back to WAL replay (not panic)
    let storage = LogPayloadStorage::new(temp.path()).expect("Should recover via WAL");
    assert!(storage.retrieve(1).expect("Retrieve failed").is_some());
}

#[test]
fn test_snapshot_with_empty_storage() {
    // Arrange
    let (mut storage, temp) = create_test_storage();

    // Act - Snapshot empty storage
    storage.create_snapshot().expect("Snapshot failed");

    // Assert - File exists and is valid
    let snapshot_path = temp.path().join("payloads.snapshot");
    assert!(snapshot_path.exists());

    // Reopen should work
    let storage = LogPayloadStorage::new(temp.path()).expect("Reopen failed");
    assert_eq!(storage.ids().len(), 0);
}

// -------------------------------------------------------------------------
// TDD: Performance characteristics tests
// -------------------------------------------------------------------------

#[test]
fn test_wal_position_stored_in_snapshot() {
    // Arrange
    let temp = TempDir::new().expect("Failed to create temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create failed");
        for i in 1..=50 {
            storage.store(i, &json!({"id": i})).expect("Store failed");
        }
        storage.create_snapshot().expect("Snapshot failed");
    }

    // Act - Read snapshot and verify WAL position is stored
    let snapshot_path = temp.path().join("payloads.snapshot");
    let data = std::fs::read(&snapshot_path).expect("Read failed");

    // Assert - WAL position should be at offset 5 (after magic + version)
    // and should be > 0 (some data was written)
    let wal_pos = u64::from_le_bytes(data[5..13].try_into().unwrap());
    assert!(wal_pos > 0, "WAL position should be recorded");
}

// -------------------------------------------------------------------------
// P1 Audit: Snapshot Security Tests (DoS Prevention)
// -------------------------------------------------------------------------

#[test]
fn test_snapshot_malicious_entry_count_dos_prevention() {
    // Arrange - Create a malicious snapshot with huge entry_count
    // This is a DoS attack vector: claiming millions of entries → OOM
    let temp = TempDir::new().expect("Failed to create temp dir");
    let snapshot_path = temp.path().join("payloads.snapshot");

    // Create malicious snapshot: valid header but entry_count = u64::MAX
    let mut malicious_data = Vec::new();
    malicious_data.extend_from_slice(b"VSNP"); // Magic
    malicious_data.push(1); // Version
    malicious_data.extend_from_slice(&0u64.to_le_bytes()); // WAL pos
    malicious_data.extend_from_slice(&u64::MAX.to_le_bytes()); // MALICIOUS: huge entry_count
                                                               // Add fake CRC (will fail anyway)
    malicious_data.extend_from_slice(&0u32.to_le_bytes());

    std::fs::create_dir_all(temp.path()).expect("Create dir failed");
    std::fs::write(&snapshot_path, &malicious_data).expect("Write failed");

    // Also create an empty WAL so storage can be created
    let wal_path = temp.path().join("payloads.log");
    std::fs::write(&wal_path, []).expect("Create WAL failed");

    // Act - Should NOT crash or OOM, should fall back to WAL
    let result = LogPayloadStorage::new(temp.path());

    // Assert - Storage should be created (via WAL fallback), no panic/OOM
    assert!(
        result.is_ok(),
        "Should handle malicious snapshot gracefully"
    );
    let storage = result.unwrap();
    assert_eq!(storage.ids().len(), 0); // Empty because WAL is empty
}

#[test]
fn test_snapshot_truncated_data() {
    // Arrange - Create a truncated snapshot (header only, no entries/CRC)
    let temp = TempDir::new().expect("Failed to create temp dir");
    let snapshot_path = temp.path().join("payloads.snapshot");

    let mut truncated_data = Vec::new();
    truncated_data.extend_from_slice(b"VSNP"); // Magic
    truncated_data.push(1); // Version
    truncated_data.extend_from_slice(&100u64.to_le_bytes()); // WAL pos
    truncated_data.extend_from_slice(&10u64.to_le_bytes()); // 10 entries claimed
                                                            // No entries, no CRC - truncated!

    std::fs::create_dir_all(temp.path()).expect("Create dir failed");
    std::fs::write(&snapshot_path, &truncated_data).expect("Write failed");
    let wal_path = temp.path().join("payloads.log");
    std::fs::write(&wal_path, []).expect("Create WAL failed");

    // Act & Assert - Should handle truncated data gracefully
    let result = LogPayloadStorage::new(temp.path());
    assert!(
        result.is_ok(),
        "Should handle truncated snapshot gracefully"
    );
}

#[test]
fn test_snapshot_wrong_magic() {
    // Arrange - Create snapshot with wrong magic bytes
    let temp = TempDir::new().expect("Failed to create temp dir");
    let snapshot_path = temp.path().join("payloads.snapshot");

    let mut bad_magic = Vec::new();
    bad_magic.extend_from_slice(b"HACK"); // Wrong magic
    bad_magic.push(1);
    bad_magic.extend_from_slice(&0u64.to_le_bytes());
    bad_magic.extend_from_slice(&0u64.to_le_bytes());
    bad_magic.extend_from_slice(&0u32.to_le_bytes());

    std::fs::create_dir_all(temp.path()).expect("Create dir failed");
    std::fs::write(&snapshot_path, &bad_magic).expect("Write failed");
    let wal_path = temp.path().join("payloads.log");
    std::fs::write(&wal_path, []).expect("Create WAL failed");

    // Act & Assert - Should reject and fall back to WAL
    let result = LogPayloadStorage::new(temp.path());
    assert!(result.is_ok(), "Should handle wrong magic gracefully");
}

#[test]
fn test_snapshot_unsupported_version() {
    // Arrange - Create snapshot with future version
    let temp = TempDir::new().expect("Failed to create temp dir");
    let snapshot_path = temp.path().join("payloads.snapshot");

    let mut future_version = Vec::new();
    future_version.extend_from_slice(b"VSNP");
    future_version.push(255); // Future version
    future_version.extend_from_slice(&0u64.to_le_bytes());
    future_version.extend_from_slice(&0u64.to_le_bytes());
    future_version.extend_from_slice(&0u32.to_le_bytes());

    std::fs::create_dir_all(temp.path()).expect("Create dir failed");
    std::fs::write(&snapshot_path, &future_version).expect("Write failed");
    let wal_path = temp.path().join("payloads.log");
    std::fs::write(&wal_path, []).expect("Create WAL failed");

    // Act & Assert - Should reject and fall back to WAL
    let result = LogPayloadStorage::new(temp.path());
    assert!(
        result.is_ok(),
        "Should handle unsupported version gracefully"
    );
}

#[test]
fn test_snapshot_random_garbage() {
    // Arrange - Create snapshot with random garbage data
    let temp = TempDir::new().expect("Failed to create temp dir");
    let snapshot_path = temp.path().join("payloads.snapshot");

    // Random garbage
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let garbage: Vec<u8> = (0..100).map(|i| (i * 17 + 31) as u8).collect();

    std::fs::create_dir_all(temp.path()).expect("Create dir failed");
    std::fs::write(&snapshot_path, &garbage).expect("Write failed");
    let wal_path = temp.path().join("payloads.log");
    std::fs::write(&wal_path, []).expect("Create WAL failed");

    // Act & Assert - Should handle garbage gracefully
    let result = LogPayloadStorage::new(temp.path());
    assert!(result.is_ok(), "Should handle garbage data gracefully");
}

#[test]
fn test_snapshot_entry_count_overflow() {
    // Arrange - Create snapshot where entry_count * 16 would overflow usize
    let temp = TempDir::new().expect("Failed to create temp dir");
    let snapshot_path = temp.path().join("payloads.snapshot");

    // entry_count that would cause overflow when multiplied by 16
    let overflow_count = (usize::MAX / 16) as u64 + 1;

    let mut overflow_data = Vec::new();
    overflow_data.extend_from_slice(b"VSNP");
    overflow_data.push(1);
    overflow_data.extend_from_slice(&0u64.to_le_bytes());
    overflow_data.extend_from_slice(&overflow_count.to_le_bytes());
    overflow_data.extend_from_slice(&0u32.to_le_bytes());

    std::fs::create_dir_all(temp.path()).expect("Create dir failed");
    std::fs::write(&snapshot_path, &overflow_data).expect("Write failed");
    let wal_path = temp.path().join("payloads.log");
    std::fs::write(&wal_path, []).expect("Create WAL failed");

    // Act & Assert - Should NOT panic on overflow, should fall back to WAL
    let result = LogPayloadStorage::new(temp.path());
    assert!(result.is_ok(), "Should handle overflow gracefully");
}
