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
use crate::metrics::{global_guardrails_metrics, LimitType};

/// Counts a violation in `velesdb_limits_exceeded_total` and hands it back,
/// so each check stays one expression.
fn exceeded(limit: LimitType, violation: GuardRailViolation) -> GuardRailViolation {
    global_guardrails_metrics().record_limit_exceeded(limit);
    violation
}

/// Query execution context with guard-rail tracking (EPIC-048).
#[derive(Debug)]
pub struct QueryContext {
    /// Query limits configuration.
    pub limits: QueryLimits,
    /// Whose budget `limits.timeout_ms` is; set by
    /// [`set_client_timeout_ms`](Self::set_client_timeout_ms).
    timeout_budget: crate::error::TimeoutBudget,
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
    /// `Arc` so the query pipeline can hand the cell to the search options,
    /// which flow through every dispatch path — including the vector leg of
    /// the CBO `Parallel` strategy, which runs on a rayon worker thread
    /// (a thread-local channel would silently lose that leg's record).
    executed_filter_strategy: Arc<ExecutedStrategyCell>,
    /// Milliseconds added to the measured time (test-only fault hook; see
    /// [`QueryClockSkewGuard`]).
    #[cfg(any(test, feature = "test-fault-injection"))]
    clock_skew_ms: u64,
}

/// Shared cell the executor records the ran filter strategy into.
///
/// The atomic encoding (0 = unset) is a private detail of this type: callers
/// only ever [`record`](Self::record) a [`FilterStrategy`](crate::velesql::FilterStrategy) and
/// [`get`](Self::get) an `Option` back. Relaxed ordering is sufficient — the
/// only cross-thread hand-off (a rayon `join` leg recording before the join
/// point returns) is ordered by the join itself.
#[derive(Debug, Default)]
pub(crate) struct ExecutedStrategyCell(AtomicU8);

impl ExecutedStrategyCell {
    /// Records the strategy the executor is about to run.
    pub(crate) fn record(&self, strategy: crate::velesql::FilterStrategy) {
        use crate::velesql::FilterStrategy as F;
        let raw = match strategy {
            // `None` maps to the "unset" encoding: recording it is a no-op
            // read back as absent. A future variant added to the
            // (non-exhaustive) enum fails compilation here, forcing an
            // explicit encoding choice.
            F::None => 0,
            F::PreFilter => 1,
            F::PreFilterExact => 2,
            F::PostFilter => 3,
        };
        self.0.store(raw, Ordering::Relaxed);
    }

    /// Returns the recorded strategy, if any query dispatch recorded one.
    pub(crate) fn get(&self) -> Option<crate::velesql::FilterStrategy> {
        use crate::velesql::FilterStrategy as F;
        match self.0.load(Ordering::Relaxed) {
            1 => Some(F::PreFilter),
            2 => Some(F::PreFilterExact),
            3 => Some(F::PostFilter),
            _ => None,
        }
    }
}

/// Test-only fault hook: while bound, every [`QueryContext`] that the
/// [`GuardRails`](super::GuardRails) it was activated on creates measures its
/// elapsed time `skew_ms` later than the wall clock, so a test can make a
/// query run past a time budget deterministically instead of racing one.
///
/// The skew belongs to that one `GuardRails` — one collection — so tests
/// running in parallel on other collections never see it. Dropping the guard
/// resets it, panics included. Bind it to a named variable
/// (`let _skew = ...`): `let _ = ...` drops it at once.
#[cfg(any(test, feature = "test-fault-injection"))]
#[derive(Debug)]
pub struct QueryClockSkewGuard(Arc<AtomicU64>);

#[cfg(any(test, feature = "test-fault-injection"))]
impl QueryClockSkewGuard {
    /// Starts skewing the clock of every query context `guard_rails`
    /// creates by `skew_ms`.
    #[must_use = "the guard must be bound to a variable or the skew resets immediately"]
    pub fn activate(guard_rails: &super::GuardRails, skew_ms: u64) -> Self {
        let skew = guard_rails.clock_skew();
        skew.store(skew_ms, Ordering::SeqCst);
        Self(skew)
    }
}

#[cfg(any(test, feature = "test-fault-injection"))]
impl Drop for QueryClockSkewGuard {
    fn drop(&mut self) {
        self.0.store(0, Ordering::SeqCst);
    }
}

impl QueryContext {
    /// Creates a new query context with the given limits.
    #[must_use]
    pub fn new(limits: QueryLimits) -> Self {
        Self {
            limits,
            timeout_budget: crate::error::TimeoutBudget::Configured,
            start_time: Instant::now(),
            current_depth: AtomicU64::new(0),
            current_cardinality: AtomicUsize::new(0),
            memory_used: AtomicUsize::new(0),
            traversal_nodes_visited: AtomicU64::new(0),
            traversal_edges_traversed: AtomicU64::new(0),
            executed_filter_strategy: Arc::new(ExecutedStrategyCell::default()),
            #[cfg(any(test, feature = "test-fault-injection"))]
            clock_skew_ms: 0,
        }
    }

    /// Measures this context's time `skew_ms` later than the wall clock
    /// (test-only fault hook; see [`QueryClockSkewGuard`]).
    #[cfg(any(test, feature = "test-fault-injection"))]
    #[must_use]
    pub(crate) fn with_clock_skew_ms(mut self, skew_ms: u64) -> Self {
        self.clock_skew_ms = skew_ms;
        self
    }

    /// Returns the shared cell the executor records the ran filter strategy
    /// into. Handed to the search options at pipeline entry; read back by
    /// EXPLAIN ANALYZE via [`executed_filter_strategy`](Self::executed_filter_strategy).
    #[must_use]
    pub(crate) fn executed_strategy_slot(&self) -> Arc<ExecutedStrategyCell> {
        Arc::clone(&self.executed_filter_strategy)
    }

    /// Returns the filter strategy the executor recorded for this query, if
    /// the query went through the filtered vector-search dispatch.
    #[must_use]
    pub(crate) fn executed_filter_strategy(&self) -> Option<crate::velesql::FilterStrategy> {
        self.executed_filter_strategy.get()
    }

    /// Applies the time budget the request asked for (`VelesQL`
    /// `WITH (timeout_ms = N)`), as
    /// [`QueryLimits::client_timeout_ms`] resolves it against the server's
    /// limit: never longer than that limit, and `0` keeps it.
    ///
    /// A timeout under the result is classified by
    /// [`QueryLimits::timeout_budget`]: the client's only when its budget is
    /// strictly tighter than the server's limit.
    pub fn set_client_timeout_ms(&mut self, requested_ms: u64) {
        let effective_ms = self.limits.client_timeout_ms(requested_ms);
        self.timeout_budget = self.limits.timeout_budget(effective_ms);
        self.limits.timeout_ms = effective_ms;
    }

    /// Milliseconds since the query started, as the timeout check sees them.
    fn elapsed_ms(&self) -> u64 {
        let measured = self.start_time.elapsed().as_millis() as u64;
        #[cfg(any(test, feature = "test-fault-injection"))]
        let measured = measured.saturating_add(self.clock_skew_ms);
        measured
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
        let elapsed_ms = self.elapsed_ms();
        if elapsed_ms >= self.limits.timeout_ms {
            return Err(exceeded(
                LimitType::Timeout,
                GuardRailViolation::Timeout {
                    max_ms: self.limits.timeout_ms,
                    elapsed_ms,
                    budget: self.timeout_budget,
                },
            ));
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
            return Err(exceeded(
                LimitType::Depth,
                GuardRailViolation::DepthExceeded {
                    max: self.limits.max_depth,
                    actual: depth,
                },
            ));
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
            return Err(exceeded(
                LimitType::Cardinality,
                GuardRailViolation::CardinalityExceeded {
                    max: self.limits.max_cardinality,
                    actual: current,
                },
            ));
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
            return Err(exceeded(
                LimitType::Memory,
                GuardRailViolation::MemoryExceeded {
                    max_bytes: self.limits.memory_limit_bytes,
                    used_bytes: current,
                },
            ));
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
