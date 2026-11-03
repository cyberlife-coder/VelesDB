//! Durability tests for `crud_histogram`'s stats write path (audit #2590).

use super::Collection;
use crate::DistanceMetric;
use crate::storage::atomic_write::{AtomicWriteBoundary, FaultGuard};

/// `analyze_collection` advertises that it "persists them to disk":
/// `write_stats_guarded` must reach the `atomic_write` barriers — a
/// fault at the parent-directory fsync must fail the write (a barrier
/// not wired would pass vacuously), and a clean write must land.
/// Audit #2590: the earlier `std::fs::write` + bare rename did neither
/// fsync.
#[test]
fn write_stats_guarded_reaches_the_dir_fsync() {
    let dir = tempfile::tempdir().expect("tempdir");
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine)
        .expect("collection");
    let stats = coll.analyze().expect("analyze");

    let guard = FaultGuard::inject(AtomicWriteBoundary::ParentDirectorySync);
    let result = coll.write_stats_guarded(&stats);
    drop(guard);
    assert!(result.is_err(), "the dir-fsync barrier must be in-path");

    coll.write_stats_guarded(&stats).expect("clean write");
    assert!(dir.path().join("collection.stats.json").exists());
}
