//! Query limits configuration and violation types (EPIC-048).
//!
//! Defines the configurable thresholds for guard-rails and the error
//! type returned when a query exceeds any of them.

use serde::{Deserialize, Serialize};

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
    /// Creates a new `QueryLimits` with default values.
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
