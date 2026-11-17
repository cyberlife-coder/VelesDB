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

/// The error `connectors::common::check_response` returns for `status`.
fn source_status(status: u16, body: &str) -> Error {
    crate::connectors::common::handle_http_error(status, body, "Test")
}

#[test]
fn test_retryable_rate_limit() {
    assert!(is_retryable_error(&Error::RateLimit(60)));
    assert!(is_retryable_error(&source_status(429, "Too Many Requests")));
}

#[test]
fn test_retryable_transient_statuses() {
    for status in [408, 500, 502, 503, 504] {
        assert!(
            is_retryable_error(&source_status(status, "scroll failed")),
            "HTTP {status} must be retried"
        );
    }
}

#[test]
fn test_not_retryable_permanent_statuses() {
    for status in [400, 404, 409, 422, 501] {
        assert!(
            !is_retryable_error(&source_status(status, "scroll failed")),
            "HTTP {status} must not be retried"
        );
    }
}

/// Regression: the status used to be read back out of the message, so a
/// client error whose body quoted a server-error code was retried.
#[test]
fn test_not_retryable_client_error_quoting_server_code() {
    let error = source_status(400, "scroll failed: limit must be <= 500, timeout too long");
    assert!(error.to_string().contains("500"));
    assert!(!is_retryable_error(&error));
}

/// Regression: any message containing `500`, `timeout` or `temporary` used
/// to be retried, whatever the error was.
#[test]
fn test_not_retryable_by_message_text() {
    let errors = [
        Error::Extraction("vector dimension 1500 does not match 1536".to_string()),
        Error::SourceConnection("Connection timeout after 30s".to_string()),
        Error::SchemaMismatch("temporary collection has 500 points".to_string()),
    ];
    for error in &errors {
        assert!(!is_retryable_error(error), "{error} must not be retried");
    }
}

#[test]
fn test_retryable_io_error() {
    use std::io::ErrorKind;
    for kind in [
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionRefused,
        ErrorKind::TimedOut,
        ErrorKind::BrokenPipe,
    ] {
        let error = Error::Io(std::io::Error::new(kind, "transient"));
        assert!(is_retryable_error(&error), "{kind:?} must be retried");
    }
}

#[test]
fn test_not_retryable_permanent_io_error() {
    use std::io::ErrorKind;
    for kind in [
        ErrorKind::NotFound,
        ErrorKind::PermissionDenied,
        ErrorKind::InvalidData,
    ] {
        let error = Error::Io(std::io::Error::new(kind, "permanent"));
        assert!(!is_retryable_error(&error), "{kind:?} must not be retried");
    }
}

/// Regression: a reqwest connection failure displays as "error sending
/// request for url (...)", which the old text match did not recognise.
#[tokio::test]
async fn test_retryable_http_connect_error() {
    // A port that was just released: connecting to it is refused locally.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);

    let error = reqwest::Client::new()
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect_err("nothing listens on a released port");

    assert!(
        error.is_connect(),
        "precondition: a connect error, got {error}"
    );
    assert!(is_retryable_error(&Error::Http(error)));
}

#[test]
fn test_not_retryable_auth_error() {
    assert!(!is_retryable_error(&source_status(401, "Unauthorized")));
    assert!(!is_retryable_error(&Error::Authentication(
        "HTTP 401 Unauthorized".to_string()
    )));
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
                Err(source_status(503, "Service Unavailable"))
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
            Err(source_status(500, "Internal Server Error"))
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
