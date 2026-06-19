//! Crash-safety tests for `vectors.idx` persistence.
//!
//! Audit 2026-06 (cluster C2, finding 3): rewriting `vectors.idx` in place
//! via `File::create` right after compaction durably truncated the WAL left
//! a crash window where the index ends up 0-byte/torn with an empty WAL —
//! the database became permanently unopenable and the vectors unrecoverable.
//! These tests pin the atomic staged-rename persistence (`vectors.idx.new`)
//! and the 0-byte-index open hardening.

use super::traits::VectorStorage;
use super::MmapStorage;

use tempfile::tempdir;

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
fn test_open_with_zero_byte_idx_replays_wal() {
    // GIVEN the crash footprint of a torn in-place index rewrite from
    // pre-atomic versions (File::create truncated vectors.idx, the process
    // died before the write): a 0-byte vectors.idx next to an intact WAL.
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    {
        let storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3]);
        drop(storage); // crash: vectors.idx never written by flush_index
    }
    std::fs::write(dir.path().join("vectors.idx"), b"").expect("plant 0-byte idx");

    // WHEN reopening
    let storage = MmapStorage::new(dir.path(), dim)
        .expect("a 0-byte vectors.idx must be treated as absent, not brick open()");

    // THEN the WAL replay rebuilds every vector.
    for id in [1u64, 2, 3] {
        assert!(
            storage.retrieve(id).expect("retrieve").is_some(),
            "vector {id} must be recovered from the WAL"
        );
    }
    assert_eq!(storage.len(), 3);
}

#[test]
fn test_open_after_compaction_with_torn_idx_does_not_brick() {
    // GIVEN the exact post-compaction crash footprint of the legacy flow:
    // compact() committed (vectors.idx promoted, WAL durably empty), then a
    // redundant in-place File::create rewrite of vectors.idx was interrupted,
    // leaving a 0-byte index, an empty WAL and no repair artifacts.
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    {
        let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3, 4]);
        storage.delete(2).expect("delete");
        storage.delete(4).expect("delete");
        assert!(storage.compact().expect("compact") > 0, "must reclaim");
    }
    std::fs::write(dir.path().join("vectors.idx"), b"").expect("plant torn idx");

    // WHEN reopening — before the atomic-persist fix this failed InvalidData
    // forever (unreadable index, empty WAL, nothing for recovery to promote).
    // THEN open succeeds: the 0-byte file carries no information and is
    // treated as absent. The fixed write path (staged rename, no rewrite
    // after the compaction commit) can no longer produce this footprint.
    let storage = MmapStorage::new(dir.path(), dim).expect("reopen must not brick open()");
    assert_eq!(
        storage.len(),
        0,
        "0-byte idx treated as absent + durably-empty WAL: recovered storage must be empty (no ghost entries) yet openable"
    );
    drop(storage);
}

#[test]
fn test_flush_index_writes_through_dedicated_staging_file() {
    // GIVEN a stale vectors.idx.new left behind by a crash between the
    // staged write and the rename of a previous index persist.
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2]);
    let staging = dir.path().join("vectors.idx.new");
    std::fs::write(&staging, b"stale-partial-write").expect("plant staging file");

    // WHEN the index is persisted.
    storage.flush_full().expect("flush_full");

    // THEN the staging file was consumed by the atomic rename — proving the
    // persist goes through vectors.idx.new instead of truncating
    // vectors.idx in place — and the persisted index is valid on reopen.
    assert!(
        !staging.exists(),
        "vectors.idx must be replaced by renaming the staged vectors.idx.new, \
         not rewritten in place"
    );
    drop(storage);
    let storage = MmapStorage::new(dir.path(), dim).expect("reopen");
    assert_eq!(storage.len(), 2);
}

#[test]
fn test_kill_between_staged_idx_write_and_rename_keeps_old_idx() {
    // GIVEN a storage whose vectors.idx is valid and durable.
    let dir = tempdir().expect("tempdir");
    let dim = 4;
    {
        let mut storage = storage_with_vectors(dir.path(), dim, &[1, 2, 3]);
        storage.flush_full().expect("flush_full");
    }
    // AND a kill between the staged write and the rename: vectors.idx.new
    // exists (possibly torn), vectors.idx untouched by the interrupted
    // persist.
    std::fs::write(dir.path().join("vectors.idx.new"), [0xAB, 0xCD])
        .expect("plant torn staging file");

    // WHEN reopening.
    let storage = MmapStorage::new(dir.path(), dim).expect("reopen");

    // THEN the previous index is intact and every vector resolves; the torn
    // staging file is never promoted.
    assert_eq!(storage.len(), 3);
    for id in [1u64, 2, 3] {
        assert!(
            storage.retrieve(id).expect("retrieve").is_some(),
            "vector {id} must resolve through the untouched vectors.idx"
        );
    }
}
