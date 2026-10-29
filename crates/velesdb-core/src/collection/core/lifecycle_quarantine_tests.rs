//! Tests for moving an undecodable `edge_store.bin` aside.

use super::quarantine_edge_snapshot_at;

#[test]
fn test_a_quarantine_moves_the_snapshot_to_its_timestamped_name() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("edge_store.bin");
    std::fs::write(&snapshot, b"undecodable").unwrap();

    let moved = quarantine_edge_snapshot_at(&snapshot, 42).unwrap();

    assert_eq!(moved, dir.path().join("edge_store.bin.unreadable-42"));
    assert_eq!(std::fs::read(&moved).unwrap(), b"undecodable");
    assert!(!snapshot.exists());
}

/// A name already taken, by an earlier quarantine in the same millisecond,
/// gets a counter: the earlier file keeps its bytes.
#[test]
fn test_a_quarantine_never_replaces_an_earlier_one() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("edge_store.bin");
    let earlier = dir.path().join("edge_store.bin.unreadable-42");
    std::fs::write(&earlier, b"earlier").unwrap();
    std::fs::write(dir.path().join("edge_store.bin.unreadable-42-1"), b"also").unwrap();
    std::fs::write(&snapshot, b"later").unwrap();

    let moved = quarantine_edge_snapshot_at(&snapshot, 42).unwrap();

    assert_eq!(moved, dir.path().join("edge_store.bin.unreadable-42-2"));
    assert_eq!(std::fs::read(&moved).unwrap(), b"later");
    assert_eq!(std::fs::read(&earlier).unwrap(), b"earlier");
}

/// A failed rename leaves the snapshot where it was and no empty reservation.
#[test]
fn test_a_failed_quarantine_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("edge_store.bin");

    assert!(quarantine_edge_snapshot_at(&missing, 42).is_err());

    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// A directory sync that fails after the rename keeps the moved snapshot:
/// its bytes are what the quarantine exists for.
#[cfg(unix)]
#[test]
fn test_a_failed_directory_sync_keeps_the_quarantined_bytes() {
    use crate::storage::atomic_write::{AtomicWriteBoundary, FaultGuard};

    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("edge_store.bin");
    std::fs::write(&snapshot, b"undecodable").unwrap();

    let moved = {
        let _fault = FaultGuard::inject(AtomicWriteBoundary::ParentDirectorySync);
        quarantine_edge_snapshot_at(&snapshot, 42).unwrap()
    };

    assert_eq!(std::fs::read(&moved).unwrap(), b"undecodable");
    assert!(!snapshot.exists());
}
