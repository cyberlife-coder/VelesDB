//! A test's hook into a merge (`Collection::index_current`), run once a
//! chunk's vectors are read and before they are inserted: where a delete or
//! an upsert racing the merge would land if the merge let go of storage in
//! between.

use std::cell::RefCell;

type Hook = Box<dyn FnOnce()>;

thread_local! {
    static AFTER_READ: RefCell<Option<Hook>> = RefCell::new(None);
}

/// Runs `hook` the next time a merge on this thread has read its vectors.
pub(crate) fn set_after_read(hook: impl FnOnce() + 'static) {
    AFTER_READ.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

pub(crate) fn after_read() {
    if let Some(hook) = AFTER_READ.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

/// Storage reads a search made for its pending points on a rayon worker.
///
/// A batch search holds the `vector_storage` read guard across a join on the
/// global rayon pool while it resolves results. A global worker that takes
/// that guard again parks behind any queued writer, and the batch holding
/// the guard then waits for a worker that never frees: the whole process
/// stops (#2343's shape, one lock lower). No search may read storage from a
/// rayon worker; tests read this count to hold it.
pub(crate) static STORAGE_READS_ON_RAYON: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Counts a pending-point storage read made on a rayon worker.
pub(crate) fn note_storage_read() {
    if rayon::current_thread_index().is_some() {
        STORAGE_READS_ON_RAYON.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}
