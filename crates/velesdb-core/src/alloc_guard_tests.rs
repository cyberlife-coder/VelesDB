//! Tests for `alloc_guard` module

use super::alloc_guard::*;
use serial_test::serial;
use std::alloc::{dealloc, Layout};

#[test]
fn test_alloc_guard_basic() {
    let layout = Layout::from_size_align(1024, 8).unwrap();
    let guard = AllocGuard::new(layout).expect("allocation failed");

    assert!(!guard.as_ptr().is_null());
    assert_eq!(guard.layout().size(), 1024);
    assert_eq!(guard.layout().align(), 8);
}

#[test]
fn test_alloc_guard_into_raw() {
    let layout = Layout::from_size_align(64, 8).unwrap();
    let guard = AllocGuard::new(layout).expect("allocation failed");
    let ptr = guard.into_raw();

    // Must manually deallocate
    assert!(!ptr.is_null());
    // SAFETY: `dealloc` requires a pointer from `alloc` with the same layout.
    // - Condition 1: `ptr` was obtained from `into_raw()`, which transfers ownership
    //   of a valid allocation created by `AllocGuard::new(layout)`.
    // - Condition 2: `layout` is the same layout used for the original allocation.
    // Reason: `into_raw()` disables the RAII guard; caller must deallocate manually.
    unsafe {
        dealloc(ptr, layout);
    }
}

#[test]
fn test_alloc_guard_zero_size() {
    let layout = Layout::from_size_align(0, 1).unwrap();
    assert!(AllocGuard::new(layout).is_none());
}

#[test]
fn test_alloc_guard_aligned() {
    // Cache-line aligned (64 bytes)
    let layout = Layout::from_size_align(256, 64).unwrap();
    let guard = AllocGuard::new(layout).expect("allocation failed");

    let addr = guard.as_ptr() as usize;
    assert_eq!(addr % 64, 0, "Not cache-line aligned");
}

#[test]
fn test_alloc_guard_cast() {
    let layout =
        Layout::from_size_align(std::mem::size_of::<f32>() * 10, std::mem::align_of::<f32>())
            .unwrap();

    let guard = AllocGuard::new(layout).expect("allocation failed");
    let float_ptr: *mut f32 = guard.cast();

    // Write some data
    // SAFETY: `float_ptr.add(i)` requires a valid, aligned pointer within the allocation.
    // - Condition 1: `guard` allocated `size_of::<f32>() * 10` bytes with `align_of::<f32>()`.
    // - Condition 2: `i` ranges 0..10, so `add(i)` stays within the allocation bounds.
    // Reason: Verifying that `AllocGuard::cast` produces a usable typed pointer.
    #[allow(clippy::cast_precision_loss)]
    unsafe {
        for i in 0..10 {
            *float_ptr.add(i) = i as f32;
        }
    }

    // Read back
    // SAFETY: Same invariants as the write block above.
    // - Condition 1: Data was written in the preceding block; no reallocation occurred.
    // - Condition 2: `guard` is still alive, so the allocation is valid.
    // Reason: Round-trip verification of typed pointer read/write.
    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    unsafe {
        for i in 0..10 {
            assert_eq!(*float_ptr.add(i), i as f32);
        }
    }
}

#[test]
fn test_alloc_guard_drop_frees_memory() {
    // This test verifies the guard deallocates on drop across repeated cycles.
    // Each allocation is asserted to succeed; the guard is then dropped, freeing memory.
    for _ in 0..1000 {
        let layout = Layout::from_size_align(1024, 8).unwrap();
        let guard = AllocGuard::new(layout);
        assert!(
            guard.is_some(),
            "1 KiB allocation must succeed under default ceiling"
        );
        // guard dropped here, memory freed
    }
}

#[test]
fn test_alloc_guard_panic_safety() {
    use std::panic;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Set only after AllocGuard::new produced a real, non-null allocation, so the
    // assertion fails if `new` is stubbed to None (the `expect` would unwind first)
    // or hands back a null pointer.
    static GUARD_BUILT: AtomicBool = AtomicBool::new(false);

    let layout = Layout::from_size_align(1024, 8).unwrap();
    GUARD_BUILT.store(false, Ordering::SeqCst);

    // Simulate panic during operation, with a live AllocGuard on the stack so its
    // RAII Drop runs during unwinding.
    let result = panic::catch_unwind(|| {
        let guard = AllocGuard::new(layout).expect("allocation failed");
        assert!(!guard.as_ptr().is_null());
        GUARD_BUILT.store(true, Ordering::SeqCst);
        panic!("simulated panic");
        // `guard` is dropped here during unwind, freeing the allocation.
    });

    assert!(result.is_err());
    assert!(
        GUARD_BUILT.load(Ordering::SeqCst),
        "AllocGuard::new must produce a valid allocation before the panic, so its \
         Drop runs during unwind"
    );
}

// =========================================================================
// #899 — Allocation-bound regression tests
//
// Tests here may only ever *raise* the process-global `ALLOC_BYTE_LIMIT` back
// to its default (`set_alloc_byte_limit(0)`); they must NEVER pin it to a low
// value.
//
// `#[serial]` is not sufficient protection: it only excludes other `#[serial]`
// tests, while the thousands of unannotated tests in this binary keep running
// in parallel against whatever ceiling is installed. A low global ceiling
// therefore fails unrelated tests at random with an `AllocationFailed` naming a
// limit they never configured (observed 2026-07-25). `#[serial]` is kept below
// only so these tests do not race each other's *reads* of the global.
//
// - Need a low ceiling for one operation? Use `with_alloc_byte_limit`
//   (thread-local, scoped, invisible to other threads).
// - Need to verify the process-global setter itself? It lives in its own test
//   binary: `tests/alloc_guard_global_limit.rs`.
// =========================================================================

/// The default ceiling is the high 1 TiB backstop — not a 16 GiB workload cap.
#[test]
#[serial]
fn test_default_ceiling_is_high_backstop() {
    let saved = alloc_byte_limit();
    set_alloc_byte_limit(0); // normalize to the default
    assert_eq!(alloc_byte_limit(), DEFAULT_ALLOC_BYTE_LIMIT);
    assert_eq!(DEFAULT_ALLOC_BYTE_LIMIT, 1024 * 1024 * 1024 * 1024);
    set_alloc_byte_limit(saved);
}

/// A request above the configured byte ceiling returns `None` (no allocation),
/// while a normal-sized request still succeeds.
#[test]
#[serial]
fn test_alloc_guard_rejects_above_ceiling() {
    let saved = alloc_byte_limit();
    set_alloc_byte_limit(0);
    let limit = alloc_byte_limit();
    assert_eq!(limit, DEFAULT_ALLOC_BYTE_LIMIT);

    // Just above the ceiling: rejected without touching the allocator
    // (constructing the Layout never allocates).
    let oversized = Layout::from_size_align(limit + 1, 8).unwrap();
    assert!(AllocGuard::new(oversized).is_none());
    assert!(AllocGuard::new_zeroed(oversized).is_none());

    // A normal, sane allocation still succeeds.
    let ok = Layout::from_size_align(4096, 64).unwrap();
    assert!(AllocGuard::new(ok).is_some());
    assert!(AllocGuard::new_zeroed(ok).is_some());
    set_alloc_byte_limit(saved);
}

/// `check_alloc_bound` errors above the limit and is OK at/below it.
#[test]
#[serial]
fn test_check_alloc_bound() {
    let saved = alloc_byte_limit();
    set_alloc_byte_limit(0);
    let limit = alloc_byte_limit();
    assert!(check_alloc_bound(limit).is_ok());
    assert!(check_alloc_bound(0).is_ok());
    assert!(check_alloc_bound(limit + 1).is_err());
    set_alloc_byte_limit(saved);
}

// `test_set_alloc_byte_limit_roundtrip` moved to
// `tests/alloc_guard_global_limit.rs`: it must pin the process-global ceiling to
// 8192, which is unsafe in this binary (see the module note above).

/// REGRESSION (#899 follow-up): a large-but-legitimate single-buffer size that
/// the old 16 GiB cap would have falsely rejected is now accepted by the
/// bound-decision function. We test the *decision*, never a real 20 GiB alloc.
#[test]
#[serial]
fn test_large_legit_buffer_not_falsely_rejected() {
    const GIB: usize = 1024 * 1024 * 1024;
    let saved = alloc_byte_limit();
    set_alloc_byte_limit(0); // default 1 TiB backstop

    // ~2.8M vectors @768D ≈ 8.2 GiB; ~5.6M @768D ≈ 16.5 GiB — both tripped the
    // old 16 GiB cap. Probe sizes well above 16 GiB but below 1 TiB: all OK now.
    for gib in [20usize, 64, 128, 512] {
        let bytes = gib * GIB;
        assert!(
            check_alloc_bound(bytes).is_ok(),
            "{gib} GiB single buffer must not be falsely rejected"
        );
    }
    set_alloc_byte_limit(saved);
}

/// REGRESSION (#899 follow-up): the persisted-index LOAD bound is derived from
/// the file-backed payload, so a realistic large `count` (above the old cap)
/// reloads. `with_min_alloc_byte_limit` raises the ceiling to the file-backed
/// size for the load scope, then restores it.
///
/// The low starting ceiling is pinned with `with_alloc_byte_limit` (thread-local)
/// rather than `set_alloc_byte_limit` (process-global): pinning 4096 globally
/// made every *other* test in this binary fail at random with an unrelated
/// `AllocationFailed` while the window was open. `#[serial]` does not prevent
/// that — it only excludes other `#[serial]` tests, and the thousands of
/// unannotated tests keep running in parallel.
#[test]
fn test_load_path_bound_allows_realistic_large_count() {
    // Pin a deliberately low limit to prove the load path raises past it.
    with_alloc_byte_limit(4096, || {
        // ~30 GiB file-backed payload (8M vectors @768D *4 ≈ 24 GiB) — a legit
        // persisted index. The load path must accept its own file-backed size.
        let file_backed_bytes = 30usize * 1024 * 1024 * 1024;
        let inner = with_min_alloc_byte_limit(file_backed_bytes, || {
            // Inside the scope the ceiling covers the file-backed size.
            assert!(check_alloc_bound(file_backed_bytes).is_ok());
            alloc_byte_limit()
        });
        assert_eq!(inner, file_backed_bytes, "ceiling raised within load scope");

        // Restored after the scope (no leak of the raised limit).
        assert_eq!(alloc_byte_limit(), 4096);
    });
}

/// `with_min_alloc_byte_limit` is a transparent pass-through when the current
/// ceiling already covers the requested minimum (no mutation).
#[test]
#[serial]
fn test_with_min_alloc_byte_limit_passthrough() {
    let saved = alloc_byte_limit();
    set_alloc_byte_limit(0); // 1 TiB default
    let before = alloc_byte_limit();
    let observed = with_min_alloc_byte_limit(1024, alloc_byte_limit);
    assert_eq!(
        observed, before,
        "no raise needed; ceiling unchanged in scope"
    );
    assert_eq!(alloc_byte_limit(), before);
    set_alloc_byte_limit(saved);
}

// =========================================================================
// Scoped-ceiling isolation — regression tests
//
// A scoped ceiling adjustment used to be written to the process-global
// `ALLOC_BYTE_LIMIT`. Two independent defects followed, both reproduced
// deterministically below with barriers rather than left to scheduling luck.
// =========================================================================

/// REGRESSION: a scoped raise must not lift the backstop for allocations
/// happening concurrently on unrelated threads.
///
/// While a global raise was in flight, every other thread was judged against the
/// raised ceiling — precisely the pathological sizes the #899 backstop exists to
/// reject were admitted for the duration of any index load.
#[test]
#[cfg(target_pointer_width = "64")]
fn test_scoped_raise_is_not_visible_to_other_threads() {
    const TWO_TIB: usize = 2 * 1024 * 1024 * 1024 * 1024;

    let expected = alloc_byte_limit();
    let observed = with_min_alloc_byte_limit(TWO_TIB, || {
        // A raise on THIS thread is in force here...
        assert_eq!(alloc_byte_limit(), TWO_TIB, "raise applies to this thread");
        // ...but a concurrent thread must still see the unraised ceiling.
        std::thread::spawn(|| (alloc_byte_limit(), check_alloc_bound(TWO_TIB).is_err()))
            .join()
            .expect("observer thread panicked")
    });

    assert_eq!(
        observed.0, expected,
        "scoped raise leaked to another thread: the allocation backstop was \
         silently lifted process-wide for the duration of the scope"
    );
    assert!(
        observed.1,
        "an oversized allocation must still be rejected on threads outside the scope"
    );
}

/// REGRESSION: overlapping scoped adjustments must not clobber each other.
///
/// With a single global cell, each scope saves the ceiling it happens to observe
/// on entry and restores it on exit. When two scopes overlap, the inner one saves
/// the outer one's temporary value and republishes it after the outer has already
/// restored — a lost update that leaves the process-wide backstop permanently
/// wrong (here: pinned at 2 TiB forever, long after both loads finished).
#[test]
#[cfg(target_pointer_width = "64")]
fn test_overlapping_scoped_raises_do_not_clobber() {
    use std::sync::{Arc, Barrier};

    const TWO_TIB: usize = 2 * 1024 * 1024 * 1024 * 1024;
    const THREE_TIB: usize = 3 * 1024 * 1024 * 1024 * 1024;

    let before = alloc_byte_limit();

    // Force the exact interleaving: A enters, B enters, A exits, B exits.
    let (entered_a, entered_b, exited_a) = (
        Arc::new(Barrier::new(2)),
        Arc::new(Barrier::new(2)),
        Arc::new(Barrier::new(2)),
    );

    let (a1, b1, x1) = (
        Arc::clone(&entered_a),
        Arc::clone(&entered_b),
        Arc::clone(&exited_a),
    );
    let thread_a = std::thread::spawn(move || {
        with_min_alloc_byte_limit(TWO_TIB, || {
            a1.wait(); // A is inside its scope
            b1.wait(); // wait until B is inside its scope too
        });
        x1.wait(); // A has left its scope
    });

    let thread_b = std::thread::spawn(move || {
        entered_a.wait(); // enter only once A is inside
        with_min_alloc_byte_limit(THREE_TIB, || {
            entered_b.wait(); // B is inside its scope
            exited_a.wait(); // hold the scope open until A has left
        });
    });

    thread_a.join().expect("thread A panicked");
    thread_b.join().expect("thread B panicked");

    assert_eq!(
        alloc_byte_limit(),
        before,
        "overlapping scopes corrupted the ceiling: it must be exactly as it was \
         before both scopes ran, not a value republished by a lost update"
    );
}

/// REGRESSION (2026-07-25 flake): pinning a *low* ceiling for one operation must
/// not make a legitimate allocation fail on another thread.
///
/// A low ceiling pinned globally by the `alloc_guard` tests made unrelated tests
/// in the same binary fail at random with an `AllocationFailed` naming a limit
/// they never configured (observed: a 1.6 MB agent-memory buffer rejected
/// against a 4096-byte ceiling).
#[test]
fn test_low_scoped_ceiling_does_not_break_other_threads() {
    // The exact allocation from the reported failure: a 4-dim collection with
    // 100_000 capacity = 1_600_000 bytes.
    const LEGITIMATE_BYTES: usize = 4 * 100_000 * std::mem::size_of::<f32>();

    let admitted = with_alloc_byte_limit(4096, || {
        assert!(
            check_alloc_bound(LEGITIMATE_BYTES).is_err(),
            "the pinned ceiling must still be enforced on the pinning thread"
        );
        std::thread::spawn(|| check_alloc_bound(LEGITIMATE_BYTES).is_ok())
            .join()
            .expect("observer thread panicked")
    });

    assert!(
        admitted,
        "a ceiling pinned low for one operation must not reject legitimate \
         allocations on unrelated threads"
    );
}

/// A scoped ceiling nests: the inner scope restores the outer scope's value, not
/// the process-global one.
#[test]
fn test_scoped_alloc_byte_limit_nests() {
    with_alloc_byte_limit(8192, || {
        assert_eq!(alloc_byte_limit(), 8192);
        with_alloc_byte_limit(4096, || {
            assert_eq!(alloc_byte_limit(), 4096);
        });
        assert_eq!(
            alloc_byte_limit(),
            8192,
            "inner scope restored the outer one"
        );
    });
}

/// `with_min_alloc_byte_limit` restores the previous ceiling even if the closure
/// panics (RAII restore), so a panicking load cannot leak a raised limit.
#[test]
fn test_with_min_alloc_byte_limit_restores_on_panic() {
    use std::panic;

    // Thread-local pin (see `test_load_path_bound_allows_realistic_large_count`
    // for why this must not be a global `set_alloc_byte_limit`).
    with_alloc_byte_limit(4096, || {
        let huge = 30usize * 1024 * 1024 * 1024;
        let result = panic::catch_unwind(|| {
            with_min_alloc_byte_limit(huge, || {
                panic!("simulated load failure");
            });
        });
        assert!(result.is_err());
        assert_eq!(alloc_byte_limit(), 4096, "ceiling restored after panic");
    });
}
