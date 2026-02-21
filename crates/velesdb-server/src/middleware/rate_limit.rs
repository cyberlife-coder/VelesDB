//! Token-bucket rate limiting middleware for VelesDB server.
//!
//! Configurable via environment variables:
//! - `VELESDB_RATE_LIMIT`: max requests per window (default: 1000)
//! - `VELESDB_RATE_WINDOW_SECS`: window duration in seconds (default: 60)
//!
//! When not configured, rate limiting is disabled.
//! Health and docs endpoints bypass rate limiting.

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Paths that bypass rate limiting.
const BYPASS_PATHS: &[&str] = &["/health", "/swagger-ui", "/api-docs"];

/// Shared rate limiter state.
#[derive(Debug)]
pub struct RateLimiterState {
    /// Maximum requests per window.
    pub max_requests: u64,
    /// Window duration in seconds.
    pub window_secs: u64,
    /// Current request count in this window.
    count: AtomicU64,
    /// Window start time (epoch seconds, wrapping).
    window_start: AtomicU64,
}

impl RateLimiterState {
    /// Creates a new rate limiter from environment variables.
    ///
    /// Returns `None` if `VELESDB_RATE_LIMIT` is not set (rate limiting disabled).
    #[must_use]
    pub fn from_env() -> Option<Arc<Self>> {
        let max_requests = std::env::var("VELESDB_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())?;

        let window_secs = std::env::var("VELESDB_RATE_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60);

        Some(Arc::new(Self {
            max_requests,
            window_secs,
            count: AtomicU64::new(0),
            window_start: AtomicU64::new(current_epoch_secs()),
        }))
    }

    /// Creates a rate limiter with explicit parameters (for testing).
    #[must_use]
    pub fn new(max_requests: u64, window_secs: u64) -> Arc<Self> {
        Arc::new(Self {
            max_requests,
            window_secs,
            count: AtomicU64::new(0),
            window_start: AtomicU64::new(current_epoch_secs()),
        })
    }

    /// Try to acquire a request slot. Returns true if allowed, false if rate limited.
    pub fn try_acquire(&self) -> bool {
        let now = current_epoch_secs();
        let window_start = self.window_start.load(Ordering::Relaxed);

        // Check if window has expired
        if now.saturating_sub(window_start) >= self.window_secs {
            // Reset window — CAS to prevent double-reset
            if self
                .window_start
                .compare_exchange(window_start, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                self.count.store(1, Ordering::Relaxed);
                return true;
            }
        }

        // Increment and check
        let prev = self.count.fetch_add(1, Ordering::Relaxed);
        prev < self.max_requests
    }

    /// Returns remaining requests in current window.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.max_requests
            .saturating_sub(self.count.load(Ordering::Relaxed))
    }
}

/// Returns current epoch seconds (monotonic approximation).
fn current_epoch_secs() -> u64 {
    // Reason: using Instant for monotonic time, converted to u64 seconds
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_secs()
}

/// Rate limiting middleware.
///
/// Must be used with `axum::Extension<Option<Arc<RateLimiterState>>>`.
pub async fn rate_limit_middleware(
    limiter: axum::Extension<Option<Arc<RateLimiterState>>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // No limiter configured → pass through
    let Some(ref limiter) = *limiter else {
        return Ok(next.run(request).await);
    };

    // Skip rate limiting for bypass paths
    let path = request.uri().path();
    if BYPASS_PATHS.iter().any(|bp| path.starts_with(bp)) {
        return Ok(next.run(request).await);
    }

    if limiter.try_acquire() {
        Ok(next.run(request).await)
    } else {
        Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "Rate limit exceeded. Try again later.",
                "retry_after_secs": limiter.window_secs
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiterState::new(5, 60);
        for _ in 0..5 {
            assert!(limiter.try_acquire());
        }
    }

    #[test]
    fn test_rate_limiter_rejects_over_limit() {
        let limiter = RateLimiterState::new(3, 60);
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire()); // 4th request rejected
    }

    #[test]
    fn test_rate_limiter_remaining_count() {
        let limiter = RateLimiterState::new(10, 60);
        assert_eq!(limiter.remaining(), 10);
        limiter.try_acquire();
        assert_eq!(limiter.remaining(), 9);
    }

    #[test]
    fn test_no_env_returns_none() {
        std::env::remove_var("VELESDB_RATE_LIMIT");
        assert!(RateLimiterState::from_env().is_none());
    }
}
