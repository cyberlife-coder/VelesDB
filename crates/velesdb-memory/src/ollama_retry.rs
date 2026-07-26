//! Synchronous retry and actionable failure reporting for this crate's blocking
//! Ollama calls (the [`crate::embedder`] embeddings POST and the
//! [`crate::extract`] generation POST).
//!
//! ## Why a local helper rather than `velesdb-migrate::retry`
//!
//! `velesdb-migrate` already carries a retry loop, but it is unusable here on
//! three counts: it is `async`/tokio while both Ollama call sites are blocking
//! `ureq`; it is typed against `velesdb_migrate::Error`; and `velesdb-memory`
//! does not depend on `velesdb-migrate` — adding that dependency would invert
//! the layering (the memory core would pull in the migration tool). Only the
//! *shape* of its `RetryConfig` is carried over.
//!
//! ## Why the classifier looks at variants, never at text
//!
//! `velesdb-migrate`'s `is_retryable_error` searches the rendered message for
//! the words "timeout"/"connection"/"reset". That is a guess about how a
//! dependency happens to format itself today: a wording change silently flips
//! a retry decision, and a 404 whose body mentions "connection" is replayed for
//! nothing. `ureq` exposes everything needed structurally — [`ureq::Error`] is
//! a two-variant enum, `Transport::kind()` gives an [`ureq::ErrorKind`], and
//! `Error::source()` hands back the underlying [`std::io::Error`] — so every
//! decision below is taken on a variant.

use std::time::Duration;

/// Exponential-backoff schedule for a retried operation.
#[derive(Debug, Clone)]
pub(crate) struct RetryConfig {
    /// Replays allowed *in addition to* the first attempt.
    max_retries: u32,
    /// Delay before the first replay.
    initial_delay: Duration,
    /// Ceiling applied to every computed delay.
    max_delay: Duration,
    /// Growth factor applied per replay (`2.0` doubles each time).
    backoff_multiplier: f64,
}

/// The schedule every Ollama call uses: two replays, 100 ms then 200 ms.
///
/// Deliberately small. The failure this exists for — a keep-alive connection
/// the server closed under us — is fixed by the *second* attempt, which dials a
/// fresh connection. A longer schedule would only pile load onto an Ollama that
/// is already struggling, and would inflate the worst case of
/// `remember_extracted`, which issues one embed per fact *and* one per entity
/// hub. Total added latency on a hard failure: ~300 ms.
pub(crate) const OLLAMA_RETRIES: RetryConfig = RetryConfig {
    max_retries: 2,
    initial_delay: Duration::from_millis(100),
    max_delay: Duration::from_secs(1),
    backoff_multiplier: 2.0,
};

impl RetryConfig {
    /// Delay to observe before replay number `attempt` (1-based).
    fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        let exponent = i32::try_from(attempt.saturating_sub(1)).unwrap_or(i32::MAX);
        let seconds = self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi(exponent);
        Duration::from_secs_f64(seconds.min(self.max_delay.as_secs_f64()))
    }
}

/// Is this HTTP status worth another attempt?
///
/// `5xx` and `429` describe a server that is momentarily unable, not a request
/// that is wrong. Every other `4xx` is a verdict on the request itself — a
/// missing model (404) or a malformed body (400) answers the same way forever,
/// so replaying it only wastes the caller's deadline.
pub(crate) fn status_is_retryable(status: u16) -> bool {
    status >= 500 || status == 429
}

/// Is this I/O failure worth another attempt?
///
/// Everything transient at the socket layer is — a reset, an abort, a refused
/// connection, a half-closed pipe, a truncated body. The one carve-out is a
/// **timeout**: a timeout means the caller's whole budget was already spent
/// waiting, so replaying it multiplies the worst case instead of repairing
/// anything (`extract.rs` would go from 300 s to 900 s per call). The carve-out
/// is still purely structural — `std::io::ErrorKind`, not a substring.
pub(crate) fn io_is_retryable(err: &std::io::Error) -> bool {
    !matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::InvalidInput
    )
}

/// Transport kinds that describe the *network*, not the request. Anything else
/// (`InvalidUrl`, `BadStatus`, `TooManyRedirects`, …) is deterministic: the
/// same call would fail identically, so it is reported at once.
fn kind_is_transient(kind: ureq::ErrorKind) -> bool {
    matches!(
        kind,
        ureq::ErrorKind::Dns
            | ureq::ErrorKind::ConnectionFailed
            | ureq::ErrorKind::ProxyConnect
            | ureq::ErrorKind::Io
    )
}

/// Is this `ureq` failure worth another attempt? Decided on the enum variant,
/// the [`ureq::ErrorKind`], and the underlying [`std::io::ErrorKind`] — never on
/// the rendered message.
pub(crate) fn is_retryable(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::Status(status, _) => status_is_retryable(*status),
        ureq::Error::Transport(transport) => {
            if !kind_is_transient(transport.kind()) {
                return false;
            }
            // A transient kind with no `io::Error` underneath (e.g. a DNS
            // failure ureq reports on its own) still deserves one replay.
            std::error::Error::source(transport)
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .is_none_or(io_is_retryable)
        }
    }
}

/// Run `op` until it succeeds, until `retryable` refuses the error, or until
/// the schedule is exhausted — synchronously, on the calling thread, with no
/// tokio anywhere.
///
/// On failure it returns the last error **and the number of attempts actually
/// made**. That counter is not bookkeeping: it is what lets the caller say "gave
/// up after 3 attempts" instead of a bare transport string, which is precisely
/// the information the previous one-shot code could not provide.
pub(crate) fn with_retry<T, E>(
    config: &RetryConfig,
    retryable: impl Fn(&E) -> bool,
    mut op: impl FnMut() -> Result<T, E>,
) -> Result<T, (E, u32)> {
    let mut attempts: u32 = 0;
    loop {
        attempts = attempts.saturating_add(1);
        let err = match op() {
            Ok(value) => return Ok(value),
            Err(err) => err,
        };
        if attempts > config.max_retries || !retryable(&err) {
            return Err((err, attempts));
        }
        std::thread::sleep(config.delay_for_attempt(attempts));
    }
}

/// The environment variables that actually govern one Ollama call site.
///
/// Taken as parameters rather than hardcoded because the two call sites are
/// configured by **different** variables: the embedder reads
/// `VELESDB_MEMORY_OLLAMA_URL`/`_MODEL`, the extractor reads
/// `VELESDB_MEMORY_EXTRACTOR_URL`/`_MODEL` (see `main.rs`). Naming the wrong
/// pair would send the user to edit a setting that has no effect — worse than
/// saying nothing.
pub(crate) struct OllamaLevers<'a> {
    /// Variable that repoints the base URL.
    pub url_var: &'a str,
    /// Variable that selects the model.
    pub model_var: &'a str,
    /// Optional escape hatch sentence (e.g. the fully-offline embedder).
    pub fallback: Option<&'a str>,
}

/// Render a failure the reader can act on: what was called, against which
/// model, how many times it was tried, why it failed, and which knobs change
/// the outcome. Modelled on `main.rs`'s hash-embedder notice, which already
/// states a trade-off and points at its opt-in.
pub(crate) fn actionable_failure(
    endpoint: &str,
    url: &str,
    model: &str,
    attempts: u32,
    cause: &str,
    levers: &OllamaLevers<'_>,
) -> String {
    let plural = if attempts == 1 { "attempt" } else { "attempts" };
    let mut message = format!(
        "ollama {endpoint} call failed after {attempts} {plural}: POST {url} \
         (model '{model}'): {cause}. Check Ollama is running and the model is \
         pulled (`ollama pull {model}`). Point elsewhere with {} / {}",
        levers.url_var, levers.model_var
    );
    if let Some(fallback) = levers.fallback {
        message.push_str(", or ");
        message.push_str(fallback);
    }
    message.push('.');
    message
}

#[cfg(test)]
mod tests {
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
        let response =
            ureq::Response::new(503, "Service Unavailable", "loading").expect("response");
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
        let loud =
            ureq::Response::new(404, "Not Found", "connection reset timeout").expect("response");
        assert_eq!(
            is_retryable(&ureq::Error::Status(404, quiet)),
            is_retryable(&ureq::Error::Status(404, loud))
        );
    }

    #[test]
    fn with_retry_stops_early_on_a_non_retryable_error() {
        let mut calls = 0_u32;
        let outcome: Result<(), (&str, u32)> = with_retry(
            &OLLAMA_RETRIES,
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
        let outcome: Result<(), (&str, u32)> =
            with_retry(&OLLAMA_RETRIES, |_| true, || Err("transient"));
        assert!(
            matches!(outcome, Err((_, 3))),
            "one attempt plus two replays, and the count must reach the caller"
        );
    }

    #[test]
    fn the_backoff_grows_and_stays_capped() {
        assert_eq!(OLLAMA_RETRIES.delay_for_attempt(0), Duration::ZERO);
        assert_eq!(
            OLLAMA_RETRIES.delay_for_attempt(1),
            Duration::from_millis(100)
        );
        assert_eq!(
            OLLAMA_RETRIES.delay_for_attempt(2),
            Duration::from_millis(200)
        );
        assert_eq!(
            OLLAMA_RETRIES.delay_for_attempt(99),
            OLLAMA_RETRIES.max_delay,
            "the schedule must not drift into a long sleep"
        );
    }

    #[test]
    fn the_failure_message_names_the_levers_of_its_own_call_site() {
        let message = actionable_failure(
            "embeddings",
            "http://localhost:11434/api/embeddings",
            "all-minilm",
            3,
            "Network Error: Connection reset by peer (os error 54)",
            &OllamaLevers {
                url_var: "VELESDB_MEMORY_OLLAMA_URL",
                model_var: "VELESDB_MEMORY_OLLAMA_MODEL",
                fallback: Some(
                    "fall back to the offline embedder with VELESDB_MEMORY_EMBEDDER=hash",
                ),
            },
        );
        assert!(message.contains("3 attempts"));
        assert!(message.contains("http://localhost:11434/api/embeddings"));
        assert!(message.contains("all-minilm"));
        assert!(message.contains("VELESDB_MEMORY_OLLAMA_URL"));
        assert!(message.contains("VELESDB_MEMORY_EMBEDDER=hash"));
    }
}
