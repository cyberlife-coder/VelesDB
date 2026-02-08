//! Guard-Rails, Quotas & Timeouts for VelesDB queries (EPIC-048).
//!
//! This module provides production-grade protections against runaway queries:
//! - **Query Timeout**: Maximum execution time (US-001 - already in SearchConfig)
//! - **Traversal Depth Limit**: Maximum graph traversal depth (US-002)
//! - **Cardinality Limit**: Maximum intermediate results (US-003)
//! - **Memory Limit**: Memory budget per query (US-004)
//! - **Rate Limiting**: Queries per second per client (US-005)
//! - **Circuit Breaker**: Auto-disable on repeated failures (US-006)

// SAFETY: Numeric casts in guardrails are intentional:
// - u128->u64 for millisecond durations: durations fit within u64 (thousands of years)
// - Used for timeout checking and logging, not precise calculations
#![allow(clippy::cast_possible_truncation)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Default maximum traversal depth for graph queries.
pub const DEFAULT_MAX_DEPTH: u32 = 10;

/// Default maximum cardinality (intermediate results).
pub const DEFAULT_MAX_CARDINALITY: usize = 100_000;

/// Default memory limit per query (100 MB).
pub const DEFAULT_MEMORY_LIMIT_BYTES: usize = 100 * 1024 * 1024;

/// Default rate limit (queries per second).
pub const DEFAULT_RATE_LIMIT_QPS: u32 = 100;

/// Default circuit breaker failure threshold.
pub const DEFAULT_CIRCUIT_FAILURE_THRESHOLD: u32 = 5;

/// Default circuit breaker recovery time in seconds.
pub const DEFAULT_CIRCUIT_RECOVERY_SECONDS: u64 = 30;

/// Query limits configuration (EPIC-048).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QueryLimits {
    /// Maximum graph traversal depth (US-002).
    pub max_depth: u32,
    /// Maximum intermediate cardinality (US-003).
    pub max_cardinality: usize,
    /// Memory limit per query in bytes (US-004).
    pub memory_limit_bytes: usize,
    /// Query timeout in milliseconds (US-001).
    pub timeout_ms: u64,
    /// Rate limit: max queries per second per client (US-005).
    pub rate_limit_qps: u32,
    /// Circuit breaker: failure threshold before tripping (US-006).
    pub circuit_failure_threshold: u32,
    /// Circuit breaker: recovery time in seconds (US-006).
    pub circuit_recovery_seconds: u64,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_cardinality: DEFAULT_MAX_CARDINALITY,
            memory_limit_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
            timeout_ms: 30_000,
            rate_limit_qps: DEFAULT_RATE_LIMIT_QPS,
            circuit_failure_threshold: DEFAULT_CIRCUIT_FAILURE_THRESHOLD,
            circuit_recovery_seconds: DEFAULT_CIRCUIT_RECOVERY_SECONDS,
        }
    }
}

impl QueryLimits {
    /// Creates a new QueryLimits with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum traversal depth.
    #[must_use]
    pub fn with_max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }

    /// Sets the maximum cardinality.
    #[must_use]
    pub fn with_max_cardinality(mut self, cardinality: usize) -> Self {
        self.max_cardinality = cardinality;
        self
    }

    /// Sets the memory limit in bytes.
    #[must_use]
    pub fn with_memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit_bytes = bytes;
        self
    }

    /// Sets the query timeout in milliseconds.
    #[must_use]
    pub fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}

/// Guard-rail violation error (EPIC-048).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardRailViolation {
    /// Query exceeded maximum traversal depth (US-002).
    DepthExceeded {
        /// Maximum allowed depth.
        max: u32,
        /// Actual depth reached.
        actual: u32,
    },
    /// Query exceeded maximum cardinality (US-003).
    CardinalityExceeded {
        /// Maximum allowed cardinality.
        max: usize,
        /// Actual cardinality reached.
        actual: usize,
    },
    /// Query exceeded memory limit (US-004).
    MemoryExceeded {
        /// Maximum allowed memory in bytes.
        max_bytes: usize,
        /// Actual memory used in bytes.
        used_bytes: usize,
    },
    /// Query timed out (US-001).
    Timeout {
        /// Maximum allowed time in milliseconds.
        max_ms: u64,
        /// Actual elapsed time in milliseconds.
        elapsed_ms: u64,
    },
    /// Rate limit exceeded (US-005).
    RateLimitExceeded {
        /// Configured rate limit (queries per second).
        limit_qps: u32,
    },
    /// Circuit breaker is open (US-006).
    CircuitOpen {
        /// Time until recovery in seconds.
        recovery_in_seconds: u64,
    },
}

impl std::fmt::Display for GuardRailViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DepthExceeded { max, actual } => {
                write!(f, "Traversal depth exceeded: max={max}, actual={actual}")
            }
            Self::CardinalityExceeded { max, actual } => {
                write!(f, "Cardinality exceeded: max={max}, actual={actual}")
            }
            Self::MemoryExceeded {
                max_bytes,
                used_bytes,
            } => {
                write!(
                    f,
                    "Memory limit exceeded: max={}MB, used={}MB",
                    max_bytes / (1024 * 1024),
                    used_bytes / (1024 * 1024)
                )
            }
            Self::Timeout { max_ms, elapsed_ms } => {
                write!(f, "Query timed out: max={max_ms}ms, elapsed={elapsed_ms}ms")
            }
            Self::RateLimitExceeded { limit_qps } => {
                write!(f, "Rate limit exceeded: {limit_qps} queries/second")
            }
            Self::CircuitOpen {
                recovery_in_seconds,
            } => {
                write!(
                    f,
                    "Circuit breaker open, recovery in {recovery_in_seconds}s"
                )
            }
        }
    }
}

impl std::error::Error for GuardRailViolation {}

/// Query execution context with guard-rail tracking (EPIC-048).
#[derive(Debug)]
pub struct QueryContext {
    /// Query limits configuration.
    pub limits: QueryLimits,
    /// Query start time.
    start_time: Instant,
    /// Current traversal depth.
    current_depth: AtomicU64,
    /// Current cardinality (intermediate results count).
    current_cardinality: AtomicUsize,
    /// Estimated memory usage in bytes.
    memory_used: AtomicUsize,
}

impl QueryContext {
    /// Creates a new query context with the given limits.
    #[must_use]
    pub fn new(limits: QueryLimits) -> Self {
        Self {
            limits,
            start_time: Instant::now(),
            current_depth: AtomicU64::new(0),
            current_cardinality: AtomicUsize::new(0),
            memory_used: AtomicUsize::new(0),
        }
    }

    /// Checks if the query has timed out (US-001).
    pub fn check_timeout(&self) -> Result<(), GuardRailViolation> {
        let elapsed_ms = self.start_time.elapsed().as_millis() as u64;
        if elapsed_ms > self.limits.timeout_ms {
            return Err(GuardRailViolation::Timeout {
                max_ms: self.limits.timeout_ms,
                elapsed_ms,
            });
        }
        Ok(())
    }

    /// Checks and updates traversal depth (US-002).
    pub fn check_depth(&self, depth: u32) -> Result<(), GuardRailViolation> {
        self.current_depth
            .store(u64::from(depth), Ordering::Relaxed);
        if depth > self.limits.max_depth {
            return Err(GuardRailViolation::DepthExceeded {
                max: self.limits.max_depth,
                actual: depth,
            });
        }
        Ok(())
    }

    /// Checks and updates cardinality (US-003).
    pub fn check_cardinality(&self, count: usize) -> Result<(), GuardRailViolation> {
        let current = self.current_cardinality.fetch_add(count, Ordering::Relaxed) + count;
        if current > self.limits.max_cardinality {
            return Err(GuardRailViolation::CardinalityExceeded {
                max: self.limits.max_cardinality,
                actual: current,
            });
        }
        Ok(())
    }

    /// Checks and updates memory usage (US-004).
    pub fn check_memory(&self, bytes: usize) -> Result<(), GuardRailViolation> {
        let current = self.memory_used.fetch_add(bytes, Ordering::Relaxed) + bytes;
        if current > self.limits.memory_limit_bytes {
            return Err(GuardRailViolation::MemoryExceeded {
                max_bytes: self.limits.memory_limit_bytes,
                used_bytes: current,
            });
        }
        Ok(())
    }

    /// Returns elapsed time since query start.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Returns current memory usage estimate.
    #[must_use]
    pub fn memory_used(&self) -> usize {
        self.memory_used.load(Ordering::Relaxed)
    }
}

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

/// Global guard-rails manager (EPIC-048).
#[derive(Debug)]
pub struct GuardRails {
    /// Default query limits.
    pub limits: QueryLimits,
    /// Rate limiter.
    pub rate_limiter: RateLimiter,
    /// Circuit breaker.
    pub circuit_breaker: CircuitBreaker,
}

impl GuardRails {
    /// Creates a new guard-rails manager with default configuration.
    #[must_use]
    pub fn new() -> Self {
        let limits = QueryLimits::default();
        Self {
            rate_limiter: RateLimiter::new(limits.rate_limit_qps),
            circuit_breaker: CircuitBreaker::new(
                limits.circuit_failure_threshold,
                limits.circuit_recovery_seconds,
            ),
            limits,
        }
    }

    /// Creates a new guard-rails manager with custom limits.
    #[must_use]
    pub fn with_limits(limits: QueryLimits) -> Self {
        Self {
            rate_limiter: RateLimiter::new(limits.rate_limit_qps),
            circuit_breaker: CircuitBreaker::new(
                limits.circuit_failure_threshold,
                limits.circuit_recovery_seconds,
            ),
            limits,
        }
    }

    /// Creates a query context for tracking execution.
    #[must_use]
    pub fn create_context(&self) -> QueryContext {
        QueryContext::new(self.limits.clone())
    }

    /// Checks all pre-execution guard-rails for a client.
    pub fn pre_check(&self, client_id: &str) -> Result<(), GuardRailViolation> {
        self.circuit_breaker.check()?;
        self.rate_limiter.check(client_id)?;
        Ok(())
    }
}

impl Default for GuardRails {
    fn default() -> Self {
        Self::new()
    }
}

// Tests moved to guardrails_tests.rs per project rules
