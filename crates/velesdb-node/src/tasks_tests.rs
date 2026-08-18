use super::*;

#[test]
fn compute_returns_closure_result() {
    let mut job: Job<u32> = Job::new(|| Ok(7));
    assert_eq!(job.compute().expect("closure result"), 7);
}

#[test]
fn compute_twice_is_an_error_not_a_panic() {
    let mut job: Job<u32> = Job::new(|| Ok(7));
    let _ = job.compute();
    let err = job.compute().expect_err("second compute must fail");
    assert!(err.reason.contains("[INTERNAL]"), "got: {}", err.reason);
}

#[test]
fn compute_converts_panic_into_error() {
    let mut job: Job<u32> = Job::new(|| panic!("boom in background job"));
    let err = job.compute().expect_err("panic must become an error");
    assert!(
        err.reason.contains("boom in background job"),
        "panic message must be preserved, got: {}",
        err.reason
    );
}

#[test]
fn compute_handles_non_string_panic_payload() {
    let mut job: Job<u32> = Job::new(|| std::panic::panic_any(42_i32));
    let err = job.compute().expect_err("panic must become an error");
    assert!(
        err.reason.contains("non-string panic payload"),
        "got: {}",
        err.reason
    );
}
