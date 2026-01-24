//! Types and enums for auto-reindex module.

use std::time::Duration;

use crate::index::hnsw::HnswParams;

/// Reindex state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReindexState {
    /// No reindex in progress
    Idle = 0,
    /// Building new index in background
    Building = 1,
    /// Validating new index performance
    Validating = 2,
    /// Swapping indexes atomically
    Swapping = 3,
}

impl From<u8> for ReindexState {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Building,
            2 => Self::Validating,
            3 => Self::Swapping,
            _ => Self::Idle,
        }
    }
}

/// Reason for triggering a reindex
#[derive(Debug, Clone, PartialEq)]
pub enum ReindexReason {
    /// Parameters diverged from optimal
    ParamDivergence {
        /// Current M parameter
        current_m: usize,
        /// Optimal M for current dataset size
        optimal_m: usize,
        /// Ratio of optimal/current
        ratio: f64,
    },
    /// Manual trigger via API
    Manual,
    /// Scheduled maintenance
    Scheduled,
}

/// Events emitted during reindex lifecycle
#[derive(Debug, Clone)]
pub enum ReindexEvent {
    /// Reindex started
    Started {
        /// Reason for triggering reindex
        reason: ReindexReason,
        /// Parameters of the old index
        old_params: HnswParams,
        /// Parameters for the new index
        new_params: HnswParams,
    },
    /// Progress update (0-100)
    Progress {
        /// Completion percentage (0-100)
        percent: u8,
    },
    /// Validation phase
    Validating {
        /// P99 latency of old index in microseconds
        old_latency_p99_us: u64,
        /// P99 latency of new index in microseconds
        new_latency_p99_us: u64,
    },
    /// Reindex completed successfully
    Completed {
        /// Total duration of the reindex operation
        duration: Duration,
    },
    /// Reindex rolled back due to regression
    RolledBack {
        /// Reason for rollback
        reason: String,
    },
}

/// Configuration for auto-reindex behavior
#[derive(Debug, Clone)]
pub struct AutoReindexConfig {
    /// Enable automatic reindex detection
    pub enabled: bool,
    /// Threshold ratio for triggering reindex (optimal_m / current_m)
    /// Default: 1.5 (trigger if optimal M is 50% higher than current)
    pub param_divergence_threshold: f64,
    /// Minimum dataset size before considering reindex
    /// Default: 10_000 vectors
    pub min_size_for_reindex: usize,
    /// Maximum acceptable latency regression (%) for rollback
    /// Default: 10.0 (rollback if new index is >10% slower)
    pub max_latency_regression_percent: f64,
    /// Maximum acceptable recall regression (%) for rollback
    /// Default: 2.0 (rollback if recall drops by >2%)
    pub max_recall_regression_percent: f64,
    /// Cooldown period between reindex attempts
    /// Default: 1 hour
    pub cooldown: Duration,
}

impl Default for AutoReindexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            param_divergence_threshold: 1.5,
            min_size_for_reindex: 10_000,
            max_latency_regression_percent: 10.0,
            max_recall_regression_percent: 2.0,
            cooldown: Duration::from_secs(3600),
        }
    }
}

impl AutoReindexConfig {
    /// Creates a disabled configuration
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Creates a configuration with custom threshold
    #[must_use]
    pub fn with_threshold(threshold: f64) -> Self {
        Self {
            param_divergence_threshold: threshold,
            ..Default::default()
        }
    }

    /// Creates a sensitive configuration (lower threshold)
    #[must_use]
    pub fn sensitive() -> Self {
        Self {
            param_divergence_threshold: 1.25,
            min_size_for_reindex: 5_000,
            ..Default::default()
        }
    }

    /// Creates a conservative configuration (higher threshold)
    #[must_use]
    pub fn conservative() -> Self {
        Self {
            param_divergence_threshold: 2.0,
            min_size_for_reindex: 50_000,
            ..Default::default()
        }
    }
}

/// Result of parameter divergence check
#[derive(Debug, Clone)]
pub struct DivergenceCheck {
    /// Whether reindex is recommended
    pub should_reindex: bool,
    /// Current M parameter
    pub current_m: usize,
    /// Optimal M for current size
    pub optimal_m: usize,
    /// Ratio of optimal/current
    pub ratio: f64,
    /// Reason (if should_reindex is true)
    pub reason: Option<ReindexReason>,
}

/// Benchmark results for comparing old vs new index
#[derive(Debug, Clone, Default)]
pub struct BenchmarkResult {
    /// P99 latency in microseconds
    pub latency_p99_us: u64,
    /// Estimated recall (0.0 - 1.0)
    pub recall_estimate: f64,
    /// Number of queries used for benchmark
    pub query_count: usize,
}
