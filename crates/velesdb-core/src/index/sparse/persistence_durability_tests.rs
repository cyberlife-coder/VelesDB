//! Power-loss publication and recovery tests for sparse snapshots.

use tempfile::TempDir;

use super::inverted_index::SparseInvertedIndex;
use super::persistence::{compact, load_from_disk, wal_append_upsert};
use super::types::SparseVector;
use crate::storage::atomic_write::{AtomicWriteBoundary, FaultGuard};

fn vector(term: u32, weight: f32) -> SparseVector {
    SparseVector::new(vec![(term, weight)])
}

fn recoverable_case() -> (TempDir, SparseInvertedIndex, Vec<u8>) {
    let dir = TempDir::new().expect("test: temp dir");
    let wal_path = dir.path().join("sparse.wal");
    let index = SparseInvertedIndex::new();

    let first = vector(1, 1.0);
    wal_append_upsert(&wal_path, 1, &first).expect("test: seed WAL");
    index.insert(1, &first);
    compact(dir.path(), &index).expect("test: seed snapshot");

    let second = vector(2, 2.0);
    wal_append_upsert(&wal_path, 2, &second).expect("test: pending WAL");
    index.insert(2, &second);
    let wal_before = std::fs::read(&wal_path).expect("test: read WAL");
    (dir, index, wal_before)
}

fn assert_complete_recovery(dir: &TempDir) {
    let recovered = load_from_disk(dir.path())
        .expect("recovery must succeed")
        .expect("snapshot must exist");
    assert_eq!(recovered.doc_count(), 2);
    assert_eq!(recovered.get_all_postings(1)[0].doc_id, 1);
    assert_eq!(recovered.get_all_postings(2)[0].doc_id, 2);
}

#[test]
fn every_snapshot_temp_sync_failure_preserves_recovery() {
    for completed_syncs in 0..5 {
        let (dir, index, wal_before) = recoverable_case();
        let result = {
            let _fault =
                FaultGuard::inject_after(AtomicWriteBoundary::TemporaryFileSync, completed_syncs);
            compact(dir.path(), &index)
        };
        assert!(result.is_err(), "temp sync {completed_syncs} must fail");
        assert_eq!(
            std::fs::read(dir.path().join("sparse.wal")).unwrap(),
            wal_before
        );
        assert_complete_recovery(&dir);
    }
}

#[test]
fn every_generation_promotion_failure_preserves_recovery() {
    for completed_promotions in 0..5 {
        let (dir, index, wal_before) = recoverable_case();
        let result = {
            let _fault =
                FaultGuard::inject_after(AtomicWriteBoundary::Replacement, completed_promotions);
            compact(dir.path(), &index)
        };
        assert!(
            result.is_err(),
            "promotion {completed_promotions} must fail"
        );
        assert_eq!(
            std::fs::read(dir.path().join("sparse.wal")).unwrap(),
            wal_before
        );
        assert_complete_recovery(&dir);
    }
}

#[cfg(unix)]
#[test]
fn lost_first_manifest_falls_back_to_wal_only() {
    let dir = TempDir::new().expect("test: temp dir");
    let wal_path = dir.path().join("sparse.wal");
    let index = SparseInvertedIndex::new();
    let first = vector(1, 1.0);
    wal_append_upsert(&wal_path, 1, &first).expect("test: seed WAL");
    index.insert(1, &first);

    let result = {
        let _fault = FaultGuard::inject_after(AtomicWriteBoundary::ParentDirectorySync, 4);
        compact(dir.path(), &index)
    };
    assert!(result.is_err(), "manifest directory barrier must fail");

    std::fs::remove_file(dir.path().join("sparse.snapshot")).unwrap();
    let recovered = load_from_disk(dir.path()).unwrap().unwrap();
    assert_eq!(recovered.doc_count(), 1);
    assert_eq!(recovered.get_all_postings(1)[0].doc_id, 1);
}

#[cfg(unix)]
#[test]
fn every_directory_sync_failure_preserves_recovery() {
    for completed_syncs in 0..6 {
        let (dir, index, _) = recoverable_case();
        let result = {
            let _fault =
                FaultGuard::inject_after(AtomicWriteBoundary::ParentDirectorySync, completed_syncs);
            compact(dir.path(), &index)
        };
        assert!(
            result.is_err(),
            "directory sync {completed_syncs} must fail"
        );
        assert_complete_recovery(&dir);
    }
}

#[test]
fn append_after_wal_reset_failure_uses_committed_generation() {
    let (dir, index, _) = recoverable_case();
    let result = {
        let _fault = FaultGuard::inject_after(AtomicWriteBoundary::TemporaryFileSync, 4);
        compact(dir.path(), &index)
    };
    assert!(result.is_err(), "WAL reset sync failure must propagate");

    let third = vector(3, 3.0);
    wal_append_upsert(&dir.path().join("sparse.wal"), 3, &third)
        .expect("next append must rebase the stale WAL");

    let recovered = load_from_disk(dir.path()).unwrap().unwrap();
    assert_eq!(recovered.doc_count(), 3);
    assert_eq!(recovered.get_all_postings(3)[0].doc_id, 3);
}

#[test]
fn stale_wal_generation_is_not_replayed_over_new_snapshot() {
    let (dir, index, stale_wal) = recoverable_case();
    compact(dir.path(), &index).expect("test: publish next generation");

    std::fs::write(dir.path().join("sparse.wal"), stale_wal).expect("test: restore stale WAL");
    assert_complete_recovery(&dir);
}
