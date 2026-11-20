//! A test's hooks into a write: each runs a closure on the thread that set
//! it, once, at a point a concurrent operation must not slip into.
//!
//! - [`pause_before_text_wal`]: a BM25 change or a delete about to take
//!   `text_wal` shared — what it must not hold while it waits there.
//! - [`pause_after_text_wal`]: a BM25 change between its WAL append and its
//!   in-memory apply — the window a BM25 snapshot must not fall into
//!   (`Collection::change_text_index`).
//! - [`pause_after_node_membership`]: a write holding the node locks of its
//!   ids, before it writes — a graph write that may change its nodes, once
//!   it read the other store's membership of its ids, or a vector
//!   collection's payload write to its rows, before it reads which of them
//!   store a vector — the window no write changing that membership may
//!   enter (`collection/core/size.rs`).
//! - [`pause_between_row_stores`]: a row write — an upsert, bulk or not —
//!   once its payloads are stored and before its vectors are: the window a
//!   delete of one of its ids must not fall into on a vector collection
//!   (`Collection::write_rows`).

use std::cell::RefCell;

type Pause = RefCell<Option<Box<dyn FnOnce()>>>;

thread_local! {
    static BEFORE_TEXT_WAL: Pause = const { RefCell::new(None) };
    static AFTER_TEXT_WAL: Pause = const { RefCell::new(None) };
    static AFTER_NODE_MEMBERSHIP: Pause = const { RefCell::new(None) };
    static BETWEEN_ROW_STORES: Pause = const { RefCell::new(None) };
}

fn arm(hook: &'static std::thread::LocalKey<Pause>, pause: impl FnOnce() + 'static) {
    hook.with(|hook| *hook.borrow_mut() = Some(Box::new(pause)));
}

fn fire(hook: &'static std::thread::LocalKey<Pause>) {
    let pause = hook.with(|hook| hook.borrow_mut().take());
    if let Some(pause) = pause {
        pause();
    }
}

/// Runs `pause` in the next BM25 change or delete this thread makes, just
/// before it takes `text_wal` shared.
pub(crate) fn pause_before_text_wal(pause: impl FnOnce() + 'static) {
    arm(&BEFORE_TEXT_WAL, pause);
}

/// Runs the closure [`pause_before_text_wal`] set on this thread, once.
pub(crate) fn before_text_wal() {
    fire(&BEFORE_TEXT_WAL);
}

/// Runs `pause` in the next BM25 change this thread makes, once its WAL
/// append returned and before its in-memory apply.
pub(crate) fn pause_after_text_wal(pause: impl FnOnce() + 'static) {
    arm(&AFTER_TEXT_WAL, pause);
}

/// Runs the closure [`pause_after_text_wal`] set on this thread, once.
pub(crate) fn after_text_wal() {
    fire(&AFTER_TEXT_WAL);
}

/// Runs `pause` in the next write on this thread that takes node locks,
/// once it holds them: a graph write once it read the other store's
/// membership of its ids, a vector collection's payload write to its rows
/// before it reads which of them store a vector.
pub(crate) fn pause_after_node_membership(pause: impl FnOnce() + 'static) {
    arm(&AFTER_NODE_MEMBERSHIP, pause);
}

/// Runs the closure [`pause_after_node_membership`] set on this thread, once.
pub(crate) fn after_node_membership() {
    fire(&AFTER_NODE_MEMBERSHIP);
}

/// Runs `pause` in the next row write on this thread, once it stored its
/// payloads and before it stores its vectors.
pub(crate) fn pause_between_row_stores(pause: impl FnOnce() + 'static) {
    arm(&BETWEEN_ROW_STORES, pause);
}

/// Runs the closure [`pause_between_row_stores`] set on this thread, once.
pub(crate) fn between_row_stores() {
    fire(&BETWEEN_ROW_STORES);
}
