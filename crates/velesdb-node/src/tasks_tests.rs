//! The task trampoline: a closure's result comes back as is, and a second
//! `compute` or a panic becomes an error instead of unwinding into Node.
//!
//! The crate denies `expect`, `unwrap` and `panic!` everywhere, tests
//! included: results are asserted through `ok()` / `err()`, and the panics
//! under test are raised with `std::panic::resume_unwind`.

use super::*;

/// The reason of the error `result` holds, or `None` for a value.
fn error_reason(result: Result<u32>) -> Option<String> {
    result.err().map(|err| err.reason)
}

#[test]
fn compute_returns_closure_result() {
    let mut job: Job<u32> = Job::new(|| Ok(7));
    assert_eq!(job.compute().ok(), Some(7));
}

#[test]
fn compute_twice_is_an_error_not_a_panic() {
    let mut job: Job<u32> = Job::new(|| Ok(7));
    let _ = job.compute();
    let reason = error_reason(job.compute());
    assert!(
        reason.as_deref().is_some_and(|r| r.contains("[INTERNAL]")),
        "second compute must fail, got: {reason:?}"
    );
}

#[test]
fn compute_converts_panic_into_error() {
    let mut job: Job<u32> =
        Job::new(|| std::panic::resume_unwind(Box::new("boom in background job")));
    let reason = error_reason(job.compute());
    assert!(
        reason
            .as_deref()
            .is_some_and(|r| r.contains("boom in background job")),
        "panic message must be preserved, got: {reason:?}"
    );
}

#[test]
fn compute_handles_non_string_panic_payload() {
    let mut job: Job<u32> = Job::new(|| std::panic::resume_unwind(Box::new(42_i32)));
    let reason = error_reason(job.compute());
    assert!(
        reason
            .as_deref()
            .is_some_and(|r| r.contains("non-string panic payload")),
        "got: {reason:?}"
    );
}
