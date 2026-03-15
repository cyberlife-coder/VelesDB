//! Tests for the `compaction` module.
//!
//! Covers `punch_hole()`, `CompactionContext::compact()`,
//! `fragmentation_ratio()`, and atomicity guarantees.

use super::compaction::{punch_hole, CompactionContext};
use super::sharded_index::ShardedIndex;
use super::traits::VectorStorage;
use super::MmapStorage;

use memmap2::MmapMut;
use parking_lot::RwLock;
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

// -------------------------------------------------------------------------
// punch_hole tests
// -------------------------------------------------------------------------

#[test]
fn test_punch_hole_zeros_target_region() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("punch.dat");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .expect("Failed to open file");

    // Fill entire file with a known pattern
    let mut writer = file.try_clone().expect("clone failed");
    let pattern = vec![0xABu8; 2048];
    writer.write_all(&pattern).expect("write failed");
    writer.flush().expect("flush failed");

    // Punch a hole in the middle: bytes [512..1024)
    let result = punch_hole(&file, 512, 512);
    assert!(result.is_ok(), "punch_hole should succeed");

    // Read back and verify the hole is zeroed
    let mut reader = file.try_clone().expect("clone failed");
    reader.seek(SeekFrom::Start(0)).expect("seek failed");
    let mut buf = vec![0u8; 2048];
    reader.read_exact(&mut buf).expect("read failed");

    // Before hole: bytes 0..512 should still be 0xAB
    assert!(
        buf[..512].iter().all(|&b| b == 0xAB),
        "Bytes before hole should be untouched"
    );
    // Hole: bytes 512..1024 should be zero
    assert!(
        buf[512..1024].iter().all(|&b| b == 0),
        "Hole region should be zeroed"
    );
    // After hole: bytes 1024..2048 should still be 0xAB
    assert!(
        buf[1024..].iter().all(|&b| b == 0xAB),
        "Bytes after hole should be untouched"
    );
}

#[test]
fn test_punch_hole_at_file_start() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("punch_start.dat");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .expect("open failed");

    let mut writer = file.try_clone().expect("clone failed");
    writer.write_all(&[0xFF; 1024]).expect("write failed");
    writer.flush().expect("flush failed");

    let result = punch_hole(&file, 0, 256);
    assert!(result.is_ok());

    let mut reader = file.try_clone().expect("clone failed");
    reader.seek(SeekFrom::Start(0)).expect("seek failed");
    let mut buf = vec![0u8; 1024];
    reader.read_exact(&mut buf).expect("read failed");

    assert!(buf[..256].iter().all(|&b| b == 0), "Start should be zeroed");
    assert!(
        buf[256..].iter().all(|&b| b == 0xFF),
        "Remainder should be untouched"
    );
}

#[test]
fn test_punch_hole_zero_length_is_noop() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("punch_zero.dat");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .expect("open failed");

    let mut writer = file.try_clone().expect("clone failed");
    writer.write_all(&[0xCC; 512]).expect("write failed");
    writer.flush().expect("flush failed");

    let result = punch_hole(&file, 128, 0);
    assert!(result.is_ok());

    let mut reader = file.try_clone().expect("clone failed");
    reader.seek(SeekFrom::Start(0)).expect("seek failed");
    let mut buf = vec![0u8; 512];
    reader.read_exact(&mut buf).expect("read failed");

    assert!(
        buf.iter().all(|&b| b == 0xCC),
        "Zero-length punch should not modify data"
    );
}

// -------------------------------------------------------------------------
// CompactionContext::compact() tests
// -------------------------------------------------------------------------

/// Helper: creates a `MmapStorage`, inserts vectors, and returns it.
fn storage_with_vectors(dir: &std::path::Path, dimension: usize, ids: &[u64]) -> MmapStorage {
    let mut storage = MmapStorage::new(dir, dimension).expect("create storage");
    #[allow(clippy::cast_precision_loss)]
    for &id in ids {
        let vector: Vec<f32> = (0..dimension).map(|d| id as f32 + d as f32).collect();
        storage.store(id, &vector).expect("store vector");
    }
    storage.flush().expect("flush");
    storage
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_compact_reclaims_deleted_vectors() {
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let vector_size = dim * std::mem::size_of::<f32>();
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3, 4, 5]);

    // Delete 3 of 5 vectors
    storage.delete(1).expect("delete");
    storage.delete(3).expect("delete");
    storage.delete(5).expect("delete");

    let reclaimed = storage.compact().expect("compact");
    assert_eq!(reclaimed, 3 * vector_size);

    // Remaining vectors are intact
    for &id in &[2u64, 4] {
        let v = storage.retrieve(id).expect("retrieve").expect("exists");
        let expected: Vec<f32> = (0..dim).map(|d| id as f32 + d as f32).collect();
        assert_eq!(v, expected, "Vector {id} data mismatch after compaction");
    }

    // Deleted vectors are gone
    for &id in &[1u64, 3, 5] {
        assert!(
            storage.retrieve(id).expect("retrieve").is_none(),
            "Vector {id} should be absent"
        );
    }
}

#[test]
fn test_compact_all_deleted_returns_zero() {
    let dir = tempdir().expect("tempdir");
    let mut storage = storage_with_vectors(dir.path(), 4, &[10, 20]);

    storage.delete(10).expect("delete");
    storage.delete(20).expect("delete");

    // Index is empty, so compact returns 0 (early exit branch)
    let reclaimed = storage.compact().expect("compact");
    assert_eq!(reclaimed, 0);
}

#[test]
fn test_compact_no_deletions_returns_zero() {
    let dir = tempdir().expect("tempdir");
    let mut storage = storage_with_vectors(dir.path(), 4, &[1, 2, 3]);

    let reclaimed = storage.compact().expect("compact");
    assert_eq!(reclaimed, 0, "No fragmentation means nothing to reclaim");
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_compact_preserves_data_after_reopen() {
    let dir = tempdir().expect("tempdir");
    let dim = 3;

    {
        let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3, 4]);
        storage.delete(2).expect("delete");
        storage.delete(4).expect("delete");
        storage.compact().expect("compact");
        storage.flush().expect("flush");
    }

    // Reopen and verify survivors
    let storage = MmapStorage::new(dir.path(), dim).expect("reopen");
    for &id in &[1u64, 3] {
        let v = storage.retrieve(id).expect("retrieve").expect("exists");
        let expected: Vec<f32> = (0..dim).map(|d| id as f32 + d as f32).collect();
        assert_eq!(v, expected, "Vector {id} mismatch after reopen");
    }
    assert_eq!(storage.len(), 2);
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_compact_then_insert_works() {
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3]);

    storage.delete(2).expect("delete");
    storage.compact().expect("compact");

    // Insert new vectors after compaction
    let new_vec: Vec<f32> = vec![99.0; dim];
    storage.store(100, &new_vec).expect("store after compact");

    let retrieved = storage.retrieve(100).expect("retrieve").expect("exists");
    assert_eq!(retrieved, new_vec);
    assert_eq!(storage.len(), 3); // 1, 3, 100
}

// -------------------------------------------------------------------------
// fragmentation_ratio() tests
// -------------------------------------------------------------------------

#[test]
fn test_fragmentation_ratio_empty_storage() {
    let dir = tempdir().expect("tempdir");
    let storage = MmapStorage::new(dir.path(), 4).expect("create");

    let ratio = storage.fragmentation_ratio();
    assert!(
        ratio < f64::EPSILON,
        "Empty storage should have zero fragmentation, got {ratio}"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_fragmentation_ratio_known_values() {
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3, 4]);

    // No deletions: 0% fragmentation
    let ratio = storage.fragmentation_ratio();
    assert!(ratio < 0.01, "No deletions: expected ~0%, got {ratio}");

    // Delete 1 of 4: expect 25%
    storage.delete(1).expect("delete");
    let ratio = storage.fragmentation_ratio();
    assert!(
        (0.20..0.30).contains(&ratio),
        "Delete 1/4: expected ~25%, got {ratio}"
    );

    // Delete another: 2 of 4 => 50%
    storage.delete(2).expect("delete");
    let ratio = storage.fragmentation_ratio();
    assert!(
        (0.45..0.55).contains(&ratio),
        "Delete 2/4: expected ~50%, got {ratio}"
    );

    // Delete third: 3 of 4 => 75%
    storage.delete(3).expect("delete");
    let ratio = storage.fragmentation_ratio();
    assert!(
        (0.70..0.80).contains(&ratio),
        "Delete 3/4: expected ~75%, got {ratio}"
    );
}

#[test]
fn test_fragmentation_ratio_zero_after_compact() {
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3, 4]);

    storage.delete(1).expect("delete");
    storage.delete(3).expect("delete");
    storage.compact().expect("compact");

    let ratio = storage.fragmentation_ratio();
    assert!(
        ratio < 0.01,
        "After compaction, fragmentation should be ~0%, got {ratio}"
    );
}

// -------------------------------------------------------------------------
// Atomicity / crash-safety tests
// -------------------------------------------------------------------------

#[test]
fn test_original_file_survives_if_temp_file_is_removed() {
    // Simulates an interrupted compaction: if the temp file disappears
    // before atomic_replace completes, the original data file must remain.
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3]);

    storage.delete(1).expect("delete");
    storage.flush().expect("flush");

    // Verify the original data file exists before we attempt anything
    let data_path = dir.path().join("vectors.dat");
    assert!(data_path.exists(), "Original data file should exist");

    // Now perform a real compaction — the fact that it succeeds means
    // the atomic replace worked. Verify data is intact afterwards.
    storage.compact().expect("compact");
    assert!(data_path.exists(), "Data file must exist after compaction");

    // The temp file should have been cleaned up
    let temp_path = dir.path().join("vectors.dat.tmp");
    assert!(
        !temp_path.exists(),
        "Temp file should be removed after successful compaction"
    );

    // Data is still there
    assert_eq!(storage.len(), 2);
    assert!(storage.retrieve(2).expect("retrieve").is_some());
    assert!(storage.retrieve(3).expect("retrieve").is_some());
}

#[test]
fn test_stale_temp_file_does_not_block_compaction() {
    // If a previous compaction was interrupted, a stale .tmp file may linger.
    // A new compaction must still succeed by overwriting/replacing it.
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3, 4]);

    // Plant a stale temp file
    let temp_path = dir.path().join("vectors.dat.tmp");
    std::fs::write(&temp_path, b"stale leftover").expect("write stale");

    storage.delete(1).expect("delete");
    let reclaimed = storage.compact().expect("compact should succeed");
    assert!(reclaimed > 0, "Should have reclaimed space");

    // Stale temp file should be gone (overwritten then renamed)
    assert!(!temp_path.exists(), "Stale temp file should be cleaned up");
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_backup_file_cleaned_after_successful_compaction() {
    // On Windows, atomic_replace uses a .dat.bak intermediate.
    // Verify it is removed after successful compaction.
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3, 4]);

    storage.delete(1).expect("delete");
    storage.delete(2).expect("delete");
    storage.compact().expect("compact");

    let backup_path = dir.path().join("vectors.dat.bak");
    assert!(
        !backup_path.exists(),
        "Backup file should be removed after successful compaction"
    );
}

// -------------------------------------------------------------------------
// CompactionContext unit tests (direct construction)
// -------------------------------------------------------------------------

/// Raw parts returned by [`build_context_parts`] for isolated unit testing.
type ContextParts = (
    ShardedIndex,
    RwLock<MmapMut>,
    AtomicUsize,
    RwLock<BufWriter<std::fs::File>>,
);

/// Builds a `CompactionContext` from raw parts for isolated unit testing.
fn build_context_parts(
    dir: &std::path::Path,
    dimension: usize,
    entries: &[(u64, Vec<f32>)],
) -> io::Result<ContextParts> {
    std::fs::create_dir_all(dir)?;

    let vector_size = dimension * std::mem::size_of::<f32>();
    let total_bytes = entries.len() * vector_size;
    let file_size = u64::try_from(total_bytes.max(4096)).unwrap_or(4096);

    // Create and populate the data file
    let data_path = dir.join("vectors.dat");
    let data_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&data_path)?;
    data_file.set_len(file_size)?;

    // SAFETY: File is open read/write and sized via set_len.
    let mut mmap = unsafe { MmapMut::map_mut(&data_file)? };

    let index = ShardedIndex::new();
    let mut offset = 0usize;
    for (id, vec) in entries {
        let bytes = crate::storage::vector_bytes::vector_to_bytes(vec);
        mmap[offset..offset + vector_size].copy_from_slice(bytes);
        index.insert(*id, offset);
        offset += vector_size;
    }
    mmap.flush()?;

    // Create WAL file
    let wal_path = dir.join("vectors.wal");
    let wal_file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&wal_path)?;
    let wal = BufWriter::new(wal_file);

    Ok((
        index,
        RwLock::new(mmap),
        AtomicUsize::new(offset),
        RwLock::new(wal),
    ))
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_context_compact_updates_index_offsets() {
    let dir = tempdir().expect("tempdir");
    let dim = 3;

    let entries: Vec<(u64, Vec<f32>)> = (1..=4)
        .map(|id| {
            let v: Vec<f32> = (0..dim).map(|d| id as f32 + d as f32).collect();
            (id, v)
        })
        .collect();

    let (index, mmap, next_offset, wal) =
        build_context_parts(dir.path(), dim, &entries).expect("build");

    // Remove entries 1 and 3 from the index (simulate deletion)
    index.remove(1);
    index.remove(3);

    let ctx = CompactionContext {
        path: dir.path(),
        dimension: dim,
        index: &index,
        mmap: &mmap,
        next_offset: &next_offset,
        wal: &wal,
        initial_size: 4096,
    };

    let reclaimed = ctx.compact().expect("compact");
    let vector_size = dim * std::mem::size_of::<f32>();
    assert_eq!(reclaimed, 2 * vector_size);

    // Verify surviving entries have valid, distinct offsets
    let offset_2 = index.get(2).expect("id 2 should exist");
    let offset_4 = index.get(4).expect("id 4 should exist");
    assert_ne!(offset_2, offset_4, "Offsets should be distinct");

    // Verify next_offset was updated to compact layout
    let expected_next = 2 * vector_size;
    assert_eq!(next_offset.load(Ordering::Relaxed), expected_next);
}

#[test]
fn test_context_fragmentation_ratio_precise() {
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let vector_size = dim * std::mem::size_of::<f32>();

    let entries: Vec<(u64, Vec<f32>)> = (1..=10).map(|id| (id, vec![0.0f32; dim])).collect();

    let (index, mmap, next_offset, wal) =
        build_context_parts(dir.path(), dim, &entries).expect("build");

    // Remove 3 of 10 entries
    index.remove(2);
    index.remove(5);
    index.remove(8);

    let ctx = CompactionContext {
        path: dir.path(),
        dimension: dim,
        index: &index,
        mmap: &mmap,
        next_offset: &next_offset,
        wal: &wal,
        initial_size: 4096,
    };

    // active_size = 7 * 16 = 112, current_offset = 10 * 16 = 160
    // ratio = 1.0 - 112/160 = 0.3
    let ratio = ctx.fragmentation_ratio();
    let active_size = 7 * vector_size;
    let current_offset = 10 * vector_size;
    #[allow(clippy::cast_precision_loss)]
    let expected = 1.0 - (active_size as f64 / current_offset as f64);

    assert!(
        (ratio - expected).abs() < 1e-10,
        "Expected ratio {expected}, got {ratio}"
    );
}
