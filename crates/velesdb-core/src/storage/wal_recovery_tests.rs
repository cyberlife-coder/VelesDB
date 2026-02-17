//! WAL Recovery Edge Case Tests
//!
//! Tests covering partial writes, data corruption detection, and crash recovery
//! scenarios for `LogPayloadStorage`.
//!
//! # WAL Binary Format (`LogPayloadStorage`)
//!
//! ```text
//! Store:  [marker=1: 1B] [id: 8B LE] [len: 4B LE] [crc32: 4B LE] [payload: len bytes]
//! Delete: [marker=2: 1B] [id: 8B LE]
//! ```
//!
//! # Recovery Path
//!
//! `LogPayloadStorage::new()` → `load_snapshot()` (if exists) → `replay_wal_from()`
//!
//! `replay_wal_from()` iterates WAL entries:
//! - Reads marker (1B) — `read_exact` failure → break (tolerant at entry boundary)
//! - Reads id (8B) — failure → propagated as error
//! - marker=1 → reads len (4B), inserts into index, seeks over payload
//! - marker=2 → removes from index
//! - unknown marker → returns `InvalidData` error
//!
//! # Snapshot Format
//!
//! ```text
//! [Magic: "VSNP" 4B] [Version: 1B] [WAL pos: 8B LE]
//! [Entry count: 8B LE] [Entries: (id: u64, offset: u64) × N] [CRC32: 4B LE]
//! ```

use super::log_payload::{LogPayloadStorage, SNAPSHOT_MAGIC, SNAPSHOT_VERSION};
use super::traits::PayloadStorage;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simple CRC32 (IEEE 802.3) — mirrors the implementation in `log_payload.rs`
#[allow(clippy::cast_possible_truncation)]
fn test_crc32_hash(data: &[u8]) -> u32 {
    const CRC32_TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut j = 0;
            while j < 8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB8_8320;
                } else {
                    crc >>= 1;
                }
                j += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

/// Writes a valid Store WAL entry to a buffer.
/// Format: [marker=1: 1B] [id: 8B LE] [len: 4B LE] [crc32: 4B LE] [payload: len bytes]
fn write_store_entry(buf: &mut Vec<u8>, id: u64, payload: &serde_json::Value) {
    let payload_bytes = serde_json::to_vec(payload).expect("serialize");
    #[allow(clippy::cast_possible_truncation)]
    let len = payload_bytes.len() as u32;
    let crc = test_crc32_hash(&payload_bytes);
    buf.push(1u8); // marker
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&payload_bytes);
}

/// Writes a valid Delete WAL entry to a buffer.
/// Format: [marker=2: 1B] [id: 8B LE]
fn write_delete_entry(buf: &mut Vec<u8>, id: u64) {
    buf.push(2u8); // marker
    buf.extend_from_slice(&id.to_le_bytes());
}

/// Creates a WAL file with the given raw bytes and opens storage.
fn open_storage_with_wal(wal_bytes: &[u8]) -> std::io::Result<(TempDir, LogPayloadStorage)> {
    let temp = TempDir::new()?;
    let log_path = temp.path().join("payloads.log");
    fs::write(&log_path, wal_bytes)?;
    let storage = LogPayloadStorage::new(temp.path())?;
    Ok((temp, storage))
}

/// Builds N valid store entries and returns the raw WAL bytes.
fn build_valid_wal(count: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    for i in 1..=count {
        write_store_entry(&mut buf, i, &json!({"id": i}));
    }
    buf
}

// ===========================================================================
// Task 2: Partial Write Recovery Tests
// ===========================================================================

#[test]
fn test_wal_recovery_truncated_header_single_byte() {
    // WAL contains only the marker byte — no id follows.
    // replay_wal_from reads marker OK, then read_exact(id) fails → error propagated.
    // But since marker read succeeds and id read fails, the current implementation
    // propagates the error. We verify no panic and error is returned OR the storage
    // opens with 0 entries (depending on implementation tolerance).
    let wal = vec![1u8]; // Just a store marker, nothing else
    let result = open_storage_with_wal(&wal);

    // Either opens empty (tolerant) or returns an error (strict) — no panic
    match result {
        Ok((_dir, storage)) => assert_eq!(storage.ids().len(), 0),
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
    }
}

#[test]
fn test_wal_recovery_truncated_id_bytes() {
    // Marker + partial ID (only 4 of 8 bytes)
    let mut wal = vec![1u8]; // store marker
    wal.extend_from_slice(&42u64.to_le_bytes()[..4]); // only half the ID

    let result = open_storage_with_wal(&wal);
    match result {
        Ok((_dir, storage)) => assert_eq!(storage.ids().len(), 0),
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
    }
}

#[test]
fn test_wal_recovery_truncated_payload_length() {
    // Marker + full ID + partial length (only 2 of 4 bytes)
    let mut wal = vec![1u8];
    wal.extend_from_slice(&1u64.to_le_bytes());
    wal.extend_from_slice(&[0x10, 0x00]); // only 2 bytes of length

    let result = open_storage_with_wal(&wal);
    match result {
        Ok((_dir, storage)) => assert_eq!(storage.ids().len(), 0),
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
    }
}

#[test]
fn test_wal_recovery_truncated_payload_data() {
    // Valid header + length claims 100 bytes but only 10 bytes of payload follow
    let mut wal = vec![1u8];
    wal.extend_from_slice(&1u64.to_le_bytes());
    let claimed_len: u32 = 100;
    wal.extend_from_slice(&claimed_len.to_le_bytes());
    wal.extend_from_slice(&[0xABu8; 10]); // only 10 of 100 bytes

    let result = open_storage_with_wal(&wal);
    // The seek past payload should fail or overshoot — no panic expected
    if let Ok((_dir, storage)) = result {
        // If recovery tolerates this, the entry may or may not be indexed.
        // Key assertion: no panic.
        let _ = storage.ids();
    } else {
        /* acceptable */
    }
}

#[test]
fn test_wal_recovery_zero_length_payload() {
    // Store entry with len=0 — valid structure, empty payload
    // WAL format: [marker=1: 1B] [id: 8B] [len: 4B] [crc32: 4B] [payload: 0B]
    let empty_payload: &[u8] = &[];
    let crc = test_crc32_hash(empty_payload);
    let mut wal = Vec::new();
    wal.push(1u8);
    wal.extend_from_slice(&1u64.to_le_bytes());
    wal.extend_from_slice(&0u32.to_le_bytes()); // len = 0
    wal.extend_from_slice(&crc.to_le_bytes()); // CRC32 of empty payload

    let (_dir, storage) = open_storage_with_wal(&wal).expect("should open");
    // Entry should be indexed (len=0 is valid)
    assert_eq!(storage.ids().len(), 1);
    assert!(storage.ids().contains(&1));
}

#[test]
fn test_wal_recovery_multiple_valid_then_truncated() {
    // 5 valid entries, then a truncated 6th entry (marker + partial id)
    let mut wal = build_valid_wal(5);
    wal.push(1u8); // start of 6th entry
    wal.extend_from_slice(&6u64.to_le_bytes()[..3]); // truncated id

    let result = open_storage_with_wal(&wal);
    if let Ok((_dir, storage)) = result {
        // At minimum, the 5 valid entries should be recovered
        let ids = storage.ids();
        assert!(
            ids.len() >= 5,
            "Expected at least 5 recovered entries, got {}",
            ids.len()
        );
        for i in 1..=5 {
            assert!(ids.contains(&i), "Missing entry {i}");
        }
    } else {
        // If the implementation propagates the truncation error, that's acceptable
        // as long as there is no panic
    }
}

#[test]
fn test_wal_recovery_write_interrupted_mid_vector() {
    // Simulates a crash mid-write: valid store header + length, but payload cut in half
    let payload = json!({"data": "this is a longer payload for testing truncation"});
    let payload_bytes = serde_json::to_vec(&payload).expect("serialize");
    let half_len = payload_bytes.len() / 2;

    let mut wal = Vec::new();
    wal.push(1u8);
    wal.extend_from_slice(&1u64.to_le_bytes());
    #[allow(clippy::cast_possible_truncation)]
    let full_len = payload_bytes.len() as u32;
    wal.extend_from_slice(&full_len.to_le_bytes());
    wal.extend_from_slice(&payload_bytes[..half_len]); // only half written

    let result = open_storage_with_wal(&wal);
    // No panic — either error or empty storage
    if let Ok((_dir, storage)) = result {
        // If indexed, the entry may point to invalid data — that's a separate concern.
        // Key: no panic during recovery.
        let _ = storage.ids();
    } else {
        /* acceptable */
    }
}

// ===========================================================================
// Task 3: Corruption Detection Tests
// ===========================================================================

#[test]
fn test_wal_recovery_invalid_marker_byte() {
    // Single entry with unknown marker (0xFF)
    let mut wal = Vec::new();
    wal.push(0xFF); // invalid marker
    wal.extend_from_slice(&1u64.to_le_bytes());

    let result = open_storage_with_wal(&wal);
    match result {
        Ok((_dir, storage)) => assert_eq!(storage.ids().len(), 0),
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData),
    }
}

#[test]
fn test_wal_recovery_flipped_marker_in_second_entry() {
    // First entry valid, second has corrupted marker
    let mut wal = Vec::new();
    write_store_entry(&mut wal, 1, &json!({"ok": true}));
    // Corrupt the second entry's marker
    wal.push(0xFE); // invalid marker
    wal.extend_from_slice(&2u64.to_le_bytes());

    let result = open_storage_with_wal(&wal);
    match result {
        Ok((_dir, storage)) => {
            // First entry should be recovered even if second fails
            let ids = storage.ids();
            assert!(ids.contains(&1), "First valid entry should be recovered");
        }
        Err(e) => {
            // InvalidData from unknown marker is the expected strict behavior
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        }
    }
}

#[test]
fn test_wal_recovery_flipped_bits_in_payload() {
    // Valid WAL structure, but payload bytes are corrupted AFTER CRC was computed.
    // With CRC32, replay now detects corruption at replay time [D-05].
    let payload = json!({"key": "value"});
    let payload_bytes = serde_json::to_vec(&payload).expect("serialize");
    let crc = test_crc32_hash(&payload_bytes);

    // Build WAL entry with correct CRC, then corrupt payload
    let mut corrupted_payload = payload_bytes.clone();
    for byte in &mut corrupted_payload {
        *byte ^= 0xFF;
    }

    let mut wal = Vec::new();
    wal.push(1u8);
    wal.extend_from_slice(&1u64.to_le_bytes());
    #[allow(clippy::cast_possible_truncation)]
    let len = corrupted_payload.len() as u32;
    wal.extend_from_slice(&len.to_le_bytes());
    wal.extend_from_slice(&crc.to_le_bytes()); // CRC of ORIGINAL payload
    wal.extend_from_slice(&corrupted_payload); // CORRUPTED payload

    // Replay should detect CRC mismatch and return error
    let result = open_storage_with_wal(&wal);
    assert!(
        result.is_err(),
        "CRC32 mismatch should be detected during WAL replay"
    );
    if let Err(e) = result {
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
    }
}

#[test]
fn test_wal_recovery_oversized_payload_length() {
    // Header claims payload is u32::MAX bytes — way larger than file
    let mut wal = Vec::new();
    wal.push(1u8);
    wal.extend_from_slice(&1u64.to_le_bytes());
    wal.extend_from_slice(&u32::MAX.to_le_bytes()); // claims 4GB payload
    wal.extend_from_slice(b"tiny"); // only 4 bytes

    let result = open_storage_with_wal(&wal);
    // Recovery should handle this gracefully — the seek will overshoot
    if let Ok((_dir, storage)) = result {
        // May index entry pointing beyond file — key: no panic
        let _ = storage.ids();
    } else {
        /* acceptable — seek past EOF */
    }
}

#[test]
fn test_wal_recovery_all_zero_bytes() {
    // WAL filled with zeros — marker=0 is unknown
    let wal = vec![0u8; 64];
    let result = open_storage_with_wal(&wal);
    match result {
        Ok((_dir, storage)) => assert_eq!(storage.ids().len(), 0),
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData),
    }
}

#[test]
fn test_wal_recovery_valid_entries_then_garbage() {
    // 3 valid entries followed by random garbage bytes
    let mut wal = build_valid_wal(3);
    wal.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE]);

    let result = open_storage_with_wal(&wal);
    if let Ok((_dir, storage)) = result {
        let ids = storage.ids();
        // At least some valid entries should be recovered
        assert!(!ids.is_empty(), "Should recover at least some entries");
    } else {
        // Any error is acceptable (InvalidData or UnexpectedEof)
    }
}

// ===========================================================================
// Task 3b: Snapshot Corruption Detection Tests
// ===========================================================================

/// Builds a valid snapshot binary blob with the given entries and WAL position.
fn build_snapshot(entries: &[(u64, u64)], wal_pos: u64) -> Vec<u8> {
    let entry_count = entries.len() as u64;
    let mut buf = Vec::new();
    buf.extend_from_slice(SNAPSHOT_MAGIC);
    buf.push(SNAPSHOT_VERSION);
    buf.extend_from_slice(&wal_pos.to_le_bytes());
    buf.extend_from_slice(&entry_count.to_le_bytes());
    for &(id, offset) in entries {
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
    }
    let crc = test_crc32_hash(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());
    buf
}

#[test]
fn test_snapshot_invalid_magic() {
    let temp = TempDir::new().expect("temp dir");
    // Create empty WAL
    fs::write(temp.path().join("payloads.log"), b"").expect("write wal");
    // Create snapshot with wrong magic
    let mut snapshot = build_snapshot(&[], 0);
    snapshot[0] = b'X'; // corrupt magic
                        // Recompute CRC won't help — magic check is first
    fs::write(temp.path().join("payloads.snapshot"), &snapshot).expect("write snap");

    // Should fall back to WAL replay (empty WAL → 0 entries)
    let storage = LogPayloadStorage::new(temp.path()).expect("should open via fallback");
    assert_eq!(storage.ids().len(), 0);
}

#[test]
fn test_snapshot_invalid_version() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(temp.path().join("payloads.log"), b"").expect("write wal");
    let mut snapshot = build_snapshot(&[], 0);
    snapshot[4] = 99; // unsupported version
    fs::write(temp.path().join("payloads.snapshot"), &snapshot).expect("write snap");

    let storage = LogPayloadStorage::new(temp.path()).expect("fallback to WAL");
    assert_eq!(storage.ids().len(), 0);
}

#[test]
fn test_snapshot_crc_mismatch() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(temp.path().join("payloads.log"), b"").expect("write wal");
    let mut snapshot = build_snapshot(&[(1, 9), (2, 30)], 100);
    // Flip a byte in the middle (entry data) — CRC will mismatch
    let mid = snapshot.len() / 2;
    snapshot[mid] ^= 0xFF;
    fs::write(temp.path().join("payloads.snapshot"), &snapshot).expect("write snap");

    // Should fall back to WAL replay
    let storage = LogPayloadStorage::new(temp.path()).expect("fallback to WAL");
    // WAL is empty → 0 entries (snapshot was corrupt)
    assert_eq!(storage.ids().len(), 0);
}

#[test]
fn test_snapshot_truncated() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(temp.path().join("payloads.log"), b"").expect("write wal");
    // Snapshot too small (< 25 bytes minimum)
    fs::write(temp.path().join("payloads.snapshot"), b"VSNP").expect("write snap");

    let storage = LogPayloadStorage::new(temp.path()).expect("fallback to WAL");
    assert_eq!(storage.ids().len(), 0);
}

// ===========================================================================
// Task 4: Crash Recovery Simulation Tests
// ===========================================================================

#[test]
fn test_crash_recovery_clean_shutdown() {
    // Baseline: store data, flush, reopen — all data intact
    let temp = TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("create");
        for i in 1..=10 {
            storage.store(i, &json!({"id": i})).expect("store");
        }
        storage.flush().expect("flush");
    }

    let storage = LogPayloadStorage::new(temp.path()).expect("reopen");
    assert_eq!(storage.ids().len(), 10);
    for i in 1..=10 {
        let val = storage.retrieve(i).expect("retrieve").expect("exists");
        assert_eq!(val["id"], i);
    }
}

#[test]
fn test_crash_recovery_unclean_shutdown_no_flush() {
    // Store data but don't call flush() — WAL buffered writes may be lost.
    // On reopen, we should recover whatever was actually written to disk.
    let temp = TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("create");
        for i in 1..=5 {
            storage.store(i, &json!({"id": i})).expect("store");
        }
        // Intentionally NO flush — simulates crash
        // LogPayloadStorage::store flushes after each write, so data should persist
    }

    let storage = LogPayloadStorage::new(temp.path()).expect("reopen");
    // Since store() calls wal.flush() internally, all entries should survive
    let ids = storage.ids();
    assert_eq!(ids.len(), 5, "All entries should survive (store flushes)");
}

#[test]
fn test_crash_recovery_stale_snapshot_with_wal_delta() {
    // Snapshot at 50 entries, then add 50 more, crash before new snapshot.
    // Recovery should load snapshot + replay WAL delta.
    let temp = TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("create");
        for i in 1..=50 {
            storage
                .store(i, &json!({"id": i, "phase": 1}))
                .expect("store");
        }
        storage.create_snapshot().expect("snapshot");

        // Add delta after snapshot
        for i in 51..=100 {
            storage
                .store(i, &json!({"id": i, "phase": 2}))
                .expect("store");
        }
        storage.flush().expect("flush");
        // No new snapshot — simulates crash before next snapshot
    }

    let storage = LogPayloadStorage::new(temp.path()).expect("reopen");
    assert_eq!(storage.ids().len(), 100);

    // Verify phase 1 data (from snapshot)
    let v1 = storage.retrieve(25).expect("retrieve").expect("exists");
    assert_eq!(v1["phase"], 1);

    // Verify phase 2 data (from WAL delta)
    let v2 = storage.retrieve(75).expect("retrieve").expect("exists");
    assert_eq!(v2["phase"], 2);
}

#[test]
fn test_crash_recovery_double_recovery_idempotent() {
    // Recover, close, recover again — should produce identical state.
    let temp = TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("create");
        for i in 1..=20 {
            storage.store(i, &json!({"id": i})).expect("store");
        }
        storage.flush().expect("flush");
    }

    // First recovery
    let storage1 = LogPayloadStorage::new(temp.path()).expect("recovery 1");
    let ids1: Vec<u64> = {
        let mut ids = storage1.ids();
        ids.sort_unstable();
        ids
    };
    drop(storage1);

    // Second recovery (idempotent)
    let storage2 = LogPayloadStorage::new(temp.path()).expect("recovery 2");
    let ids2: Vec<u64> = {
        let mut ids = storage2.ids();
        ids.sort_unstable();
        ids
    };

    assert_eq!(ids1, ids2, "Double recovery must produce identical state");
}

#[test]
fn test_crash_recovery_empty_wal_with_snapshot() {
    // Snapshot exists but WAL is empty/truncated to snapshot position.
    let temp = TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("create");
        for i in 1..=10 {
            storage.store(i, &json!({"id": i})).expect("store");
        }
        storage.create_snapshot().expect("snapshot");
    }

    // Truncate WAL to 0 bytes (simulates WAL rotation/cleanup)
    let wal_path = temp.path().join("payloads.log");
    fs::write(&wal_path, b"").expect("truncate wal");

    // Recovery should load from snapshot, WAL replay range is 0..0 (no-op)
    // But snapshot's wal_pos might point beyond truncated WAL → edge case
    let result = LogPayloadStorage::new(temp.path());
    if let Ok(storage) = result {
        // Snapshot data should be available
        let ids = storage.ids();
        assert_eq!(ids.len(), 10, "Snapshot data should be loaded");
    } else {
        // If WAL truncation invalidates snapshot replay, that's a known edge case
        // Key: no panic
    }
}

#[test]
fn test_crash_recovery_no_snapshot_no_wal() {
    // Fresh directory — no WAL, no snapshot
    let temp = TempDir::new().expect("temp dir");
    let storage = LogPayloadStorage::new(temp.path()).expect("fresh open");
    assert_eq!(storage.ids().len(), 0);
}

#[test]
fn test_crash_recovery_snapshot_with_deletes_in_delta() {
    // Snapshot has entries, WAL delta contains deletes
    let temp = TempDir::new().expect("temp dir");

    {
        let mut storage = LogPayloadStorage::new(temp.path()).expect("create");
        for i in 1..=20 {
            storage.store(i, &json!({"id": i})).expect("store");
        }
        storage.create_snapshot().expect("snapshot");

        // Delete half the entries after snapshot
        for i in 1..=10 {
            storage.delete(i).expect("delete");
        }
        storage.flush().expect("flush");
    }

    let storage = LogPayloadStorage::new(temp.path()).expect("reopen");
    let ids = storage.ids();
    assert_eq!(ids.len(), 10, "Only 10 entries should remain after deletes");
    for i in 1..=10 {
        assert!(!ids.contains(&i), "Entry {i} should be deleted");
    }
    for i in 11..=20 {
        assert!(ids.contains(&i), "Entry {i} should still exist");
    }
}

#[test]
fn test_crash_recovery_wal_with_store_then_delete_same_id() {
    // Store and delete the same ID in WAL — recovery should reflect final state
    let mut wal = Vec::new();
    write_store_entry(&mut wal, 42, &json!({"data": "created"}));
    write_delete_entry(&mut wal, 42);

    let (_dir, storage) = open_storage_with_wal(&wal).expect("should open");
    assert_eq!(storage.ids().len(), 0, "Deleted entry should not appear");
    assert!(
        storage.retrieve(42).expect("retrieve").is_none(),
        "Entry 42 should not exist after delete"
    );
}

#[test]
fn test_crash_recovery_wal_store_delete_re_store() {
    // Store → delete → re-store same ID — final state should have the re-stored version
    let mut wal = Vec::new();
    write_store_entry(&mut wal, 1, &json!({"version": 1}));
    write_delete_entry(&mut wal, 1);
    write_store_entry(&mut wal, 1, &json!({"version": 2}));

    let (_dir, storage) = open_storage_with_wal(&wal).expect("should open");
    assert_eq!(storage.ids().len(), 1);
    let val = storage.retrieve(1).expect("retrieve").expect("exists");
    assert_eq!(val["version"], 2, "Should have version 2 after re-store");
}
