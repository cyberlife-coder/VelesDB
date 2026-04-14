//! Async worker wrappers for CPU-bound search operations.
//!
//! Moves synchronous search closures onto `spawn_blocking` workers with
//! optional per-request timeout support. Extracted from `pipeline.rs`
//! (Extract Module — Fowler) to keep file NLOC under the 500-line limit.

use axum::{http::StatusCode, response::IntoResponse, Json};

use crate::types::ErrorResponse;

/// Executes the synchronous search pipeline on a `spawn_blocking` worker
/// with an optional per-request timeout.
///
/// # Contract
///
/// * When `timeout_ms` is `None`, the search runs on a blocking worker
///   and the future simply awaits its completion. No artificial timeout
///   is applied; the only bound is whatever the collection-level guard
///   rails enforce.
/// * When `timeout_ms` is `Some`, the blocking join handle is wrapped
///   in `tokio::time::timeout`. If the budget elapses first, the helper
///   returns `Err(TimeoutElapsed)` and the caller is expected to emit a
///   408 response via [`super::pipeline::timeout_response`]. The spawned
///   blocking task is **not** cancelled — synchronous Rust code cannot be
///   interrupted mid-flight by Tokio — and will continue to execute until
///   completion (its result is then discarded). This is the standard Tokio
///   pattern for bounding the latency observed by clients while keeping the
///   async runtime responsive.
///
/// # Parameters
///
/// The closure is given ownership of the [`SearchRequest`] because the
/// inner pipeline takes `&mut SearchRequest` to drain sparse vector
/// fields via `Option::take()`.
#[allow(clippy::result_large_err)]
pub(crate) async fn run_search_with_optional_timeout<F>(
    timeout_ms: Option<u64>,
    work: F,
) -> Result<
    Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response>,
    TimeoutElapsed,
>
where
    F: FnOnce() -> Result<
            velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
            axum::response::Response,
        > + Send
        + 'static,
{
    // A zero-millisecond budget is treated as an immediate short-circuit:
    // we do not even spawn the blocking worker before returning the
    // timeout signal. This guarantees deterministic behaviour for test
    // fixtures that want to exercise the 408 path without depending on
    // the runtime scheduler to win a race against `Duration::ZERO`, and
    // matches the intuitive semantic that "zero budget" means "no budget
    // to spend".
    if matches!(timeout_ms, Some(0)) {
        return Err(TimeoutElapsed);
    }

    let handle = tokio::task::spawn_blocking(work);

    match timeout_ms {
        Some(ms) => {
            let duration = std::time::Duration::from_millis(ms);
            match tokio::time::timeout(duration, handle).await {
                Ok(join_result) => Ok(unwrap_join(join_result)),
                Err(_elapsed) => Err(TimeoutElapsed),
            }
        }
        None => Ok(unwrap_join(handle.await)),
    }
}

/// Marker error returned by [`run_search_with_optional_timeout`] when
/// the per-request timeout elapses before the blocking worker finishes.
pub(crate) struct TimeoutElapsed;

/// Executes a synchronous search closure on a `spawn_blocking` worker,
/// without a timeout budget. This is the lighter-weight sibling of
/// [`run_search_with_optional_timeout`] used by handlers that do not
/// currently expose a per-request timeout (text search, hybrid
/// search, batch search) but still need to keep CPU-bound work off
/// the async runtime.
///
/// Finding F-01 of the pre-seed audit (spawn_blocking sweep).
#[allow(clippy::result_large_err)]
pub(crate) async fn run_blocking_search<F>(
    work: F,
) -> Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response>
where
    F: FnOnce() -> Result<
            velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
            axum::response::Response,
        > + Send
        + 'static,
{
    unwrap_join(tokio::task::spawn_blocking(work).await)
}

/// Converts a `JoinHandle` result into the same shape expected by
/// callers of the synchronous search pipeline. A panic or cancellation
/// of the blocking task is reported as a 500 Internal Server Error.
#[allow(clippy::result_large_err)]
fn unwrap_join(
    join_result: Result<
        Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response>,
        tokio::task::JoinError,
    >,
) -> Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response> {
    match join_result {
        Ok(inner) => inner,
        Err(join_err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Search worker task failed: {join_err}"),
                code: Some("VELES-INTERNAL-WORKER-FAILURE".to_string()),
            }),
        )
            .into_response()),
    }
}
