//! HNSW safety and observability counters.
//!
//! Provides always-on atomic counters for monitoring lock contention,
//! operation retries, invariant violations, and corruption signals.
//! These counters are active in both debug and release builds to maintain
//! high observability parity (per CONTEXT.md locked decision).

use std::sync::atomic::{AtomicU64, Ordering};

/// Global HNSW safety counters for observability.
///
/// All counters use relaxed ordering since they are advisory/diagnostic
/// and do not need to synchronize with other operations.
#[allow(clippy::struct_field_names)] // `_total` suffix aligns with metrics naming conventions.
pub(crate) struct HnswSafetyCounters {
    /// Number of times a lock acquisition blocked (contention detected).
    #[allow(dead_code)] // Reason: Observability counter — read via `snapshot()` in tests
    pub lock_contention_total: AtomicU64,
    /// Number of operations that were retried due to transient failures.
    #[allow(dead_code)] // Reason: Observability counter — read via `snapshot()` in tests
    pub operation_retry_total: AtomicU64,
    /// Number of lock-rank invariant violations detected.
    pub invariant_violation_total: AtomicU64,
    /// Number of graph corruption signals detected (e.g., adjacency invariant failures).
    pub corruption_detected_total: AtomicU64,
}

impl HnswSafetyCounters {
    /// Creates a new counter set with all values at zero.
    #[allow(dead_code)] // Reason: Used only via HNSW_COUNTERS static initialization
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lock_contention_total: AtomicU64::new(0),
            operation_retry_total: AtomicU64::new(0),
            invariant_violation_total: AtomicU64::new(0),
            corruption_detected_total: AtomicU64::new(0),
        }
    }

    /// Increments the lock contention counter.
    #[allow(dead_code)] // Reason: Observability — called from locking infrastructure
    #[inline]
    pub fn record_contention(&self) {
        self.lock_contention_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the operation retry counter.
    #[allow(dead_code)] // Reason: Observability — wired into retry loops
    #[inline]
    pub fn record_retry(&self) {
        self.operation_retry_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the invariant violation counter.
    #[allow(dead_code)] // Reason: Observability — wired into lock-rank violation detection
    #[inline]
    pub fn record_invariant_violation(&self) {
        self.invariant_violation_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the corruption detection counter.
    #[allow(dead_code)] // Reason: Observability — wired into graph integrity checks
    #[inline]
    pub fn record_corruption(&self) {
        self.corruption_detected_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Returns a snapshot of all counters.
    #[allow(dead_code)] // Reason: Used in tests for observability verification
    #[must_use]
    pub fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            lock_contention_total: self.lock_contention_total.load(Ordering::Relaxed),
            operation_retry_total: self.operation_retry_total.load(Ordering::Relaxed),
            invariant_violation_total: self.invariant_violation_total.load(Ordering::Relaxed),
            corruption_detected_total: self.corruption_detected_total.load(Ordering::Relaxed),
        }
    }
}

/// Immutable snapshot of counter values for reporting.
#[allow(dead_code)] // Reason: Returned by `HnswSafetyCounters::snapshot()` — used in tests
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)] // `_total` suffix aligns with Prometheus counter conventions.
pub(crate) struct CounterSnapshot {
    pub lock_contention_total: u64,
    pub operation_retry_total: u64,
    pub invariant_violation_total: u64,
    pub corruption_detected_total: u64,
}

/// Global safety counters instance — always active in all builds.
#[allow(dead_code)] // Reason: Observability infrastructure — used from locking module
pub(crate) static HNSW_COUNTERS: HnswSafetyCounters = HnswSafetyCounters::new();
