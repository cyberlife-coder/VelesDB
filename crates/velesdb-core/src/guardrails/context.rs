//! Query execution context with guard-rail tracking (EPIC-048).
//!
//! Tracks per-query resource consumption (time, depth, cardinality, memory)
//! and enforces the configured limits.

// Reason: Numeric casts in guardrails are intentional:
// - u128->u64 for millisecond durations: durations fit within u64 (thousands of years)
// - Used for timeout checking and logging, not precise calculations
#![allow(clippy::cast_possible_truncation)]

use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
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
    /// Graph nodes visited during MATCH traversal (for EXPLAIN ANALYZE).
    traversal_nodes_visited: AtomicU64,
    /// Graph edges traversed during MATCH traversal (for EXPLAIN ANALYZE).
    traversal_edges_traversed: AtomicU64,
    /// Filter strategy the executor actually ran (for EXPLAIN ANALYZE).
    ///
    /// `Arc` so the query pipeline can hand the slot to the search options,
    /// which flow through every dispatch path — including the vector leg of
    /// the CBO `Parallel` strategy, which runs on a rayon worker thread
    /// (a thread-local channel would silently lose that leg's record).
    /// Encoding: 0 = unset, then `FilterStrategy` per
    /// [`encode_filter_strategy`].
    executed_filter_strategy: Arc<AtomicU8>,
}

/// Encodes a [`FilterStrategy`](crate::velesql::FilterStrategy) into the
/// atomic slot representation (0 is reserved for "unset").
pub(crate) fn encode_filter_strategy(strategy: crate::velesql::FilterStrategy) -> u8 {
    use crate::velesql::FilterStrategy as F;
    match strategy {
        // `None` maps to the "unset" encoding: recording it is a no-op read
        // back as absent. A future variant added to the (non-exhaustive)
        // enum fails compilation here, forcing an explicit encoding choice.
        F::None => 0,
        F::PreFilter => 1,
        F::PreFilterExact => 2,
        F::PostFilter => 3,
    }
}

/// Decodes the atomic slot representation back into a strategy.
pub(crate) fn decode_filter_strategy(raw: u8) -> Option<crate::velesql::FilterStrategy> {
    use crate::velesql::FilterStrategy as F;
    match raw {
        1 => Some(F::PreFilter),
        2 => Some(F::PreFilterExact),
        3 => Some(F::PostFilter),
        _ => None,
    }
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
            traversal_nodes_visited: AtomicU64::new(0),
            traversal_edges_traversed: AtomicU64::new(0),
            executed_filter_strategy: Arc::new(AtomicU8::new(0)),
        }
    }

    /// Returns the shared slot the executor records the ran filter strategy
    /// into. Handed to the search options at pipeline entry; read back by
    /// EXPLAIN ANALYZE via [`executed_filter_strategy`](Self::executed_filter_strategy).
    #[must_use]
    pub(crate) fn executed_strategy_slot(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.executed_filter_strategy)
    }

    /// Returns the filter strategy the executor recorded for this query, if
    /// the query went through the filtered vector-search dispatch.
    #[must_use]
    pub(crate) fn executed_filter_strategy(&self) -> Option<crate::velesql::FilterStrategy> {
        decode_filter_strategy(self.executed_filter_strategy.load(Ordering::Relaxed))
    }

    /// Checks if the query has timed out (US-001).
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::Timeout`] when elapsed time exceeds
    /// the configured timeout.
    pub fn check_timeout(&self) -> Result<(), GuardRailViolation> {
        // timeout_ms == 0 means "disabled" — never fire.
        if self.limits.timeout_ms == 0 {
            return Ok(());
        }
        let elapsed_ms = self.start_time.elapsed().as_millis() as u64;
        if elapsed_ms >= self.limits.timeout_ms {
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
    ///
    /// # Known Limitation
    ///
    /// This method is called on the final result set (post-filter, post-ORDER BY,
    /// pre-LIMIT). It does **not** track intermediate over-fetched candidate sets
    /// (e.g., `candidates_k = execution_limit * 10 * N` during similarity search).
    /// Those are bounded by `MAX_LIMIT` internally and therefore do not escape.
    /// Future work: thread `QueryContext` into ANN search to track intermediates.
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

    /// Accumulates graph-traversal counters measured during MATCH execution.
    ///
    /// Uses `fetch_add` so multiple traversal phases compose: a multi-pattern
    /// MATCH, and the `GraphFirst` + `VectorFirst` legs of the Parallel
    /// strategy, each add their own counts. Read back by EXPLAIN ANALYZE.
    pub fn add_traversal(&self, nodes_visited: u64, edges_traversed: u64) {
        self.traversal_nodes_visited
            .fetch_add(nodes_visited, Ordering::Relaxed);
        self.traversal_edges_traversed
            .fetch_add(edges_traversed, Ordering::Relaxed);
    }

    /// Returns graph nodes visited during MATCH traversal (0 if no traversal ran).
    #[must_use]
    pub fn traversal_nodes_visited(&self) -> u64 {
        self.traversal_nodes_visited.load(Ordering::Relaxed)
    }

    /// Returns graph edges traversed during MATCH traversal (0 if no traversal ran).
    #[must_use]
    pub fn traversal_edges_traversed(&self) -> u64 {
        self.traversal_edges_traversed.load(Ordering::Relaxed)
    }
}
