//! How a search's outcome becomes a response and a circuit-breaker record.
//!
//! Every search error is answered with its canonical status
//! ([`http_status_for_error`]), which reads core's `Error::is_caller_error` —
//! the same classification the circuit breaker counts by — so a `4xx` answer
//! and "does not open the breaker" cannot drift apart. Three kinds of refusal
//! never reach the breaker: a request refused before any search ran (a bad
//! body), a read the access gate denied ([`read_refused`]), and a timeout
//! under the request's own budget ([`search_timeout_response`]).

use axum::{response::IntoResponse, Json};
use velesdb_core::collection::VectorCollection;

use crate::handlers::helpers::http_status_for_error;
use crate::types::{
    ErrorResponse, IdScoreResult, SearchIdsResponse, SearchResponse, SearchResultResponse,
};
use crate::AppState;

/// Convert a `Vec<SearchResult>` into a `SearchResponse`.
pub(crate) fn build_search_response(results: Vec<velesdb_core::SearchResult>) -> SearchResponse {
    SearchResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
    }
}

/// The error body of a failed search: the core error with its `VELES-XXX`
/// code and a hint at the likely fix.
pub(crate) fn actionable_search_error(error: &velesdb_core::Error) -> ErrorResponse {
    let base_error = error.to_string();
    let lower = base_error.to_lowercase();
    let hint = if lower.contains("dimension") {
        " Hint: check that query vector dimension matches collection dimension."
    } else if lower.contains("filter") {
        " Hint: validate filter syntax and start with a broader query before reintroducing strict filters."
    } else {
        " Hint: if you get empty results, retry without strict filters/thresholds, then tighten progressively."
    };

    ErrorResponse {
        error: format!("{base_error}{hint}"),
        code: Some(error.code().to_string()),
    }
}

/// The response for a failed search: the error's canonical status and its
/// actionable body.
pub(crate) fn search_error_response(error: &velesdb_core::Error) -> axum::response::Response {
    (
        http_status_for_error(error),
        Json(actionable_search_error(error)),
    )
        .into_response()
}

/// The response for a read the control-plane gate refused (the observer's
/// `AccessDecision::Deny`, or its own failure): the denial error's canonical
/// status, the same on every search endpoint.
///
/// Returned on the `Err` side of the execute functions, which the handlers
/// never record with the circuit breaker: a denial is the policy's answer,
/// not a sign the collection is unhealthy, whatever error carries it.
pub(crate) fn read_refused(refusal: &velesdb_core::Error) -> axum::response::Response {
    search_error_response(refusal)
}

/// Record empty-results diagnostic and notify the query timing subsystem.
fn record_search_metrics(state: &AppState, name: &str, start: std::time::Instant, is_empty: bool) {
    if is_empty {
        state.onboarding_metrics.record_empty_search_results();
    }
    let elapsed = start.elapsed();
    let duration_us = elapsed.as_micros();
    #[allow(clippy::cast_possible_truncation, deprecated)]
    // Reason: value is clamped to u64::MAX above, so the truncation is lossless.
    // `notify_query` is deprecated for callers that route through the VelesQL DML
    // path; the REST search pipeline does not, so core never fires `on_query` for
    // it and this shim is the sole, non-double-counting telemetry source.
    state
        .db
        .notify_query(name, duration_us.min(u128::from(u64::MAX)) as u64);
    // Record into Prometheus histogram (seconds).
    state
        .query_duration_histogram
        .observe(elapsed.as_secs_f64());
}

/// Core search result handler: records metrics, delegates success to `on_ok`,
/// answers a failure with [`search_error_response`].
fn finish_search_core(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
    on_ok: impl FnOnce(Vec<velesdb_core::SearchResult>) -> axum::response::Response,
) -> axum::response::Response {
    match search_result {
        Ok(results) => {
            record_search_metrics(state, name, start, results.is_empty());
            on_ok(results)
        }
        Err(e) => {
            state.operational_metrics.inc_errors();
            search_error_response(&e)
        }
    }
}

/// Shared result-handling for all search modes.
pub(crate) fn finish_search(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    finish_search_core(state, name, start, search_result, |results| {
        Json(build_search_response(results)).into_response()
    })
}

/// Maps search results to IDs+scores response with timing metrics.
pub(crate) fn finish_search_ids(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    finish_search_core(state, name, start, search_result, |results| {
        let response = SearchIdsResponse {
            results: results
                .into_iter()
                .map(|r| IdScoreResult {
                    id: r.point.id,
                    score: r.score,
                })
                .collect(),
        };
        Json(response).into_response()
    })
}

/// Record circuit-breaker outcome based on a search result: a success, or a
/// failure counted only when the error says the engine is unhealthy
/// (`CircuitBreaker::record_error`) — a caller's error never opens the breaker.
pub(crate) fn record_circuit_breaker<T>(
    collection: &VectorCollection,
    result: &velesdb_core::Result<T>,
) {
    match result {
        Ok(_) => collection.guard_rails().circuit_breaker.record_success(),
        Err(e) => collection.guard_rails().circuit_breaker.record_error(e),
    }
}

/// Handles `Ok`/`Err` from a core search call: records circuit-breaker
/// outcome and delegates to [`finish_search`] for metrics + response.
pub(crate) fn finish_search_with_cb(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    collection: &VectorCollection,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    record_circuit_breaker(collection, &search_result);
    finish_search(state, name, start, search_result)
}

/// Handles `Ok`/`Err` from a core search call: records circuit-breaker
/// outcome and delegates to [`finish_search_ids`] for metrics + response.
pub(crate) fn finish_search_ids_with_cb(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    collection: &VectorCollection,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    record_circuit_breaker(collection, &search_result);
    finish_search_ids(state, name, start, search_result)
}

/// The time budget a REST search's `timeout_ms` runs under, resolved against
/// the collection's limit by the same
/// [`QueryLimits::client_timeout_ms`](velesdb_core::QueryLimits::client_timeout_ms)
/// that `VelesQL` `WITH (timeout_ms = N)` uses: never longer than the limit,
/// and `0` keeps it. `None` when the request sets no budget, or sets `0`
/// with no server limit — the search then runs without a REST-side timer.
pub(crate) fn request_timeout_ms(
    collection: &VectorCollection,
    requested_ms: Option<u64>,
) -> Option<u64> {
    let effective = collection
        .guard_rails()
        .limits()
        .client_timeout_ms(requested_ms?);
    (effective > 0).then_some(effective)
}

/// Handles a search that ran past the request's `timeout_ms`.
///
/// Whose timeout it is comes from core's one classification,
/// [`QueryLimits::timeout_budget`](velesdb_core::QueryLimits::timeout_budget),
/// the one `VelesQL` `WITH (timeout_ms)` uses: the client's when its budget is
/// strictly tighter than the server's `timeout_ms` limit (`408`, not counted
/// by the breaker), the engine's otherwise — the search ran at least as long
/// as the server's own limit (`503`, counted). The response names the
/// collection and the budget so that clients can log actionable diagnostics.
pub(crate) fn search_timeout_response(
    collection: &VectorCollection,
    collection_name: &str,
    timeout_ms: u64,
) -> axum::response::Response {
    let budget = collection.guard_rails().limits().timeout_budget(timeout_ms);
    let message = format!(
        "Search on collection '{collection_name}' exceeded the \
         requested timeout of {timeout_ms}ms. The server returned \
         early; the in-flight query may continue in the background \
         until completion.",
    );
    let timeout = velesdb_core::Error::GuardRail {
        kind: velesdb_core::GuardRailKind::Timeout(budget),
        message: message.clone(),
    };
    collection
        .guard_rails()
        .circuit_breaker
        .record_error(&timeout);
    (
        http_status_for_error(&timeout),
        Json(ErrorResponse {
            error: message,
            code: Some(timeout.code().to_string()),
        }),
    )
        .into_response()
}

#[cfg(test)]
#[path = "timeout_response_tests.rs"]
mod timeout_response_tests;
