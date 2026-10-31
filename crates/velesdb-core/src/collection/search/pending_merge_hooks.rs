//! A test's hooks into a merge (`Collection::index_current`): one run once a
//! chunk's vectors are read and before they are inserted, where a delete or
//! an upsert racing the merge would land if the merge let go of storage in
//! between; one that fails the read of chosen ids; one that has the graph
//! refuse a batch holding a chosen id. All are thread-local: a merge runs on
//! the thread that asked for it.

use crate::index::hnsw::index::UnlessIndexed;
use std::cell::{Cell, RefCell};

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
    static REFUSED_IN_A_BATCH: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static GUARD_HOLDS: Cell<usize> = const { Cell::new(0) };
}

/// Fails every read of `ids` a merge on this thread makes from now on.
pub(crate) fn set_unreadable(ids: &[u64]) {
    UNREADABLE.with(|slot| slot.borrow_mut().extend_from_slice(ids));
}

/// Has the graph refuse, from now on, every batch of more than one pair
/// that a merge on this thread inserts and that holds one of `ids`.
pub(crate) fn set_refused_in_a_batch(ids: &[u64]) {
    REFUSED_IN_A_BATCH.with(|slot| slot.borrow_mut().extend_from_slice(ids));
}

/// The number of chunks — each read and inserted under one storage guard —
/// merges on this thread have inserted.
pub(crate) fn guard_holds() -> usize {
    GUARD_HOLDS.with(Cell::get)
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

/// The graph's answer to a batch the test has it refuse, else `None`.
pub(crate) fn insert(present: &[(u64, Vec<f32>)]) -> Option<UnlessIndexed> {
    GUARD_HOLDS.with(|holds| holds.set(holds.get() + 1));
    let refused = REFUSED_IN_A_BATCH.with(|slot| {
        let refused = slot.borrow();
        present.len() > 1 && present.iter().any(|(id, _)| refused.contains(id))
    });
    refused.then(|| UnlessIndexed {
        settled: Vec::new(),
        refused: present.iter().map(|&(id, _)| id).collect(),
        error: Some(crate::error::Error::Internal("test: batch refused".into())),
    })
}
