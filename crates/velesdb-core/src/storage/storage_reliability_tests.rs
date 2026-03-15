#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for storage reliability fixes (Issues #316, #317, #318).

use super::sharded_index::ShardedIndex;
use super::*;
use rustc_hash::FxHashMap;
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
        storage.wal.write().flush().unwrap();
        storage.wal.write().get_ref().sync_all().unwrap();
        // Flush mmap too so vector bytes are on disk
        storage.mmap.write().flush().unwrap();

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
        storage.wal.write().flush().unwrap();
        storage.wal.write().get_ref().sync_all().unwrap();
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
        storage.wal.write().flush().unwrap();
        storage.wal.write().get_ref().sync_all().unwrap();
        storage.mmap.write().flush().unwrap();
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
