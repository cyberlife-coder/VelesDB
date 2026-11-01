//! A test's hook into a flush: fails the vector storage's flush step on the
//! thread that set it, as a full disk or an I/O error would.

use std::cell::Cell;

thread_local! {
    static FAIL_VECTOR_FLUSH: Cell<bool> = const { Cell::new(false) };
}

/// Fails every vector storage flush a flush on this thread runs from now on.
pub(crate) fn fail_vector_flush() {
    FAIL_VECTOR_FLUSH.with(|fail| fail.set(true));
}

/// `flushed`, or an injected failure when the test asked for one.
pub(crate) fn vector_flush(flushed: std::io::Result<()>) -> std::io::Result<()> {
    if FAIL_VECTOR_FLUSH.with(Cell::get) {
        return Err(std::io::Error::other("test: vector storage flush failed"));
    }
    flushed
}
