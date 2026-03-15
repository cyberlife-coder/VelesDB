//! Tests for sparse index persistence: WAL, compaction, and loading.

#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use tempfile::tempdir;

use super::inverted_index::SparseInvertedIndex;
use super::persistence::*;
use super::types::SparseVector;

fn make_vector(pairs: Vec<(u32, f32)>) -> SparseVector {
    SparseVector::new(pairs)
}

#[test]
fn test_wal_write_and_replay() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("sparse.wal");

    let index1 = SparseInvertedIndex::new();
    // Insert 100 vectors and write WAL entries
    for i in 0..100u64 {
        let v = make_vector(vec![(1, 1.0), (2, 0.5 + i as f32 * 0.01)]);
        index1.insert(i, &v);
        wal_append_upsert(&wal_path, i, &v).unwrap();
    }

    // Create fresh index and replay
    let index2 = SparseInvertedIndex::new();
    let count = wal_replay(&wal_path, &index2).unwrap();
    assert_eq!(count, 100);
    assert_eq!(index2.doc_count(), 100);

    // Verify postings match
    let p1 = index1.get_all_postings(1);
    let p2 = index2.get_all_postings(1);
    assert_eq!(p1.len(), p2.len());
    for (a, b) in p1.iter().zip(p2.iter()) {
        assert_eq!(a.doc_id, b.doc_id);
        assert!((a.weight - b.weight).abs() < f32::EPSILON);
    }
}

#[test]
fn test_wal_truncated_entry() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("sparse.wal");

    // Write one valid entry
    let v = make_vector(vec![(1, 1.0)]);
    wal_append_upsert(&wal_path, 42, &v).unwrap();

    // Append 5 random bytes (simulating truncation)
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&wal_path)
            .unwrap();
        f.write_all(&[0xFF, 0x00, 0xAA, 0xBB, 0xCC]).unwrap();
    }

    // Replay should recover the valid entry
    let index = SparseInvertedIndex::new();
    let count = wal_replay(&wal_path, &index).unwrap();
    assert_eq!(count, 1);
    assert_eq!(index.doc_count(), 1);

    let postings = index.get_all_postings(1);
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].doc_id, 42);
}

#[test]
fn test_wal_delete_replay() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("sparse.wal");

    let v = make_vector(vec![(1, 1.0)]);
    wal_append_upsert(&wal_path, 1, &v).unwrap();
    wal_append_upsert(&wal_path, 2, &v).unwrap();
    wal_append_delete(&wal_path, 1).unwrap();

    let index = SparseInvertedIndex::new();
    let count = wal_replay(&wal_path, &index).unwrap();
    assert_eq!(count, 3);
    // doc_count is 1 (2 inserts - 1 delete)
    assert_eq!(index.doc_count(), 1);

    let postings = index.get_all_postings(1);
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].doc_id, 2);
}

#[test]
fn test_compaction_round_trip() {
    let dir = tempdir().unwrap();

    let index1 = SparseInvertedIndex::new();
    for i in 0..500u64 {
        let v = make_vector(vec![
            (i as u32 % 50, 1.0 + (i as f32) * 0.001),
            (100 + i as u32 % 20, 0.5),
        ]);
        index1.insert(i, &v);
    }

    // Compact to disk
    compact(dir.path(), &index1).unwrap();

    // Load from disk
    let loaded = load_from_disk(dir.path()).unwrap();
    assert!(loaded.is_some());
    let index2 = loaded.unwrap();

    assert_eq!(index2.doc_count(), 500);

    // Verify search results match for a sample term
    let p1 = index1.get_all_postings(5);
    let p2 = index2.get_all_postings(5);
    assert_eq!(p1.len(), p2.len());
    for (a, b) in p1.iter().zip(p2.iter()) {
        assert_eq!(a.doc_id, b.doc_id);
        assert!((a.weight - b.weight).abs() < f32::EPSILON);
    }
}

#[test]
fn test_empty_directory_returns_none() {
    let dir = tempdir().unwrap();
    let result = load_from_disk(dir.path()).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_full_restart_simulation() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("sparse.wal");

    // Phase 1: Insert and compact some vectors
    let index1 = SparseInvertedIndex::new();
    for i in 0..50u64 {
        let v = make_vector(vec![(1, 1.0), (2, 2.0)]);
        index1.insert(i, &v);
    }
    compact(dir.path(), &index1).unwrap();

    // Phase 2: More inserts via WAL only (simulating in-flight mutations)
    for i in 50..60u64 {
        let v = make_vector(vec![(1, 3.0), (3, 1.0)]);
        wal_append_upsert(&wal_path, i, &v).unwrap();
    }

    // Phase 3: Simulate restart — load from disk + replay WAL
    let loaded = load_from_disk(dir.path()).unwrap();
    assert!(loaded.is_some());
    let index2 = loaded.unwrap();

    // Should have 50 compacted + 10 replayed = 60
    assert_eq!(index2.doc_count(), 60);

    // Term 1: all 60 docs
    let p1 = index2.get_all_postings(1);
    assert_eq!(p1.len(), 60);

    // Term 3: only docs 50..60
    let p3 = index2.get_all_postings(3);
    assert_eq!(p3.len(), 10);
}

#[test]
fn test_meta_contains_correct_values() {
    let dir = tempdir().unwrap();

    let index = SparseInvertedIndex::new();
    for i in 0..25u64 {
        let v = make_vector(vec![(i as u32 % 5, 1.0), (10, 0.5)]);
        index.insert(i, &v);
    }
    compact(dir.path(), &index).unwrap();

    // Read meta directly
    let meta_data = std::fs::read(dir.path().join("sparse.meta")).unwrap();
    let meta: SparseMeta = postcard::from_bytes(&meta_data).unwrap();
    assert_eq!(meta.version, 1);
    assert_eq!(meta.doc_count, 25);
    // 5 terms (0..4) + term 10 = 6 terms
    assert_eq!(meta.term_count, 6);
}

#[test]
fn test_wal_missing_file_returns_zero() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("nonexistent.wal");
    let index = SparseInvertedIndex::new();
    let count = wal_replay(&wal_path, &index).unwrap();
    assert_eq!(count, 0);
}

/// Simulates a crash that left a `.tmp` file behind from a previous interrupted compaction.
///
/// Verifies that `load_from_disk` ignores stale `.tmp` artefacts and correctly recovers
/// state from the WAL alone (no `sparse.meta` present — WAL-only scenario).
#[test]
fn test_partial_compaction_crash_recovery() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("sparse.wal");

    // Insert 5 distinct vectors and record them only in the WAL (no compaction).
    for i in 0..5u64 {
        let v = make_vector(vec![(i as u32, 1.0 + i as f32 * 0.1)]);
        wal_append_upsert(&wal_path, i, &v).unwrap();
    }

    // Simulate an interrupted compaction: the .tmp file exists but the final sparse.idx
    // and sparse.meta files were never atomically renamed into place.
    let tmp_path = dir.path().join("sparse.idx.tmp");
    std::fs::write(&tmp_path, b"garbage partial write").unwrap();

    // sparse.meta must NOT exist so load_from_disk follows the WAL-only path.
    assert!(!dir.path().join("sparse.meta").exists());

    // load_from_disk must ignore the .tmp file and recover from the WAL.
    let loaded = load_from_disk(dir.path()).unwrap();
    assert!(
        loaded.is_some(),
        "WAL-only load should return Some after partial compaction crash"
    );
    let index = loaded.unwrap();

    // All 5 WAL-inserted documents must be present.
    assert_eq!(index.doc_count(), 5);

    // Verify each term has exactly one posting with the correct doc_id.
    for i in 0..5u64 {
        let postings = index.get_all_postings(i as u32);
        assert_eq!(
            postings.len(),
            1,
            "term {i} should have exactly one posting"
        );
        assert_eq!(postings[0].doc_id, i);
    }

    // The stale .tmp file must still be present (load_from_disk must not delete it).
    assert!(
        tmp_path.exists(),
        "load_from_disk must not remove stale .tmp artefacts"
    );
}

#[test]
fn test_compaction_truncates_wal() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("sparse.wal");

    let index = SparseInvertedIndex::new();
    let v = make_vector(vec![(1, 1.0)]);
    index.insert(0, &v);
    wal_append_upsert(&wal_path, 0, &v).unwrap();

    // WAL should have content
    assert!(std::fs::metadata(&wal_path).unwrap().len() > 0);

    compact(dir.path(), &index).unwrap();

    // WAL should be truncated
    assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), 0);
}
