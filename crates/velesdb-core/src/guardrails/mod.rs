//! Production-grade query guard-rails (EPIC-048).
//!
//! This module provides protections against runaway queries:
//! - **Query Timeout**: Maximum execution time (US-001 — already in `SearchConfig`)
//! - **Traversal Depth Limit**: Maximum graph traversal depth (US-002)
//! - **Cardinality Limit**: Maximum intermediate results (US-003)
//! - **Memory Limit**: Memory budget per query (US-004)
//! - **Rate Limiting**: Queries per second per client (US-005)
//! - **Circuit Breaker**: Auto-disable on repeated failures (US-006)

mod context;
mod limits;
mod resilience;

pub use context::QueryContext;
pub use limits::{
    GuardRailViolation, QueryLimits, DEFAULT_CIRCUIT_FAILURE_THRESHOLD,
    DEFAULT_CIRCUIT_RECOVERY_SECONDS, DEFAULT_MAX_CARDINALITY, DEFAULT_MAX_DEPTH,
    DEFAULT_MEMORY_LIMIT_BYTES, DEFAULT_RATE_LIMIT_QPS,
};
pub use resilience::{CircuitBreaker, CircuitState, RateLimiter};

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
    ///
    /// # Errors
    ///
    /// Returns a guard-rail violation if the circuit breaker is open or the
    /// client exceeds rate limits.
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
