use super::{run_blocking_search, run_search_with_optional_timeout, TimeoutElapsed};

/// The budget elapses deterministically: the work cannot finish until the
/// test releases it, which it does only after the wrapper has returned.
#[tokio::test]
async fn test_a_budget_that_runs_out_is_reported_as_elapsed() {
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let outcome = run_search_with_optional_timeout(Some(1), move || {
        let _ = wait.recv();
        Ok(Ok(Vec::new()))
    })
    .await;
    assert!(matches!(outcome, Err(TimeoutElapsed)));
    let _ = release.send(());
}

/// A worker that panics is the engine's fault: a core outcome carrying
/// `VELES-013`, which the handlers record with the circuit breaker — not a
/// refusal on the `Err` side, which they never record.
#[tokio::test]
async fn test_a_panicked_worker_is_an_internal_engine_failure() {
    let outcome = run_blocking_search(|| panic!("boom")).await;
    let Ok(core_outcome) = outcome else {
        panic!("a panicked worker was answered as a refusal");
    };
    let error = core_outcome.expect_err("a panicked worker returned results");
    assert_eq!(error.code(), "VELES-013", "{error}");
    assert!(!error.is_caller_error(), "{error}");
}
