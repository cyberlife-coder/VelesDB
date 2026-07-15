#![allow(
    clippy::similar_names, // Reason: clippy 1.90 flags idiomatic test bindings (dir/dim, ids/idx)
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for storage reliability fixes (Issues #316, #317, #318).

use super::sharded_index::ShardedIndex;
use super::*;
use rustc_hash::FxHashMap;
use serial_test::serial;
use std::io::Write as _;
use std::sync::Arc;
use tempfile::tempdir;

// ===========================================================================
// Issue #316: Atomic index swap during compaction
// ===========================================================================

#[test]
fn test_replace_all_atomic_no_intermediate_empty() {
    // Verify that replace_all swaps all entries atomically:
    // a concurrent reader should never see an empty index while
    // replace_all is in progress.
    let index = Arc::new(ShardedIndex::new());

    // Populate initial state
    for i in 0..100u64 {
        index.insert(i, i as usize * 16);
    }

    // Build replacement map
    let mut new_entries: FxHashMap<u64, usize> = FxHashMap::default();
    for i in 0..100u64 {
        new_entries.insert(i, i as usize * 32);
    }

    // Perform atomic replace
    index.replace_all(new_entries);

    // After replace_all, all entries should have new offsets
    for i in 0..100u64 {
        assert_eq!(
            index.get(i),
            Some(i as usize * 32),
            "ID {i} should have updated offset after replace_all"
        );
    }
    assert_eq!(index.len(), 100);
}

#[test]
fn test_replace_all_with_empty_map_clears_index() {
    let index = ShardedIndex::new();
    for i in 0..50u64 {
        index.insert(i, i as usize * 8);
    }

    index.replace_all(FxHashMap::default());
    assert!(
        index.is_empty(),
        "replace_all with empty map should clear index"
    );
}

#[test]
fn test_replace_all_concurrent_reader_sees_consistent_state() {
    // Spawn a reader thread that continuously checks index consistency
    // (either all old values or all new values, never a mix with empty).
    let index = Arc::new(ShardedIndex::new());
    for i in 0..64u64 {
        index.insert(i, i as usize * 10);
    }

    let reader_index = Arc::clone(&index);
    let reader = std::thread::spawn(move || {
        for _ in 0..10_000 {
            let mut found = 0usize;
            let mut missing = 0usize;
            for i in 0..64u64 {
                if reader_index.get(i).is_some() {
                    found += 1;
                } else {
                    missing += 1;
                }
            }
            // With atomic replace_all, we should never see a partially
            // empty state — either all found or (transiently) all missing
            // during the swap. In practice, the reader should always see
            // all 64 entries because replace_all holds all shard locks.
            assert!(
                found == 64 || found == 0,
                "Inconsistent state: found={found}, missing={missing}"
            );
        }
    });

    // Meanwhile, do many replace_all cycles
    for cycle in 0..100u64 {
        let mut new_entries: FxHashMap<u64, usize> = FxHashMap::default();
        for i in 0..64u64 {
            new_entries.insert(i, (cycle * 64 + i) as usize);
        }
        index.replace_all(new_entries);
    }

    reader.join().expect("reader thread should not panic");
}

#[test]
fn test_compaction_uses_atomic_swap() {
    // End-to-end: store, delete, compact — verify no data loss.
    let dir = tempdir().unwrap();
    let dim = 4;
    let mut storage = MmapStorage::new(dir.path(), dim).unwrap();

    for i in 0u64..20 {
        storage.store(i, &[i as f32; 4]).unwrap();
    }
    for i in 0u64..10 {
        storage.delete(i).unwrap();
    }

    let reclaimed = storage.compact().unwrap();
    assert!(reclaimed > 0);

    // All surviving vectors accessible
    for i in 10u64..20 {
        let v = storage.retrieve(i).unwrap();
        assert_eq!(v, Some(vec![i as f32; 4]));
    }
}

// ===========================================================================
// Issue #317: WAL replay for MmapStorage crash recovery
// ===========================================================================

#[test]
fn test_wal_replay_recovers_unflushed_stores() {
    // Simulate crash: store vectors, do NOT call flush(), drop, reopen.
    // The new CRC-framed WAL should allow recovery.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    {
        let mut storage = MmapStorage::new(&path, dim).unwrap();
        storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
        storage.store(2, &[4.0, 5.0, 6.0]).unwrap();

        // Flush WAL to disk but do NOT call storage.flush() (which persists index)
        storage.wal().write().flush().unwrap();
        storage.wal().write().get_ref().sync_all().unwrap();
        // Flush mmap too so vector bytes are on disk
        storage.mmap().write().flush().unwrap();

        // Do NOT call storage.flush() — simulates crash before index persistence
    }

    // Reopen — WAL replay should recover vectors
    let storage = MmapStorage::new(&path, dim).unwrap();
    let v1 = storage.retrieve(1).unwrap();
    let v2 = storage.retrieve(2).unwrap();
    assert_eq!(
        v1,
        Some(vec![1.0, 2.0, 3.0]),
        "Vector 1 should be recovered from WAL"
    );
    assert_eq!(
        v2,
        Some(vec![4.0, 5.0, 6.0]),
        "Vector 2 should be recovered from WAL"
    );
}

#[test]
fn test_wal_replay_recovers_deletes() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    {
        let mut storage = MmapStorage::new(&path, dim).unwrap();
        storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
        storage.store(2, &[4.0, 5.0, 6.0]).unwrap();
        storage.flush().unwrap(); // Persist both to index

        // Now delete one — written to WAL but NOT flushed to index
        storage.delete(1).unwrap();
        storage.wal().write().flush().unwrap();
        storage.wal().write().get_ref().sync_all().unwrap();
        // Do NOT call storage.flush() — crash
    }

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert!(
        storage.retrieve(1).unwrap().is_none(),
        "Deleted vector should not be recoverable"
    );
    assert_eq!(
        storage.retrieve(2).unwrap(),
        Some(vec![4.0, 5.0, 6.0]),
        "Non-deleted vector should survive"
    );
}

#[test]
fn test_wal_replay_skips_legacy_format() {
    // Write a legacy-format WAL (no CRC) and verify replay skips it.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::fs::create_dir_all(&path).unwrap();

    // Create an index with one entry
    let mut index: FxHashMap<u64, usize> = FxHashMap::default();
    index.insert(1, 0);
    let index_bytes = postcard::to_allocvec(&index).unwrap();
    std::fs::write(path.join("vectors.idx"), &index_bytes).unwrap();

    // Create data file with a vector at offset 0
    let data_path = path.join("vectors.dat");
    let dim = 3;
    let vector_bytes: Vec<u8> = [1.0f32, 2.0, 3.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let mut data = vec![0u8; 16 * 1024 * 1024]; // 16MB initial
    data[..vector_bytes.len()].copy_from_slice(&vector_bytes);
    std::fs::write(&data_path, &data).unwrap();

    // Write legacy WAL (no CRC): op=1, id=2, len, data
    let mut wal = Vec::new();
    wal.push(1u8);
    wal.extend_from_slice(&2u64.to_le_bytes());
    let vec_bytes: Vec<u8> = [7.0f32, 8.0, 9.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    wal.extend_from_slice(&(vec_bytes.len() as u32).to_le_bytes());
    wal.extend_from_slice(&vec_bytes);
    // No CRC appended — legacy format
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    // Open storage — legacy WAL should be skipped, only index data survives
    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.len(), 1, "Only indexed entry should exist");
    assert!(
        storage.retrieve(2).unwrap().is_none(),
        "Legacy WAL entry should not be replayed"
    );
}

#[test]
fn test_wal_replay_truncates_after_success() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    {
        let mut storage = MmapStorage::new(&path, dim).unwrap();
        storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
        storage.wal().write().flush().unwrap();
        storage.wal().write().get_ref().sync_all().unwrap();
        storage.mmap().write().flush().unwrap();
    }

    // Reopen triggers replay
    let _storage = MmapStorage::new(&path, dim).unwrap();

    // WAL should be truncated after replay
    let wal_len = std::fs::metadata(path.join("vectors.wal")).unwrap().len();
    assert_eq!(
        wal_len, 0,
        "WAL should be truncated after successful replay"
    );
}

#[test]
fn test_legacy_compaction_marker_skipped_mid_stream() {
    // Pre-fix versions appended a bare 0x04 byte to the WAL after compaction.
    // Replay must skip it (no payload) and keep going, so post-compaction
    // entries written by those versions are still recovered.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut wal = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    wal.push(4u8); // legacy compaction marker
    wal.extend_from_slice(&crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0])));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(
        storage.retrieve(2).unwrap(),
        Some(vec![4.0, 5.0, 6.0]),
        "entries after a legacy compaction marker must be replayed"
    );
}

#[test]
fn test_legacy_compaction_marker_leading_byte_detected() {
    // A WAL whose first byte is the legacy marker must still be detected as
    // CRC-framed so the entries behind the marker are recovered.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut wal = vec![4u8];
    wal.extend_from_slice(&crc_store_entry(7, &vec3_bytes([7.0, 8.0, 9.0])));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(
        storage.retrieve(7).unwrap(),
        Some(vec![7.0, 8.0, 9.0]),
        "a leading legacy marker must not disable WAL replay"
    );
}

// ===========================================================================
// Issue #318: Windows atomic_replace crash-safety (.bak recovery)
// ===========================================================================

#[test]
fn test_bak_recovery_restores_from_backup() {
    // Simulate: original gone, .bak exists -> restore from .bak
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::fs::create_dir_all(&path).unwrap();

    let data_path = path.join("vectors.dat");
    let bak_path = path.join("vectors.dat.bak");

    // Create a valid data file as the backup
    let dim = 3;
    let data = vec![0u8; 16 * 1024 * 1024];
    std::fs::write(&bak_path, &data).unwrap();

    // Create empty index and WAL
    std::fs::write(path.join("vectors.wal"), b"").unwrap();

    // No vectors.dat exists — should be restored from .bak
    assert!(!data_path.exists());
    assert!(bak_path.exists());

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.len(), 0); // Empty but opened successfully
    assert!(
        data_path.exists(),
        "vectors.dat should be restored from .bak"
    );
    assert!(!bak_path.exists(), ".bak should be cleaned up");
}

#[test]
fn test_bak_recovery_removes_stale_backup() {
    // Simulate: both original and .bak exist -> remove .bak
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::fs::create_dir_all(&path).unwrap();

    let data_path = path.join("vectors.dat");
    let bak_path = path.join("vectors.dat.bak");

    let data = vec![0u8; 16 * 1024 * 1024];
    std::fs::write(&data_path, &data).unwrap();
    std::fs::write(&bak_path, &data).unwrap();

    // Create empty WAL
    std::fs::write(path.join("vectors.wal"), b"").unwrap();

    let _storage = MmapStorage::new(&path, 3).unwrap();
    assert!(
        !bak_path.exists(),
        ".bak should be removed when original exists"
    );
}

#[test]
fn test_tmp_recovery_removes_incomplete_compaction() {
    // Simulate: .tmp file from incomplete compaction -> remove it
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::fs::create_dir_all(&path).unwrap();

    let data_path = path.join("vectors.dat");
    let tmp_path = path.join("vectors.dat.tmp");

    let data = vec![0u8; 16 * 1024 * 1024];
    std::fs::write(&data_path, &data).unwrap();
    std::fs::write(&tmp_path, b"incomplete compaction data").unwrap();
    std::fs::write(path.join("vectors.wal"), b"").unwrap();

    let _storage = MmapStorage::new(&path, 3).unwrap();
    assert!(!tmp_path.exists(), ".tmp should be removed on startup");
}

// ===========================================================================
// #898: WAL replay hardening — OOM caps, ordering, overflow, corruption policy
// ===========================================================================

/// Builds a valid CRC32-framed store entry: `[op=1][id][len][data][crc]`.
fn crc_store_entry(id: u64, data: &[u8]) -> Vec<u8> {
    use crate::storage::log_payload::crc32_hash;
    let mut frame = Vec::new();
    frame.push(1u8);
    frame.extend_from_slice(&id.to_le_bytes());
    frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
    frame.extend_from_slice(data);
    let crc = crc32_hash(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

fn vec3_bytes(v: [f32; 3]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

#[test]
fn test_898_replay_rejects_oversized_wal_length_no_huge_alloc() {
    // A store record declaring a 4 GiB payload but with only a few real bytes
    // must be rejected as a torn/corrupt tail rather than allocating 4 GiB.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    // Force CRC-framed detection with a valid first entry, then a bogus one.
    let mut wal = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    // Oversized record: op=1, id=2, len=u32::MAX, but no payload follows.
    wal.push(1u8);
    wal.extend_from_slice(&2u64.to_le_bytes());
    wal.extend_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    // Must not panic / OOM; first entry recovers, oversized tail is dropped.
    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert!(storage.retrieve(2).unwrap().is_none());
}

#[test]
fn test_898_replay_torn_tail_recovers_prior_entries() {
    // Two valid entries followed by a truncated (torn) third record.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut wal = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    wal.extend_from_slice(&crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0])));
    // Torn tail: only an op byte + partial id, no len/data/crc.
    wal.push(1u8);
    wal.extend_from_slice(&7u64.to_le_bytes()[..3]);
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(storage.retrieve(2).unwrap(), Some(vec![4.0, 5.0, 6.0]));
    assert_eq!(storage.len(), 2, "torn tail must not corrupt prior entries");
}

#[test]
// Reads/writes the process-global `wal_replay_corrupt_entries` metric; serialize
// the whole metric-asserting group so parallel tests can't pollute the counter.
#[serial(wal_corrupt_metric)]
fn test_898_replay_midstream_crc_corruption_skips_and_continues() {
    // First entry valid, second fully-framed but CRC-corrupt, third valid.
    // Policy: skip the corrupt mid-stream entry, recover the rest.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut bad = crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0]));
    let last = bad.len() - 1;
    bad[last] ^= 0xFF; // flip CRC -> mid-stream corruption

    let mut wal = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    wal.extend_from_slice(&bad);
    wal.extend_from_slice(&crc_store_entry(3, &vec3_bytes([7.0, 8.0, 9.0])));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let before = crate::metrics::global_guardrails_metrics()
        .wal_replay_corrupt_entries
        .load(std::sync::atomic::Ordering::Relaxed);

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert!(
        storage.retrieve(2).unwrap().is_none(),
        "corrupt mid-stream entry must be skipped"
    );
    assert_eq!(
        storage.retrieve(3).unwrap(),
        Some(vec![7.0, 8.0, 9.0]),
        "entries after a mid-stream corruption must still be recovered"
    );

    let after = crate::metrics::global_guardrails_metrics()
        .wal_replay_corrupt_entries
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(after > before, "corrupt-entry metric must be incremented");
}

#[test]
fn test_898_replay_grows_mmap_no_silent_gap() {
    // A WAL that places vectors past the 16 MB initial mmap must grow the
    // mapping and recover every vector — never silently drop one.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;
    let vec_size = dim * 4;

    // Build an index whose highest offset is already near 16 MB so the next
    // replayed vector lands beyond the initial mapping.
    let near_cap = 16 * 1024 * 1024 - vec_size; // last slot in the initial map
    let mut index: FxHashMap<u64, usize> = FxHashMap::default();
    index.insert(1, near_cap);
    std::fs::write(
        path.join("vectors.idx"),
        postcard::to_allocvec(&index).unwrap(),
    )
    .unwrap();

    // Data file sized to initial 16 MB, vector 1 written at near_cap.
    let mut data = vec![0u8; 16 * 1024 * 1024];
    data[near_cap..near_cap + vec_size].copy_from_slice(&vec3_bytes([1.0, 2.0, 3.0]));
    std::fs::write(path.join("vectors.dat"), &data).unwrap();

    // WAL stores a NEW vector (id=2) which will be placed at next_offset
    // (== 16 MB), beyond the initial mapping -> replay must grow.
    let wal = crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0]));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(
        storage.retrieve(2).unwrap(),
        Some(vec![4.0, 5.0, 6.0]),
        "vector beyond initial mmap must be recovered, not dropped"
    );
}

#[test]
// Reads/writes the process-global corrupt-entry metric; serialize with the rest
// of the metric-asserting group.
#[serial(wal_corrupt_metric)]
fn test_replay_corrupt_first_record_recovers_following_entries() {
    // Regression: a CRC-framed WAL whose FIRST record is corrupt must NOT be
    // misclassified as legacy (which silently discarded the ENTIRE WAL). Format
    // detection now scans for any CRC-valid record, so the corrupt first record
    // is treated as mid-stream corruption (skip + metric) and every valid record
    // behind it is still recovered.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    // First record: fully framed but with a broken CRC.
    let mut bad = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    let last = bad.len() - 1;
    bad[last] ^= 0xFF;

    let mut wal = bad;
    wal.extend_from_slice(&crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0])));
    wal.extend_from_slice(&crc_store_entry(3, &vec3_bytes([7.0, 8.0, 9.0])));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let before = crate::metrics::global_guardrails_metrics()
        .wal_replay_corrupt_entries
        .load(std::sync::atomic::Ordering::Relaxed);

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert!(
        storage.retrieve(1).unwrap().is_none(),
        "corrupt first record must be skipped, not applied"
    );
    assert_eq!(
        storage.retrieve(2).unwrap(),
        Some(vec![4.0, 5.0, 6.0]),
        "records after a corrupt first record must be recovered, not discarded with the whole WAL"
    );
    assert_eq!(storage.retrieve(3).unwrap(), Some(vec![7.0, 8.0, 9.0]));

    let after = crate::metrics::global_guardrails_metrics()
        .wal_replay_corrupt_entries
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        after > before,
        "a corrupt first record must be counted as mid-stream corruption"
    );
}

#[test]
fn test_replay_growth_persists_data_file_size_for_reopen() {
    // FIX (fsync ordering): after a replay that GROWS the data file, the grown
    // size and rebuilt index must be mutually consistent so a subsequent reopen's
    // load_index bound check (offset <= data file size) passes. Exercises the
    // sync_all-before-persist-index ordering added to `replay_wal`. (A true crash
    // between truncation and data-file durability cannot be injected here without
    // a fault harness; this guards the open -> replay+grow -> reopen round-trip.)
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;
    let vec_size = dim * 4;

    // Index whose highest offset is the last slot of the 16 MB initial map.
    let near_cap = 16 * 1024 * 1024 - vec_size;
    let mut index: FxHashMap<u64, usize> = FxHashMap::default();
    index.insert(1, near_cap);
    std::fs::write(
        path.join("vectors.idx"),
        postcard::to_allocvec(&index).unwrap(),
    )
    .unwrap();

    let mut data = vec![0u8; 16 * 1024 * 1024];
    data[near_cap..near_cap + vec_size].copy_from_slice(&vec3_bytes([1.0, 2.0, 3.0]));
    std::fs::write(path.join("vectors.dat"), &data).unwrap();

    // A NEW vector lands at next_offset (== 16 MB), beyond the initial mapping,
    // forcing replay to grow the data file.
    let wal = crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0]));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    // First open replays, grows, persists the index, then truncates the WAL.
    {
        let storage = MmapStorage::new(&path, dim).unwrap();
        assert_eq!(storage.retrieve(2).unwrap(), Some(vec![4.0, 5.0, 6.0]));
        assert_eq!(
            std::fs::metadata(path.join("vectors.wal")).unwrap().len(),
            0,
            "WAL truncated only after data file + index made durable"
        );
    }

    // The persisted index now references an offset in the GROWN region; a reopen
    // must load it without load_index rejecting the offset as past the data file,
    // proving the grown size persisted consistently with the index.
    let reopened = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(reopened.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(reopened.retrieve(2).unwrap(), Some(vec![4.0, 5.0, 6.0]));
}

#[test]
fn test_flush_full_after_live_growth_persists_data_file_size_for_reopen() {
    // FIX (fsync ordering): flush_full() persists vectors.idx after a LIVE growth
    // (ensure_capacity -> set_len during a store). The grown data-file size must be
    // fsync'd BEFORE the index is persisted so a subsequent reopen's load_index
    // bound check (offset <= data file size) passes. Guards the
    // sync_all-before-persist-index ordering added to `flush_full`, mirroring the
    // replay path. (A true crash between growth and data-file durability cannot be
    // injected here without a fault harness; this guards the
    // open -> store+grow -> flush_full -> reopen round-trip.)
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;
    let vec_size = dim * 4;

    // Seed an index whose highest offset is the last slot of the 16 MB initial
    // map, so next_offset lands exactly at the map boundary and the next store
    // forces a live growth.
    let near_cap = 16 * 1024 * 1024 - vec_size;
    let mut index: FxHashMap<u64, usize> = FxHashMap::default();
    index.insert(1, near_cap);
    std::fs::write(
        path.join("vectors.idx"),
        postcard::to_allocvec(&index).unwrap(),
    )
    .unwrap();

    let mut data = vec![0u8; 16 * 1024 * 1024];
    data[near_cap..near_cap + vec_size].copy_from_slice(&vec3_bytes([1.0, 2.0, 3.0]));
    std::fs::write(path.join("vectors.dat"), &data).unwrap();

    // Open (no WAL to replay), then store a NEW vector at next_offset (== 16 MB),
    // forcing ensure_capacity to grow the data file live. flush_full then persists
    // the index referencing the grown offset.
    {
        let mut storage = MmapStorage::new(&path, dim).unwrap();
        storage.store(2, &[4.0, 5.0, 6.0]).unwrap();
        assert!(
            std::fs::metadata(path.join("vectors.dat")).unwrap().len() > 16 * 1024 * 1024,
            "storing past the initial map must have grown the data file live"
        );
        storage.flush_full().unwrap();
    }

    // The persisted index now references an offset in the GROWN region; a reopen
    // must load it without load_index rejecting the offset as past the data file,
    // proving flush_full made the grown size durable consistently with the index.
    let reopened = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(reopened.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(reopened.retrieve(2).unwrap(), Some(vec![4.0, 5.0, 6.0]));
}

#[test]
fn test_898_load_index_rejects_out_of_bounds_offset() {
    // A corrupt index offset that points past the data file must be rejected,
    // not silently accepted (which would wrap into an OOB read later).
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    // Tiny data file but an index offset far beyond it.
    std::fs::write(path.join("vectors.dat"), vec![0u8; 64]).unwrap();
    let mut index: FxHashMap<u64, usize> = FxHashMap::default();
    index.insert(1, 1_000_000);
    std::fs::write(
        path.join("vectors.idx"),
        postcard::to_allocvec(&index).unwrap(),
    )
    .unwrap();
    std::fs::write(path.join("vectors.wal"), b"").unwrap();

    let result = MmapStorage::new(&path, dim);
    assert!(
        result.is_err(),
        "index offset beyond data file must be rejected as corrupt"
    );
}

#[test]
fn test_898_fsync_store_persists_wal_before_ok() {
    // DurabilityMode::Fsync single-store path must leave the entry on disk
    // (flushed past the BufWriter) by the time store() returns Ok.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut storage = MmapStorage::new_with_durability(&path, dim, DurabilityMode::Fsync).unwrap();
    storage.store(1, &[1.0, 2.0, 3.0]).unwrap();

    // Inspect the WAL file directly without any further flush/drop: a full
    // CRC-framed store entry (17 + 12 bytes) must already be present.
    let wal_len = std::fs::metadata(path.join("vectors.wal")).unwrap().len();
    assert!(
        wal_len >= (17 + dim * 4) as u64,
        "Fsync store must persist the WAL entry before returning Ok (got {wal_len} bytes)"
    );
}

#[test]
fn test_898_valid_roundtrip_and_crash_recovery_still_works() {
    // Sanity: normal store -> simulated crash (no flush_full) -> reopen recovers.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    {
        let mut storage = MmapStorage::new(&path, dim).unwrap();
        storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
        storage.store(2, &[4.0, 5.0, 6.0]).unwrap();
        storage.flush().unwrap();
        // Drop without flush_index() — simulates a crash before idx persist.
    }

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(storage.retrieve(2).unwrap(), Some(vec![4.0, 5.0, 6.0]));
    assert_eq!(storage.len(), 2);

    // WAL must be cleared after the recovery truncation.
    let wal_len = std::fs::metadata(path.join("vectors.wal")).unwrap().len();
    assert_eq!(wal_len, 0, "WAL truncated only after mmap+idx made durable");
}

// ===========================================================================
// #898 follow-up: durable delete before hole-punch, offset-reserve ordering,
// and torn-tail vs mid-stream CRC classification.
// ===========================================================================

/// Builds a valid CRC32-framed delete entry: `[op=2][id][crc]`.
fn crc_delete_entry(id: u64) -> Vec<u8> {
    use crate::storage::log_payload::crc32_hash;
    let mut frame = Vec::new();
    frame.push(2u8);
    frame.extend_from_slice(&id.to_le_bytes());
    let crc = crc32_hash(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

/// Reads the global mid-stream corrupt-WAL-entry counter.
fn corrupt_entry_count() -> u64 {
    crate::metrics::global_guardrails_metrics()
        .wal_replay_corrupt_entries
        .load(std::sync::atomic::Ordering::Relaxed)
}

#[test]
fn test_898b_fsync_delete_persists_wal_before_punch_hole() {
    // The fix: under DurabilityMode::Fsync, delete() must make the WAL delete
    // record durable BEFORE the destructive punch_hole. We assert the record is
    // already on disk the instant delete() returns — no drop/flush in between.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut storage = MmapStorage::new_with_durability(&path, dim, DurabilityMode::Fsync).unwrap();
    storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
    storage.flush_full().unwrap(); // persist idx + truncate WAL

    let wal_before = std::fs::metadata(path.join("vectors.wal")).unwrap().len();
    storage.delete(1).unwrap();
    let wal_after = std::fs::metadata(path.join("vectors.wal")).unwrap().len();

    // A CRC-framed delete record is op(1)+id(8)+crc(4) = 13 bytes; it must be
    // physically present (flushed past the BufWriter) before delete() returns.
    assert_eq!(
        wal_after - wal_before,
        13,
        "Fsync delete must persist the WAL delete record before punch_hole"
    );
}

#[test]
fn test_898b_delete_survives_crash_no_zero_resurrection() {
    // Faithful crash scenario via the replay seam: a durable store record
    // followed by a durable delete record for the same id. On reopen, replay
    // must apply both in order -> the id stays deleted and does NOT come back
    // as a zero vector.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut wal = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    wal.extend_from_slice(&crc_delete_entry(1));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert!(
        storage.retrieve(1).unwrap().is_none(),
        "deleted id must stay deleted after replay, not resurrect as zeros"
    );
    assert_eq!(storage.len(), 0);
}

#[test]
fn test_898b_valid_delete_normal_recovery_no_regression() {
    // Regression guard: a normal store + delete + flush cycle still recovers
    // with the id absent after reopen.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    {
        let mut storage = MmapStorage::new(&path, dim).unwrap();
        storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
        storage.store(2, &[4.0, 5.0, 6.0]).unwrap();
        storage.delete(1).unwrap();
        storage.flush().unwrap();
    }

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert!(
        storage.retrieve(1).unwrap().is_none(),
        "id 1 must stay deleted"
    );
    assert_eq!(storage.retrieve(2).unwrap(), Some(vec![4.0, 5.0, 6.0]));
    assert_eq!(storage.len(), 1);
}

#[test]
fn test_898b_store_offset_overflow_leaves_next_offset_unchanged() {
    // The fix: the overflow guard must run BEFORE next_offset is advanced, so a
    // rejected store leaves the allocator watermark exactly where it was.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;
    let vector_size = dim * std::mem::size_of::<f32>();

    let mut storage = MmapStorage::new(&path, dim).unwrap();

    // Force next_offset so close to usize::MAX that offset + vector_size wraps.
    let poisoned = usize::MAX - (vector_size - 1);
    storage
        .next_offset()
        .store(poisoned, std::sync::atomic::Ordering::SeqCst);

    let before = storage
        .next_offset()
        .load(std::sync::atomic::Ordering::SeqCst);
    let result = storage.store(999, &[1.0, 2.0, 3.0]);
    let after = storage
        .next_offset()
        .load(std::sync::atomic::Ordering::SeqCst);

    assert!(result.is_err(), "overflowing store offset must be rejected");
    assert_eq!(
        before, after,
        "next_offset must not advance on the overflow error path"
    );
}

#[test]
fn test_898_store_batch_offset_overflow_leaves_next_offset_unchanged() {
    // Same guarantee as the single-store path, for the batch allocator: an
    // overflowing batch must be rejected BEFORE next_offset is advanced.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;
    let vector_size = dim * std::mem::size_of::<f32>();

    let mut storage = MmapStorage::new(&path, dim).unwrap();
    let poisoned = usize::MAX - (vector_size - 1);
    storage
        .next_offset()
        .store(poisoned, std::sync::atomic::Ordering::SeqCst);

    let before = storage
        .next_offset()
        .load(std::sync::atomic::Ordering::SeqCst);
    let v = [1.0_f32, 2.0, 3.0];
    let result = storage.store_batch(&[(999_u64, &v[..])]);
    let after = storage
        .next_offset()
        .load(std::sync::atomic::Ordering::SeqCst);

    assert!(result.is_err(), "overflowing batch store must be rejected");
    assert_eq!(
        before, after,
        "next_offset must not advance on the batch overflow error path"
    );
}

#[test]
#[serial(wal_corrupt_metric)]
fn test_898b_torn_tail_crc_fail_at_eof_no_corrupt_metric() {
    // A fully-framed but CRC-failing record that is the LAST record is a normal
    // post-crash torn tail: replay stops cleanly, recovers prior entries, and
    // must NOT increment the corrupt-entry metric (no false bit-rot alert).
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut tail = crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0]));
    let last = tail.len() - 1;
    tail[last] ^= 0xFF; // flip CRC on the final record

    let mut wal = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    wal.extend_from_slice(&tail);
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let before = corrupt_entry_count();

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert!(
        storage.retrieve(2).unwrap().is_none(),
        "torn-tail record must be dropped"
    );

    assert_eq!(
        before,
        corrupt_entry_count(),
        "a CRC-failing torn tail at EOF must NOT raise a corruption alert"
    );
}

#[test]
#[serial(wal_corrupt_metric)]
fn test_898b_midstream_crc_fail_with_valid_after_increments_metric() {
    // Counterpart to the torn-tail test: a CRC-failing record followed by a
    // validly framed record IS genuine mid-stream corruption and MUST increment
    // the corrupt-entry metric while still recovering the trailing valid entry.
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 3;

    let mut bad = crc_store_entry(2, &vec3_bytes([4.0, 5.0, 6.0]));
    let last = bad.len() - 1;
    bad[last] ^= 0xFF; // flip CRC -> corruption

    let mut wal = crc_store_entry(1, &vec3_bytes([1.0, 2.0, 3.0]));
    wal.extend_from_slice(&bad);
    wal.extend_from_slice(&crc_store_entry(3, &vec3_bytes([7.0, 8.0, 9.0])));
    std::fs::write(path.join("vectors.wal"), &wal).unwrap();

    let before = corrupt_entry_count();

    let storage = MmapStorage::new(&path, dim).unwrap();
    assert_eq!(storage.retrieve(1).unwrap(), Some(vec![1.0, 2.0, 3.0]));
    assert!(storage.retrieve(2).unwrap().is_none());
    assert_eq!(storage.retrieve(3).unwrap(), Some(vec![7.0, 8.0, 9.0]));

    assert!(
        corrupt_entry_count() > before,
        "a CRC failure with valid framing after it must increment the metric"
    );
}
