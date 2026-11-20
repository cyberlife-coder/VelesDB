//! Retry logic with exponential backoff for resilient network operations.
//!
//! This module provides utilities for retrying failed operations with
//! configurable backoff strategies, essential for reliable migrations
//! over unreliable networks or rate-limited APIs.

use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::error::{Error, Result};

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not including the initial attempt).
    pub max_retries: u32,
    /// Initial delay before the first retry.
    pub initial_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Multiplier for exponential backoff (e.g., 2.0 doubles delay each retry).
    pub backoff_multiplier: f64,
    /// Whether to add jitter to prevent thundering herd.
    pub add_jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            add_jitter: true,
        }
    }
}

impl RetryConfig {
    /// Creates a config optimized for API rate limits.
    pub fn for_rate_limits() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            add_jitter: true,
        }
    }

    /// Creates a config for quick retries on transient errors.
    pub fn for_transient_errors() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 2.0,
            add_jitter: true,
        }
    }

    /// Creates a config with no retries (for testing or when retries are unwanted).
    pub fn no_retry() -> Self {
        Self {
            max_retries: 0,
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            backoff_multiplier: 1.0,
            add_jitter: false,
        }
    }

    /// Calculates the delay for a given attempt number.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let base_delay = self.initial_delay.as_secs_f64()
            * self
                .backoff_multiplier
                .powi(attempt.saturating_sub(1) as i32);

        let capped_delay = base_delay.min(self.max_delay.as_secs_f64());

        let final_delay = if self.add_jitter {
            // Add up to 25% jitter
            let jitter = capped_delay * 0.25 * rand_jitter();
            capped_delay + jitter
        } else {
            capped_delay
        };

        Duration::from_secs_f64(final_delay)
    }
}

/// Simple pseudo-random jitter (0.0 to 1.0) without external dependencies.
fn rand_jitter() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 1000) as f64 / 1000.0
}

/// Determines if an error is retryable.
///
/// Decided on the error's type and the status, timeout or I/O kind it
/// carries — never on its message, which embeds source-controlled response
/// bodies: a `400` whose body happens to quote `500` is not a server error.
/// Errors that carry no transient signal (configuration, authentication,
/// schema, parse failures) are never retried.
pub fn is_retryable_error(error: &Error) -> bool {
    match error {
        Error::RateLimit(_) => true,
        Error::SourceStatus { status, .. } => is_retryable_status(*status),
        Error::Http(e) => is_retryable_http(e),
        Error::Io(e) => is_transient_io(e.kind()),
        _ => false,
    }
}

/// Request timeout, too many requests, and the server-side statuses that
/// signal a temporary condition. `501 Not Implemented` and the other 5xx
/// codes describe a permanent answer and are not retried.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// A request that timed out or never reached the server, or a response
/// whose status is itself retryable.
fn is_retryable_http(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || error.is_request()
        || error
            .status()
            .is_some_and(|status| is_retryable_status(status.as_u16()))
}

fn is_transient_io(kind: std::io::ErrorKind) -> bool {
    use std::io::ErrorKind;
    matches!(
        kind,
        ErrorKind::TimedOut
            | ErrorKind::Interrupted
            | ErrorKind::WouldBlock
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::BrokenPipe
    )
}

/// Executes an async operation with retry logic.
///
/// The operation runs once, then up to `config.max_retries` more times while
/// it fails with an error [`is_retryable_error`] accepts, sleeping
/// [`RetryConfig::delay_for_attempt`] before each retry.
///
/// # Errors
///
/// Returns the first non-retryable error, or the last error once the retries
/// are exhausted.
pub async fn with_retry<F, Fut, T>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut retries = 0;
    loop {
        match operation().await {
            Ok(result) => {
                if retries > 0 {
                    debug!("{operation_name}: succeeded after {retries} retries");
                }
                return Ok(result);
            }
            Err(e) if retries < config.max_retries && is_retryable_error(&e) => {
                retries += 1;
                let delay = config.delay_for_attempt(retries);
                warn!(
                    "{operation_name}: retryable error, retry {retries}/{} in {delay:?}: {e}",
                    config.max_retries
                );
                sleep(delay).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
