//! Test-only fault-injection seams.
//!
//! This module is gated behind the `test-fault-injection` cargo
//! feature and is intended exclusively for integration tests in
//! downstream crates (notably `velesdb-server`) that need to force
//! specific internal failures without touching the real file system.
//!
//! Never enable the `test-fault-injection` feature in production
//! builds. The hooks are implemented as process-wide atomic flags so
//! they are cheap when disabled (a single `AtomicBool::load` per
//! call) but they would otherwise leak failures across unrelated
//! code paths if compiled into a running server.
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::fault_injection::SaveConfigFaultGuard;
//!
//! // The guard forces every `Collection::save_config()` call on
//! // this process to fail with a PermissionDenied error for the
//! // lifetime of the guard.
//! {
//!     let _guard = SaveConfigFaultGuard::activate();
//!     // Exercise the rollback path of apply_advanced_config,
//!     // upsert_points, or any other caller of save_config().
//! }
//! // Guard dropped → normal operation resumes.
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};

/// Sentinel value meaning "no fault injection scheduled".
/// `save_config` never reaches this call count in any realistic
/// scenario so the comparison is effectively disabled at rest.
const SAVE_CONFIG_FAIL_DISABLED: usize = usize::MAX;

/// Process-wide counter of every `Collection::save_config()` call
/// since the most recent guard activation. Compared against
/// `SAVE_CONFIG_FAIL_AT` on every call — the first call whose
/// zero-based index reaches `SAVE_CONFIG_FAIL_AT` returns a
/// synthetic `Error::Io(PermissionDenied)` and subsequent calls
/// pass through untouched (the guard "fires once").
pub static SAVE_CONFIG_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Process-wide threshold at which `save_config` starts failing.
/// Set to `usize::MAX` (via `SAVE_CONFIG_FAIL_DISABLED`) at rest so
/// the check is effectively a no-op when no guard is active.
/// A guard activation stores a finite value here; dropping the
/// guard resets it back to the sentinel.
pub(crate) static SAVE_CONFIG_FAIL_AT: AtomicUsize = AtomicUsize::new(SAVE_CONFIG_FAIL_DISABLED);

/// RAII guard that schedules the Nth call to
/// `Collection::save_config()` on this process to return a
/// synthetic `Error::Io(PermissionDenied)` instead of touching the
/// file system. All preceding and following calls succeed normally.
///
/// The "fail after N" semantics are what makes this guard useful for
/// Phase-2 rollback tests: when a REST handler first creates a
/// collection (Phase 1) and then applies advanced config (Phase 2),
/// the test needs Phase 1's save_config() calls to succeed and only
/// Phase 2's to fail. Activate the guard with
/// `fail_at = <count of Phase 1 calls>` — Phase 1 then completes
/// normally, Phase 2 immediately hits the injected failure, and the
/// rollback logic can be exercised end-to-end.
///
/// Dropping the guard resets both the threshold and the counter, so
/// tests that construct a guard inside a scope can never leak state
/// into unrelated tests — even if they panic in between. This
/// matters because the state is process-wide (atomic) rather than
/// thread-local: without RAII semantics a flaky test could poison
/// the whole test binary.
///
/// Always bind the guard to a named variable (`let _guard = ...`)
/// rather than `let _ = ...` — the latter drops the guard
/// immediately and defeats the purpose.
pub struct SaveConfigFaultGuard;

impl SaveConfigFaultGuard {
    /// Activates the `save_config` fault injection so the call
    /// whose zero-based index matches `fail_at` returns an
    /// `Error::Io(PermissionDenied)`. Earlier calls succeed; later
    /// calls also succeed (the guard fires exactly once). Pass
    /// `fail_at = 0` to fail the very first call.
    #[must_use = "the guard must be bound to a variable or the fault resets immediately"]
    pub fn activate(fail_at: usize) -> Self {
        SAVE_CONFIG_CALL_COUNT.store(0, Ordering::SeqCst);
        SAVE_CONFIG_FAIL_AT.store(fail_at, Ordering::SeqCst);
        Self
    }

    /// Convenience: equivalent to `activate(0)`. Kept for symmetry
    /// with the simpler "fail every call" intent that some tests may
    /// prefer when they only want to exercise the first
    /// `save_config()` on a fresh collection.
    #[must_use = "the guard must be bound to a variable or the fault resets immediately"]
    pub fn activate_on_first_call() -> Self {
        Self::activate(0)
    }
}

impl Drop for SaveConfigFaultGuard {
    fn drop(&mut self) {
        SAVE_CONFIG_FAIL_AT.store(SAVE_CONFIG_FAIL_DISABLED, Ordering::SeqCst);
        SAVE_CONFIG_CALL_COUNT.store(0, Ordering::SeqCst);
    }
}

/// Called by `Collection::save_config()` at the top of the function
/// to decide whether to return a synthetic error. Returns `true` if
/// the caller should fail.
///
/// The counter is incremented unconditionally (regardless of whether
/// a guard is active) so tests can read `SAVE_CONFIG_CALL_COUNT`
/// between operations to measure how many `save_config` calls a
/// given code path produces — essential for calibrating the
/// `fail_at` threshold of subsequent fault injection.
#[inline]
pub(crate) fn should_fail_save_config() -> bool {
    let current = SAVE_CONFIG_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    let threshold = SAVE_CONFIG_FAIL_AT.load(Ordering::SeqCst);
    if threshold == SAVE_CONFIG_FAIL_DISABLED {
        return false;
    }
    current == threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_fires_on_configured_call_index() {
        let _guard = SaveConfigFaultGuard::activate(2);
        assert!(!should_fail_save_config()); // call 0
        assert!(!should_fail_save_config()); // call 1
        assert!(should_fail_save_config()); // call 2 → fire
        assert!(!should_fail_save_config()); // call 3 → back to normal
    }

    #[test]
    fn test_guard_activate_on_first_call_fails_immediately() {
        let _guard = SaveConfigFaultGuard::activate_on_first_call();
        assert!(should_fail_save_config());
        assert!(!should_fail_save_config());
    }

    #[test]
    fn test_guard_clears_state_on_drop() {
        {
            let _guard = SaveConfigFaultGuard::activate(0);
            assert!(should_fail_save_config());
        }
        // After drop: counter reset, threshold cleared.
        assert_eq!(
            SAVE_CONFIG_FAIL_AT.load(Ordering::SeqCst),
            SAVE_CONFIG_FAIL_DISABLED
        );
        assert!(!should_fail_save_config());
    }

    #[test]
    fn test_guard_clears_flag_even_on_panic() {
        let result = std::panic::catch_unwind(|| {
            let _guard = SaveConfigFaultGuard::activate(0);
            assert!(should_fail_save_config());
            panic!("simulated test failure");
        });
        assert!(result.is_err());
        assert_eq!(
            SAVE_CONFIG_FAIL_AT.load(Ordering::SeqCst),
            SAVE_CONFIG_FAIL_DISABLED
        );
    }
}
