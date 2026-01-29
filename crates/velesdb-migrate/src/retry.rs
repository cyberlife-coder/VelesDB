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
    // Check error message for retryable patterns
    let error_msg = error.to_string().to_lowercase();

    // Rate limits are always retryable
    if matches!(error, Error::RateLimit(_)) {
        return true;
    }

    // IO errors are often transient
    if matches!(error, Error::Io(_)) {
        return true;
    }

    // HTTP errors - check for retryable status codes
    if matches!(error, Error::Http(_)) {
        return error_msg.contains("timeout")
            || error_msg.contains("connection")
            || error_msg.contains("reset");
    }

    // Check message content for retryable patterns
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

    // Source/Extraction errors with retryable messages
    match error {
        Error::SourceConnection(_) | Error::Extraction(_) => {
            is_rate_limit || is_transient || is_server_error
        }
        _ => is_rate_limit || is_transient || is_server_error,
    }
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    // ==================== RetryConfig Tests ====================

    #[test]
    fn test_retry_config_default() {
        // Arrange & Act
        let config = RetryConfig::default();

        // Assert
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay, Duration::from_millis(500));
        assert_eq!(config.max_delay, Duration::from_secs(30));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert!(config.add_jitter);
    }

    #[test]
    fn test_retry_config_for_rate_limits() {
        // Arrange & Act
        let config = RetryConfig::for_rate_limits();

        // Assert
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_delay, Duration::from_secs(1));
        assert_eq!(config.max_delay, Duration::from_secs(60));
    }

    #[test]
    fn test_retry_config_for_transient_errors() {
        // Arrange & Act
        let config = RetryConfig::for_transient_errors();

        // Assert
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay, Duration::from_millis(100));
    }

    #[test]
    fn test_retry_config_no_retry() {
        // Arrange & Act
        let config = RetryConfig::no_retry();

        // Assert
        assert_eq!(config.max_retries, 0);
    }

    #[test]
    fn test_delay_for_attempt_zero() {
        // Arrange
        let config = RetryConfig::default();

        // Act
        let delay = config.delay_for_attempt(0);

        // Assert
        assert_eq!(delay, Duration::ZERO);
    }

    #[test]
    fn test_delay_for_attempt_exponential() {
        // Arrange
        let config = RetryConfig {
            initial_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_delay: Duration::from_secs(100),
            add_jitter: false,
            ..Default::default()
        };

        // Act & Assert
        assert_eq!(config.delay_for_attempt(1), Duration::from_secs(1)); // 1 * 2^0 = 1
        assert_eq!(config.delay_for_attempt(2), Duration::from_secs(2)); // 1 * 2^1 = 2
        assert_eq!(config.delay_for_attempt(3), Duration::from_secs(4)); // 1 * 2^2 = 4
        assert_eq!(config.delay_for_attempt(4), Duration::from_secs(8)); // 1 * 2^3 = 8
    }

    #[test]
    fn test_delay_capped_at_max() {
        // Arrange
        let config = RetryConfig {
            initial_delay: Duration::from_secs(10),
            backoff_multiplier: 10.0,
            max_delay: Duration::from_secs(30),
            add_jitter: false,
            ..Default::default()
        };

        // Act
        let delay = config.delay_for_attempt(5); // Would be 10 * 10^4 = 100000 without cap

        // Assert
        assert_eq!(delay, Duration::from_secs(30));
    }

    // ==================== is_retryable_error Tests ====================

    #[test]
    fn test_retryable_rate_limit_429() {
        // Arrange
        let error = Error::SourceConnection("HTTP 429 Too Many Requests".to_string());

        // Act & Assert
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_retryable_rate_limit_text() {
        // Arrange
        let error = Error::SourceConnection("Rate limit exceeded, retry after 60s".to_string());

        // Act & Assert
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_retryable_timeout() {
        // Arrange
        let error = Error::SourceConnection("Connection timeout after 30s".to_string());

        // Act & Assert
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_retryable_server_error_500() {
        // Arrange
        let error = Error::SourceConnection("HTTP 500 Internal Server Error".to_string());

        // Act & Assert
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_retryable_server_error_503() {
        // Arrange
        let error = Error::SourceConnection("HTTP 503 Service Unavailable".to_string());

        // Act & Assert
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_retryable_connection_refused() {
        // Arrange
        let error = Error::SourceConnection("Connection refused".to_string());

        // Act & Assert
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_retryable_io_error() {
        // Arrange
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        let error = Error::Io(io_err);

        // Act & Assert
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_not_retryable_auth_error() {
        // Arrange
        let error = Error::Authentication("HTTP 401 Unauthorized".to_string());

        // Act & Assert
        assert!(!is_retryable_error(&error));
    }

    #[test]
    fn test_not_retryable_not_found() {
        // Arrange
        let error = Error::SourceConnection("HTTP 404 Not Found".to_string());

        // Act & Assert
        assert!(!is_retryable_error(&error));
    }

    #[test]
    fn test_not_retryable_config_error() {
        // Arrange
        let error = Error::Config("Invalid configuration".to_string());

        // Act & Assert
        assert!(!is_retryable_error(&error));
    }

    // ==================== with_retry Tests ====================

    #[tokio::test]
    async fn test_with_retry_success_first_try() {
        // Arrange
        let config = RetryConfig::no_retry();
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        // Act
        let result = with_retry(&config, "test_op", || {
            let count = call_count_clone.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>(42)
            }
        })
        .await;

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_with_retry_success_after_retries() {
        // Arrange
        let config = RetryConfig {
            max_retries: 3,
            initial_delay: Duration::from_millis(1), // Fast for tests
            add_jitter: false,
            ..Default::default()
        };
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        // Act
        let result = with_retry(&config, "test_op", || {
            let count = call_count_clone.clone();
            async move {
                let current = count.fetch_add(1, Ordering::SeqCst);
                if current < 2 {
                    // Fail first 2 times with retryable error
                    Err(Error::SourceConnection(
                        "HTTP 503 Service Unavailable".to_string(),
                    ))
                } else {
                    Ok::<_, Error>(42)
                }
            }
        })
        .await;

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // 2 failures + 1 success
    }

    #[tokio::test]
    async fn test_with_retry_all_attempts_fail() {
        // Arrange
        let config = RetryConfig {
            max_retries: 2,
            initial_delay: Duration::from_millis(1),
            add_jitter: false,
            ..Default::default()
        };
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        // Act
        let result: Result<i32> = with_retry(&config, "test_op", || {
            let count = call_count_clone.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Err(Error::SourceConnection(
                    "HTTP 500 Internal Server Error".to_string(),
                ))
            }
        })
        .await;

        // Assert
        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn test_with_retry_non_retryable_error_no_retry() {
        // Arrange
        let config = RetryConfig {
            max_retries: 5,
            initial_delay: Duration::from_millis(1),
            ..Default::default()
        };
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        // Act
        let result: Result<i32> = with_retry(&config, "test_op", || {
            let count = call_count_clone.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                // Non-retryable error (auth failure)
                Err(Error::Authentication("HTTP 401 Unauthorized".to_string()))
            }
        })
        .await;

        // Assert
        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // No retries for non-retryable
    }
}
