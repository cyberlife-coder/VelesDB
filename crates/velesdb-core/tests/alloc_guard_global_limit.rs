//! Process-global allocation-ceiling tests (#899).
//!
//! # Why these live in their own test binary
//!
//! [`set_alloc_byte_limit`] configures a **process-global** ceiling: that is its
//! contract, so a test that pins it to a small value necessarily lowers it for
//! every thread in the process.
//!
//! Inside `--lib`, that is unsafe at any level of annotation. `#[serial]`
//! (`serial_test`) only excludes other `#[serial]` tests; the thousands of
//! unannotated tests in the same binary keep running in parallel and are judged
//! against whatever ceiling happens to be installed. On 2026-07-25 this
//! surfaced as a reproducible flake: with a 4096-byte ceiling pinned by
//! `alloc_guard`, an unrelated agent-memory test failed on a legitimate
//! 1_600_000-byte buffer with an `AllocationFailed` naming a limit it never
//! configured. The test that failed varied run to run; only the full suite
//! showed it, and each test passed in isolation.
//!
//! Process-global state needs *process* isolation. Cargo runs every integration
//! test target as its own process, so these tests get a private global here and
//! cannot perturb — or be perturbed by — the main suite. The scoped, thread-local
//! counterpart (`with_alloc_byte_limit`) is exercised in the `--lib` tests, where
//! it is safe by construction.

use serial_test::serial;
use std::alloc::Layout;
use velesdb_core::alloc_guard::{
    alloc_byte_limit, set_alloc_byte_limit, AllocGuard, DEFAULT_ALLOC_BYTE_LIMIT,
};

/// The ceiling is configurable process-wide; `0` restores the default.
#[test]
#[serial]
fn test_set_alloc_byte_limit_roundtrip() {
    let original = alloc_byte_limit();

    set_alloc_byte_limit(8192);
    assert_eq!(alloc_byte_limit(), 8192);
    assert!(AllocGuard::new(Layout::from_size_align(16384, 8).unwrap()).is_none());
    assert!(AllocGuard::new(Layout::from_size_align(4096, 8).unwrap()).is_some());

    // `0` means "no override" → back to default.
    set_alloc_byte_limit(0);
    assert_eq!(alloc_byte_limit(), DEFAULT_ALLOC_BYTE_LIMIT);

    // Restore whatever the harness started with.
    set_alloc_byte_limit(original);
}

/// The global ceiling applies to **every** thread — that is the whole point of
/// `set_alloc_byte_limit`, and what distinguishes it from the thread-local
/// `with_alloc_byte_limit` scope.
#[test]
#[serial]
fn test_global_limit_applies_to_worker_threads() {
    let original = alloc_byte_limit();

    set_alloc_byte_limit(8192);
    let rejected_on_worker = std::thread::spawn(|| {
        AllocGuard::new(Layout::from_size_align(16384, 8).unwrap()).is_none()
    })
    .join()
    .expect("worker thread panicked");

    set_alloc_byte_limit(original);

    assert!(
        rejected_on_worker,
        "an operator-configured ceiling must bound allocations on worker threads \
         (rayon index builds, tokio blocking pool), not just the caller's thread"
    );
}
