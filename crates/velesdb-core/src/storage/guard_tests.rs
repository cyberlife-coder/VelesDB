//! Tests for [`VectorSliceGuard`] epoch validation, `as_slice()`,
//! `Deref`, and `AsRef` behavior on epoch mismatch.

use super::guard::VectorSliceGuard;
use memmap2::MmapMut;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Helper: builds a `VectorSliceGuard` pointing at `data` with the given
/// creation epoch. The `epoch_ptr` is shared so tests can bump it.
fn make_guard<'a>(
    lock: &'a RwLock<MmapMut>,
    data: &[f32],
    epoch_ptr: &'a AtomicU64,
    creation_epoch: u64,
) -> VectorSliceGuard<'a> {
    VectorSliceGuard {
        _guard: lock.read(),
        ptr: data.as_ptr(),
        len: data.len(),
        epoch_ptr,
        epoch_at_creation: creation_epoch,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// as_slice() returns Ok on valid guard
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn guard_as_slice_valid_epoch() {
    let data: Vec<f32> = vec![1.0, 2.0, 3.0];
    let mmap = MmapMut::map_anon(4096).expect("anon mmap");
    let lock = RwLock::new(mmap);
    let epoch = AtomicU64::new(1);

    let guard = make_guard(&lock, &data, &epoch, 1);
    let slice = guard.as_slice().expect("should succeed");
    assert_eq!(slice, &[1.0, 2.0, 3.0]);
}

#[test]
fn guard_try_deref_same_as_slice() {
    let data: Vec<f32> = vec![4.0, 5.0];
    let mmap = MmapMut::map_anon(4096).expect("anon mmap");
    let lock = RwLock::new(mmap);
    let epoch = AtomicU64::new(0);

    let guard = make_guard(&lock, &data, &epoch, 0);
    let via_slice = guard.as_slice().expect("ok");
    let via_try = guard.try_deref().expect("ok");
    assert_eq!(
        via_try,
        &[4.0, 5.0f32],
        "try_deref must return the actual vector data"
    );
    assert_eq!(via_slice, via_try, "try_deref must alias as_slice");
}

// ─────────────────────────────────────────────────────────────────────────────
// as_slice() returns Err on epoch mismatch
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn guard_as_slice_epoch_mismatch() {
    let data: Vec<f32> = vec![1.0, 2.0];
    let mmap = MmapMut::map_anon(4096).expect("anon mmap");
    let lock = RwLock::new(mmap);
    let epoch = AtomicU64::new(1);

    // Guard was created at epoch 1, but global epoch advances to 2
    let guard = make_guard(&lock, &data, &epoch, 1);
    epoch.store(2, Ordering::Release);

    let result = guard.as_slice();
    assert!(result.is_err(), "stale epoch should return Err");
}

// ─────────────────────────────────────────────────────────────────────────────
// Deref / AsRef return empty slice on mismatch
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn guard_deref_returns_empty_on_mismatch() {
    let data: Vec<f32> = vec![9.0, 8.0, 7.0];
    let mmap = MmapMut::map_anon(4096).expect("anon mmap");
    let lock = RwLock::new(mmap);
    let epoch = AtomicU64::new(5);

    let guard = make_guard(&lock, &data, &epoch, 5);
    epoch.store(6, Ordering::Release);

    let slice: &[f32] = &guard;
    assert!(slice.is_empty(), "Deref must return empty on mismatch");
}

#[test]
fn guard_as_ref_returns_empty_on_mismatch() {
    let data: Vec<f32> = vec![1.0];
    let mmap = MmapMut::map_anon(4096).expect("anon mmap");
    let lock = RwLock::new(mmap);
    let epoch = AtomicU64::new(10);

    let guard = make_guard(&lock, &data, &epoch, 10);
    epoch.store(11, Ordering::Release);

    let slice: &[f32] = guard.as_ref();
    assert!(slice.is_empty(), "AsRef must return empty on mismatch");
}

#[test]
fn guard_deref_valid_epoch_returns_data() {
    let data: Vec<f32> = vec![3.125, 2.71];
    let mmap = MmapMut::map_anon(4096).expect("anon mmap");
    let lock = RwLock::new(mmap);
    let epoch = AtomicU64::new(42);

    let guard = make_guard(&lock, &data, &epoch, 42);
    let slice: &[f32] = &guard;
    assert_eq!(slice, &[3.125, 2.71]);
}
