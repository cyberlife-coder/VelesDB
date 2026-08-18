use super::*;
use std::io;

/// Build a transport error carrying `kind` as its `io::Error` source, the
/// exact shape `ureq` produces for a socket failure.
fn transport(kind: io::ErrorKind, message: &str) -> ureq::Error {
    ureq::Error::from(io::Error::new(kind, message.to_owned()))
}

/// The observed field failure: `Connection reset by peer (os error 54)`
/// against an Ollama whose `/api/tags` answered in 7 ms — a stale pooled
/// keep-alive connection, repaired by dialling again.
#[test]
fn a_connection_reset_is_replayable() {
    assert!(is_retryable(&transport(
        io::ErrorKind::ConnectionReset,
        "Connection reset by peer (os error 54)"
    )));
}

#[test]
fn a_refused_connection_is_replayable() {
    assert!(is_retryable(&transport(
        io::ErrorKind::ConnectionRefused,
        "Connection refused"
    )));
}

/// A body that stops mid-flight. Untested before this module existed, and
/// indistinguishable from success to the old code until the JSON parse blew
/// up with an opaque "invalid embeddings response".
#[test]
fn a_truncated_body_is_replayable() {
    assert!(is_retryable(&transport(
        io::ErrorKind::UnexpectedEof,
        "response body closed before all bytes were read"
    )));
    assert!(io_is_retryable(&io::Error::from(
        io::ErrorKind::UnexpectedEof
    )));
}

/// The carve-out that keeps the fix from making things worse: the budget is
/// already spent, so replaying triples the worst case.
#[test]
fn an_exhausted_timeout_is_not_replayable() {
    assert!(!is_retryable(&transport(
        io::ErrorKind::TimedOut,
        "timed out reading response"
    )));
    assert!(!io_is_retryable(&io::Error::from(io::ErrorKind::TimedOut)));
}

#[test]
fn a_client_error_is_not_replayable() {
    let response = ureq::Response::new(404, "Not Found", "model not found").expect("response");
    assert!(!is_retryable(&ureq::Error::Status(404, response)));
    assert!(!status_is_retryable(400));
    assert!(!status_is_retryable(404));
}

#[test]
fn a_server_error_is_replayable() {
    let response = ureq::Response::new(503, "Service Unavailable", "loading").expect("response");
    assert!(is_retryable(&ureq::Error::Status(503, response)));
    assert!(status_is_retryable(429));
    assert!(status_is_retryable(500));
}

/// Non-regression, stated on purpose: the decision must come from the
/// variant alone. `velesdb-migrate`'s string-matching classifier cannot
/// hold this property — it would replay the second error below (its text
/// contains "connection") and refuse the first.
#[test]
fn the_classifier_never_reads_the_error_text() {
    let terse = transport(io::ErrorKind::ConnectionReset, "");
    let chatty = transport(
        io::ErrorKind::ConnectionReset,
        "the connection was reset while the request timed out mid-flight",
    );
    assert_eq!(is_retryable(&terse), is_retryable(&chatty));

    let quiet = ureq::Response::new(404, "Not Found", "").expect("response");
    let loud = ureq::Response::new(404, "Not Found", "connection reset timeout").expect("response");
    assert_eq!(
        is_retryable(&ureq::Error::Status(404, quiet)),
        is_retryable(&ureq::Error::Status(404, loud))
    );
}

#[test]
fn with_retry_stops_early_on_a_non_retryable_error() {
    let mut calls = 0_u32;
    let outcome: Result<(), (&str, u32)> = with_retry(
        &HTTP_RETRIES,
        |_| false,
        || {
            calls += 1;
            Err("deterministic")
        },
    );
    assert_eq!(calls, 1, "a deterministic failure must not be replayed");
    assert!(matches!(outcome, Err(("deterministic", 1))));
}

#[test]
fn with_retry_reports_the_attempt_count() {
    let outcome: Result<(), (&str, u32)> = with_retry(&HTTP_RETRIES, |_| true, || Err("transient"));
    assert!(
        matches!(outcome, Err((_, 3))),
        "one attempt plus two replays, and the count must reach the caller"
    );
}

#[test]
fn the_backoff_grows_and_stays_capped() {
    assert_eq!(HTTP_RETRIES.delay_for_attempt(0), Duration::ZERO);
    assert_eq!(
        HTTP_RETRIES.delay_for_attempt(1),
        Duration::from_millis(100)
    );
    assert_eq!(
        HTTP_RETRIES.delay_for_attempt(2),
        Duration::from_millis(200)
    );
    assert_eq!(
        HTTP_RETRIES.delay_for_attempt(99),
        HTTP_RETRIES.max_delay,
        "the schedule must not drift into a long sleep"
    );
}

#[test]
fn the_failure_message_names_the_levers_of_its_own_call_site() {
    let message = actionable_ollama_failure(
        "embeddings",
        "http://localhost:11434/api/embeddings",
        "all-minilm",
        3,
        "Network Error: Connection reset by peer (os error 54)",
        &FailureLevers {
            url_var: "VELESDB_MEMORY_OLLAMA_URL",
            model_var: "VELESDB_MEMORY_OLLAMA_MODEL",
            fallback: Some("fall back to the offline embedder with VELESDB_MEMORY_EMBEDDER=hash"),
        },
    );
    assert!(message.contains("3 attempts"));
    assert!(message.contains("http://localhost:11434/api/embeddings"));
    assert!(message.contains("all-minilm"));
    assert!(message.contains("VELESDB_MEMORY_OLLAMA_URL"));
    assert!(message.contains("VELESDB_MEMORY_EMBEDDER=hash"));
}
