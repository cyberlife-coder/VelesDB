//! Safety wrapper for self-referential HNSW index.
//!
//! This module encapsulates the unsafe lifetime management required by `hnsw_rs`
//! when loading from disk. The library requires the `Hnsw` struct to borrow from
//! the `HnswIo` loader, creating a self-referential requirement when we want to
//! own both in the same struct.
//!
//! # Safety Mechanism
//!
//! `HnswSafeWrapper` ensures safety by:
//! 1. Wrapping `HnswInner` in `ManuallyDrop` to prevent automatic dropping.
//! 2. Storing the owner `HnswIo` (if present) in the same struct.
//! 3. Implementing `Drop` to explicitly drop `HnswInner` *before* `HnswIo`.
//!
//! This creates a safe abstraction boundary: users of `HnswSafeWrapper` don't need
//! to worry about field ordering or drop order in their own structs.

use super::inner::HnswInner;
use hnsw_rs::hnswio::HnswIo;
use parking_lot::RwLock;
use std::mem::ManuallyDrop;

/// Safe wrapper for HNSW index that manages self-referential lifetimes.
pub struct HnswSafeWrapper {
    /// The HNSW graph wrapper.
    ///
    /// Wrapped in `ManuallyDrop` so we can explicitly control that it gets
    /// dropped BEFORE `_io_holder`.
    ///
    /// Wrapped in `RwLock` for concurrent access.
    inner: RwLock<ManuallyDrop<HnswInner>>,

    /// The IO holder that owns the memory-mapped data.
    ///
    /// If `Some`, `inner` contains references to data owned by this field.
    /// This MUST be dropped AFTER `inner`.
    _io_holder: Option<Box<HnswIo>>,
}

impl HnswSafeWrapper {
    /// Creates a new wrapper for an in-memory HNSW index.
    ///
    /// Since the index owns its data, no `io_holder` is needed.
    pub fn new(inner: HnswInner) -> Self {
        Self {
            inner: RwLock::new(ManuallyDrop::new(inner)),
            _io_holder: None,
        }
    }

    /// Creates a wrapper for a loaded HNSW index.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that `inner` was created from `io_holder`
    /// and that the "lifetime lie" (casting lifetime to 'static) is valid
    /// as long as `io_holder` is alive.
    ///
    /// This wrapper takes ownership of both and ensures `io_holder` outlives `inner`.
    pub unsafe fn new_loaded(inner: HnswInner, io_holder: Box<HnswIo>) -> Self {
        Self {
            inner: RwLock::new(ManuallyDrop::new(inner)),
            _io_holder: Some(io_holder),
        }
    }

    /// Acquires a read lock on the inner HNSW index.
    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, ManuallyDrop<HnswInner>> {
        self.inner.read()
    }

    /// Acquires a write lock on the inner HNSW index.
    pub fn write(&self) -> parking_lot::RwLockWriteGuard<'_, ManuallyDrop<HnswInner>> {
        self.inner.write()
    }
}

impl Drop for HnswSafeWrapper {
    fn drop(&mut self) {
        // SAFETY: We strictly enforce drop order here.
        // 1. Acquire write lock to ensure exclusive access (though in Drop we have &mut self)
        // 2. Explicitly drop the HnswInner first.
        //    This destroys the HNSW graph and releases any references to the mmap data.
        unsafe {
            ManuallyDrop::drop(self.inner.get_mut());
        }
        
        // 3. _io_holder is dropped automatically here.
        //    Since inner is already gone, it's safe to unmap the memory.
    }
}
