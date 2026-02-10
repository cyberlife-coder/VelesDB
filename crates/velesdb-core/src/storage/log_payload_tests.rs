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

// -------------------------------------------------------------------------
// TDD: WAL per-entry CRC32 tests [D-05]
// -------------------------------------------------------------------------

#[test]
fn test_wal_crc32_store_and_retrieve_intact() {
    // CRC32 should be transparent — store and retrieve works normally
    let (mut storage, _temp) = create_test_storage();
    let payload = json!({"name": "crc_test", "value": 123});

    storage.store(1, &payload).expect("Store failed");
    let retrieved = storage.retrieve(1).expect("Retrieve failed");

    assert_eq!(retrieved, Some(payload));
}

#[test]
fn test_wal_crc32_detects_payload_corruption_on_retrieve() {
    // Store an entry, corrupt the payload bytes on disk, verify error on retrieve
    let (mut storage, temp) = create_test_storage();
    let payload = json!({"important": "data", "value": 42});
    storage.store(1, &payload).expect("Store failed");

    // Get the WAL file and corrupt payload bytes
    let wal_path = temp.path().join("payloads.log");
    let mut wal_data = std::fs::read(&wal_path).expect("Read WAL failed");

    // WAL format: [marker=1: 1B] [id: 8B] [len: 4B] [crc32: 4B] [payload: N bytes]
    // Payload starts at offset 17 (1 + 8 + 4 + 4)
    // Flip bits in the payload area
    if wal_data.len() > 20 {
        wal_data[20] ^= 0xFF;
        wal_data[21] ^= 0xFF;
    }
    std::fs::write(&wal_path, &wal_data).expect("Write WAL failed");

    // Reopen storage (replay should detect corruption)
    let result = LogPayloadStorage::new(temp.path());
    match result {
        Ok(storage) => {
            // If replay didn't verify (seek-only mode), retrieve should detect
            let retrieve_result = storage.retrieve(1);
            assert!(
                retrieve_result.is_err(),
                "Corrupted payload should be detected via CRC32"
            );
        }
        Err(e) => {
            // Replay detected corruption — also acceptable
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        }
    }
}

#[test]
fn test_wal_crc32_multiple_entries_all_protected() {
    // All entries should have CRC32 protection
    let (mut storage, _temp) = create_test_storage();
    for i in 1..=10 {
        storage
            .store(i, &json!({"id": i, "data": format!("entry_{i}")}))
            .expect("Store failed");
    }

    // All entries should be retrievable
    for i in 1..=10 {
        let result = storage.retrieve(i).expect("Retrieve failed");
        assert!(result.is_some(), "Entry {i} should exist");
        assert_eq!(result.unwrap()["id"], i);
    }
}

#[test]
fn test_wal_crc32_survives_reopen() {
    // CRC32 entries should survive close/reopen (WAL replay)
    let temp = tempfile::TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create failed");
        for i in 1..=5 {
            storage.store(i, &json!({"id": i})).expect("Store failed");
        }
        storage.flush().expect("Flush failed");
    }

    // Reopen — replay should verify CRC32 and succeed
    let storage = LogPayloadStorage::new(temp.path()).expect("Reopen failed");
    assert_eq!(storage.ids().len(), 5);
    for i in 1..=5 {
        let val = storage
            .retrieve(i)
            .expect("Retrieve failed")
            .expect("exists");
        assert_eq!(val["id"], i);
    }
}

#[test]
fn test_wal_crc32_delete_entries_unaffected() {
    // Delete entries have no payload → no CRC32 needed
    let (mut storage, _temp) = create_test_storage();
    storage.store(1, &json!({"key": "val"})).expect("Store");
    storage.store(2, &json!({"key": "val2"})).expect("Store");
    storage.delete(1).expect("Delete");

    assert!(storage.retrieve(1).expect("Retrieve").is_none());
    assert!(storage.retrieve(2).expect("Retrieve").is_some());
}

#[test]
fn test_wal_crc32_empty_payload() {
    // Empty JSON object should have valid CRC32
    let (mut storage, _temp) = create_test_storage();
    let payload = json!({});
    storage.store(1, &payload).expect("Store failed");
    let retrieved = storage.retrieve(1).expect("Retrieve failed");
    assert_eq!(retrieved, Some(payload));
}

// -------------------------------------------------------------------------
// TDD: Batch flush tests [D-06]
// -------------------------------------------------------------------------

#[test]
fn test_store_batch_basic() {
    // store_batch should write multiple entries with a single flush
    let (mut storage, _temp) = create_test_storage();

    let p1 = serde_json::to_vec(&json!({"id": 1})).expect("serialize");
    let p2 = serde_json::to_vec(&json!({"id": 2})).expect("serialize");
    let p3 = serde_json::to_vec(&json!({"id": 3})).expect("serialize");

    let items: Vec<(u64, &[u8])> = vec![(1, &p1), (2, &p2), (3, &p3)];
    storage.store_batch(&items).expect("Batch store failed");

    // All entries should be retrievable
    for i in 1..=3 {
        let val = storage.retrieve(i).expect("Retrieve failed");
        assert!(val.is_some(), "Entry {i} should exist");
        assert_eq!(val.unwrap()["id"], i);
    }
}

#[test]
fn test_store_batch_crc_protected() {
    // Batch entries should also have CRC32 protection
    let temp = tempfile::TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create");
        let p1 = serde_json::to_vec(&json!({"batch": true, "id": 1})).expect("serialize");
        let p2 = serde_json::to_vec(&json!({"batch": true, "id": 2})).expect("serialize");
        let items: Vec<(u64, &[u8])> = vec![(1, &p1), (2, &p2)];
        storage.store_batch(&items).expect("Batch store");
        storage.flush().expect("Flush");
    }

    // Reopen — CRC32 should verify during replay
    let storage = LogPayloadStorage::new(temp.path()).expect("Reopen");
    assert_eq!(storage.ids().len(), 2);
}

#[test]
fn test_store_batch_empty() {
    // Empty batch should be a no-op
    let (mut storage, _temp) = create_test_storage();
    storage
        .store_batch(&[])
        .expect("Empty batch should succeed");
    assert_eq!(storage.ids().len(), 0);
}

#[test]
fn test_store_batch_mixed_with_single_store() {
    // Batch and single stores should coexist
    let (mut storage, _temp) = create_test_storage();

    storage.store(1, &json!({"single": true})).expect("Store");

    let p2 = serde_json::to_vec(&json!({"batch": true, "id": 2})).expect("serialize");
    let p3 = serde_json::to_vec(&json!({"batch": true, "id": 3})).expect("serialize");
    let items: Vec<(u64, &[u8])> = vec![(2, &p2), (3, &p3)];
    storage.store_batch(&items).expect("Batch store");

    assert_eq!(storage.ids().len(), 3);
    assert_eq!(storage.retrieve(1).unwrap().unwrap()["single"], true);
    assert_eq!(storage.retrieve(2).unwrap().unwrap()["batch"], true);
}

#[test]
fn test_store_batch_survives_reopen() {
    // Batch entries should survive close/reopen
    let temp = tempfile::TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("Create");
        let p1 = serde_json::to_vec(&json!({"id": 10})).expect("serialize");
        let p2 = serde_json::to_vec(&json!({"id": 20})).expect("serialize");
        let items: Vec<(u64, &[u8])> = vec![(10, &p1), (20, &p2)];
        storage.store_batch(&items).expect("Batch store");
        storage.flush().expect("Flush");
    }

    let storage = LogPayloadStorage::new(temp.path()).expect("Reopen");
    assert_eq!(storage.ids().len(), 2);
    assert_eq!(storage.retrieve(10).unwrap().unwrap()["id"], 10);
    assert_eq!(storage.retrieve(20).unwrap().unwrap()["id"], 20);
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
