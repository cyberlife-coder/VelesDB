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
