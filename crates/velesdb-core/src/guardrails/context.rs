//! Query execution context with guard-rail tracking (EPIC-048).
//!
//! Tracks per-query resource consumption (time, depth, cardinality, memory)
//! and enforces the configured limits.

// SAFETY: Numeric casts in guardrails are intentional:
// - u128->u64 for millisecond durations: durations fit within u64 (thousands of years)
// - Used for timeout checking and logging, not precise calculations
#![allow(clippy::cast_possible_truncation)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::limits::{GuardRailViolation, QueryLimits};

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
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::Timeout`] when elapsed time exceeds
    /// the configured timeout.
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
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::DepthExceeded`] when `depth` is greater
    /// than the configured maximum.
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
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::CardinalityExceeded`] when cumulative
    /// intermediate result count exceeds the configured maximum.
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
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::MemoryExceeded`] when cumulative estimated
    /// memory usage exceeds the configured budget.
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
