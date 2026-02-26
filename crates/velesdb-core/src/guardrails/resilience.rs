//! Rate limiting and circuit breaker patterns (EPIC-048 US-005, US-006).
//!
//! Provides production-grade resilience primitives:
//! - **Token-bucket rate limiter** for per-client query throttling
//! - **Circuit breaker** for automatic failure protection

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::limits::GuardRailViolation;

// ─────────────────────────────────────────────────────────────────────────────
// Rate Limiter
// ─────────────────────────────────────────────────────────────────────────────

/// Rate limiter for query throttling (EPIC-048 US-005).
#[derive(Debug)]
pub struct RateLimiter {
    /// Tokens per second limit.
    limit_qps: u32,
    /// Last check time per client.
    clients: parking_lot::RwLock<HashMap<String, TokenBucket>>,
}

#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    last_update: Instant,
}

impl RateLimiter {
    /// Creates a new rate limiter with the given QPS limit.
    #[must_use]
    pub fn new(limit_qps: u32) -> Self {
        Self {
            limit_qps,
            clients: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Checks if a request from the given client is allowed.
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::RateLimitExceeded`] when the client has
    /// no available tokens in the current refill window.
    pub fn check(&self, client_id: &str) -> Result<(), GuardRailViolation> {
        let mut clients = self.clients.write();
        let now = Instant::now();
        let limit = f64::from(self.limit_qps);

        let bucket = clients.entry(client_id.to_string()).or_insert(TokenBucket {
            tokens: limit,
            last_update: now,
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * limit).min(limit);
        bucket.last_update = now;

        // Try to consume a token
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(GuardRailViolation::RateLimitExceeded {
                limit_qps: self.limit_qps,
            })
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Circuit Breaker
// ─────────────────────────────────────────────────────────────────────────────

/// Circuit breaker state (EPIC-048 US-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed, requests are allowed.
    Closed,
    /// Circuit is open, requests are rejected.
    Open,
    /// Circuit is half-open, testing if service is healthy.
    HalfOpen,
}

/// Circuit breaker for automatic failure protection (EPIC-048 US-006).
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Current state.
    state: parking_lot::RwLock<CircuitState>,
    /// Consecutive failure count.
    failure_count: AtomicU64,
    /// Failure threshold before opening.
    failure_threshold: u32,
    /// Recovery time in seconds.
    recovery_seconds: u64,
    /// Time when circuit was opened.
    opened_at: parking_lot::RwLock<Option<Instant>>,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker with the given configuration.
    #[must_use]
    pub fn new(failure_threshold: u32, recovery_seconds: u64) -> Self {
        Self {
            state: parking_lot::RwLock::new(CircuitState::Closed),
            failure_count: AtomicU64::new(0),
            failure_threshold,
            recovery_seconds,
            opened_at: parking_lot::RwLock::new(None),
        }
    }

    /// Checks if a request is allowed.
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::CircuitOpen`] when the breaker is open and
    /// recovery time has not elapsed.
    pub fn check(&self) -> Result<(), GuardRailViolation> {
        let state = *self.state.read();
        match state {
            CircuitState::Closed | CircuitState::HalfOpen => Ok(()),
            CircuitState::Open => {
                // Check if recovery time has passed
                if let Some(opened_at) = *self.opened_at.read() {
                    let elapsed = opened_at.elapsed().as_secs();
                    if elapsed >= self.recovery_seconds {
                        // Transition to half-open
                        *self.state.write() = CircuitState::HalfOpen;
                        return Ok(());
                    }
                    return Err(GuardRailViolation::CircuitOpen {
                        recovery_in_seconds: self.recovery_seconds.saturating_sub(elapsed),
                    });
                }
                // Should not happen, but allow request
                Ok(())
            }
        }
    }

    /// Records a successful request.
    pub fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        let mut state = self.state.write();
        if *state == CircuitState::HalfOpen {
            *state = CircuitState::Closed;
        }
    }

    /// Records a failed request.
    pub fn record_failure(&self) {
        let count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= u64::from(self.failure_threshold) {
            let mut state = self.state.write();
            if *state == CircuitState::Closed || *state == CircuitState::HalfOpen {
                *state = CircuitState::Open;
                *self.opened_at.write() = Some(Instant::now());
            }
        }
    }

    /// Returns the current state.
    #[must_use]
    pub fn state(&self) -> CircuitState {
        *self.state.read()
    }
}
