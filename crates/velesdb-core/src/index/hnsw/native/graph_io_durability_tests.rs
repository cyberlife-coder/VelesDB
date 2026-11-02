//! Crash-simulation tests for the `.vectors` / `.graph` dump paths
//! (audit #2590).
//!
//! `file_dump` used to write both files in place through `BufWriter::flush`
//! only — no fsync, no rename. A power loss mid-dump could leave a
//! right-sized file whose unwritten tail reads as zeros, which `load`'s
//! structural validation cannot distinguish from real data. Both files now
//! go through `atomic_write`: a crashed dump leaves the previous complete
//! file, and a committed one is durable. The `atomic_write` fault seam
//! proves each barrier is actually wired into the dump path — a check that
//! cannot fire proves nothing.

use super::super::distance::CpuDistance;
use super::NativeHnsw;
use crate::distance::DistanceMetric;
use crate::storage::atomic_write::{AtomicWriteBoundary, FaultGuard};
use tempfile::tempdir;

type H = NativeHnsw<CpuDistance>;

fn build_index(n: usize) -> H {
    let hnsw = NativeHnsw::new(CpuDistance::new(DistanceMetric::Euclidean), 16, 100, 1000);
    for i in 0..n {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test: insert");
    }
    hnsw
}

/// The temp-file fsync barrier must be wired into `file_dump`.
///
/// Injecting a failure at the first `TemporaryFileSync` (the `.vectors`
/// staging file) must make the dump fail: if `file_dump` still bypassed
/// `atomic_write`, nothing would trip the boundary and this would pass
/// vacuously.
#[test]
fn vectors_dump_reaches_the_atomic_write_barriers() {
    let dir = tempdir().expect("test: tempdir");
    let hnsw = build_index(4);

    let guard = FaultGuard::inject(AtomicWriteBoundary::TemporaryFileSync);
    let result = hnsw.file_dump(dir.path(), "native_hnsw");
    drop(guard);

    assert!(
        result.is_err(),
        "a fault at the .vectors temp-file fsync must fail the dump"
    );
}

/// The `.graph` file must go through the same barrier: skipping the first
/// `TemporaryFileSync` (the `.vectors` staging file) and failing the second
/// must still fail the dump.
#[test]
fn graph_dump_reaches_the_atomic_write_barriers() {
    let dir = tempdir().expect("test: tempdir");
    let hnsw = build_index(4);

    let guard = FaultGuard::inject_after(AtomicWriteBoundary::TemporaryFileSync, 1);
    let result = hnsw.file_dump(dir.path(), "native_hnsw");
    drop(guard);

    assert!(
        result.is_err(),
        "a fault at the .graph temp-file fsync must fail the dump"
    );
}

/// Crash-safety: a dump that dies before its renames must leave the
/// *previous* complete snapshot readable.
///
/// A dump is persisted, the index grows, and the second dump is killed at
/// the rename boundary. Under the old in-place `File::create` rewrite the
/// live `.vectors`/`.graph` was truncated first, so this crash destroyed
/// the only copy. With `atomic_write` the failure lands before the
/// renames, so the gen-1 files must still load — and yield the gen-1
/// vector count.
#[test]
fn crashed_dump_leaves_previous_complete_snapshot() {
    let dir = tempdir().expect("test: tempdir");
    let hnsw = build_index(4);
    hnsw
        .file_dump(dir.path(), "native_hnsw")
        .expect("test: gen-1 dump");

    // Grow the index so a successful second dump would differ visibly.
    hnsw
        .insert(&[42.0f32; 32])
        .expect("test: grow");

    let guard = FaultGuard::inject(AtomicWriteBoundary::Replacement);
    let second = hnsw.file_dump(dir.path(), "native_hnsw");
    drop(guard);
    assert!(second.is_err(), "the crashed dump must report failure");

    // The previous snapshot must survive whole — the property the audit
    // found missing. `file_load` returning Ok(()) is the assertion: the
    // old write would have left a truncated `.vectors`.
    let loaded = H::file_load(
        dir.path(),
        "native_hnsw",
        CpuDistance::new(DistanceMetric::Euclidean),
    )
    .expect("test: previous snapshot must survive a crashed dump");
    assert_eq!(loaded.len(), 4, "the surviving snapshot is the gen-1 one");
}
