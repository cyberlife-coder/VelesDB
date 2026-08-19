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
pub fn is_retryable_error(error: &Error) -> bool {
    // Check struct-level variants first to avoid the string allocation.
    if matches!(error, Error::RateLimit(_)) {
        return true;
    }
    if matches!(error, Error::Io(_)) {
        return true;
    }

    // Fall back to message inspection — allocate the lowercase string once.
    let error_msg = error.to_string().to_lowercase();

    if matches!(error, Error::Http(_)) {
        return error_msg.contains("timeout")
            || error_msg.contains("connection")
            || error_msg.contains("reset");
    }

    let is_rate_limit = error_msg.contains("429")
        || error_msg.contains("rate limit")
        || error_msg.contains("too many requests");

    let is_transient = error_msg.contains("timeout")
        || error_msg.contains("connection refused")
        || error_msg.contains("connection reset")
        || error_msg.contains("temporary");

    let is_server_error = error_msg.contains("500")
        || error_msg.contains("502")
        || error_msg.contains("503")
        || error_msg.contains("504")
        || error_msg.contains("internal server error")
        || error_msg.contains("bad gateway")
        || error_msg.contains("service unavailable");

    is_rate_limit || is_transient || is_server_error
}

/// Executes an async operation with retry logic.
///
/// # Arguments
///
/// * `config` - Retry configuration
/// * `operation_name` - Name for logging purposes
/// * `operation` - The async operation to execute
///
/// # Returns
///
/// The result of the operation, or the last error if all retries failed.
#[allow(clippy::cognitive_complexity)] // Reason: Retry logic with backoff requires tracking multiple states
pub async fn with_retry<F, Fut, T>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last_error: Option<Error> = None;
    let max_attempts = config.max_retries + 1;

    for attempt in 0..max_attempts {
        if attempt > 0 {
            let delay = config.delay_for_attempt(attempt);
            debug!(
                "{}: Retry attempt {}/{} after {:?}",
                operation_name, attempt, config.max_retries, delay
            );
            sleep(delay).await;
        }

        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    debug!("{}: Succeeded after {} retries", operation_name, attempt);
                }
                return Ok(result);
            }
            Err(e) => {
                if is_retryable_error(&e) && attempt < config.max_retries {
                    warn!(
                        "{}: Retryable error (attempt {}/{}): {}",
                        operation_name,
                        attempt + 1,
                        max_attempts,
                        e
                    );
                    last_error = Some(e);
                } else {
                    // Non-retryable error or last attempt
                    return Err(e);
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| Error::Extraction("All retry attempts failed".to_string())))
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
