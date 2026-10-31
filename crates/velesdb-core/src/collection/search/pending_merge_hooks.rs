//! A test's hooks into a merge (`Collection::index_current`): one run once a
//! chunk's vectors are read and before they are inserted, where a delete or
//! an upsert racing the merge would land if the merge let go of storage in
//! between, and one that fails the read of chosen ids. Both are
//! thread-local: a merge runs on the thread that asked for it.

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

thread_local! {
    static UNREADABLE: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Fails every read of `ids` a merge on this thread makes from now on.
pub(crate) fn set_unreadable(ids: &[u64]) {
    UNREADABLE.with(|slot| slot.borrow_mut().extend_from_slice(ids));
}

pub(crate) fn read(
    id: u64,
    read: std::io::Result<Option<Vec<f32>>>,
) -> std::io::Result<Option<Vec<f32>>> {
    if UNREADABLE.with(|slot| slot.borrow().contains(&id)) {
        return Err(std::io::Error::other(format!("test: id {id} unreadable")));
    }
    read
}
