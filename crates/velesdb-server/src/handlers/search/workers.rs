//! Async worker wrappers for CPU-bound search operations.
//!
//! Moves synchronous search closures onto `spawn_blocking` workers with
//! optional per-request timeout support. Extracted from `pipeline.rs`
//! (Extract Module — Fowler) to keep file NLOC under the 500-line limit.

use crate::handlers::helpers::worker_failure;

/// Marker error returned when a per-request timeout elapses before the
/// blocking worker finishes.
pub(crate) struct TimeoutElapsed;

/// Outcome of a synchronous search closure: core search result, or HTTP error response.
pub(crate) type SearchOutcome =
    Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response>;

/// Outcome of the timed wrapper: outer `Err` signals the budget expired.
pub(crate) type TimedSearchOutcome = Result<SearchOutcome, TimeoutElapsed>;

/// Runs the sync search pipeline on a `spawn_blocking` worker with an optional
/// timeout, already resolved against the server's limit
/// ([`request_timeout_ms`](super::outcome::request_timeout_ms)). On expiry
/// returns `Err(TimeoutElapsed)`; the blocking task keeps running (Tokio
/// cannot cancel sync code) and its result is discarded.
#[allow(clippy::result_large_err)]
pub(crate) async fn run_search_with_optional_timeout<F>(
    timeout_ms: Option<u64>,
    work: F,
) -> TimedSearchOutcome
where
    F: FnOnce() -> SearchOutcome + Send + 'static,
{
    let handle = tokio::task::spawn_blocking(work);
    match timeout_ms {
        Some(ms) => await_with_timeout(handle, ms).await,
        None => Ok(unwrap_join(handle.await)),
    }
}

/// Awaits a join handle under a millisecond budget.
#[allow(clippy::result_large_err)]
async fn await_with_timeout(
    handle: tokio::task::JoinHandle<SearchOutcome>,
    ms: u64,
) -> TimedSearchOutcome {
    let duration = std::time::Duration::from_millis(ms);
    match tokio::time::timeout(duration, handle).await {
        Ok(join_result) => Ok(unwrap_join(join_result)),
        Err(_elapsed) => Err(TimeoutElapsed),
    }
}

/// Executes a synchronous search closure on a `spawn_blocking` worker,
/// without a timeout budget. This is the lighter-weight sibling of
/// [`run_search_with_optional_timeout`] used by handlers that do not
/// currently expose a per-request timeout (text search, hybrid
/// search, batch search) but still need to keep CPU-bound work off
/// the async runtime.
///
/// Finding F-01 of the pre-seed audit (spawn_blocking sweep).
#[allow(clippy::result_large_err)]
pub(crate) async fn run_blocking_search<F>(work: F) -> SearchOutcome
where
    F: FnOnce() -> SearchOutcome + Send + 'static,
{
    unwrap_join(tokio::task::spawn_blocking(work).await)
}

/// Converts a `JoinHandle` result into the same shape expected by
/// callers of the synchronous search pipeline. A panic or cancellation of
/// the blocking task is the engine's fault: it comes back as the core
/// outcome [`worker_failure`] (`VELES-013`, `500`), which the handlers record
/// with the circuit breaker like any other engine failure — never on the
/// `Err` side, which carries only refusals made before any search ran.
#[allow(clippy::result_large_err)]
fn unwrap_join(join_result: Result<SearchOutcome, tokio::task::JoinError>) -> SearchOutcome {
    match join_result {
        Ok(inner) => inner,
        Err(join_err) => Ok(Err(worker_failure("search", &join_err))),
    }
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod workers_tests;
