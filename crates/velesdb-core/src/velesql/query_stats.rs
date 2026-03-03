//! Runtime statistics for adaptive query planning.

// Reason: u64 → f64 casts are for selectivity ratio normalisation and EMA retrieval;
// cardinalities and ratios here never approach 2^53 so precision loss is negligible.
#![allow(clippy::cast_precision_loss)]

use std::sync::atomic::{AtomicU64, Ordering};

/// Statistics for query planning decisions.
#[derive(Debug, Default)]
pub struct QueryStats {
    /// Estimated ratio of nodes matching graph patterns (0.0-1.0).
    graph_selectivity: AtomicU64,
    /// Average vector search latency in microseconds.
    avg_vector_latency_us: AtomicU64,
    /// Average graph traversal latency in microseconds.
    avg_graph_latency_us: AtomicU64,
    /// Number of vector queries executed (for averaging).
    vector_query_count: AtomicU64,
    /// Number of graph queries executed (for averaging).
    graph_query_count: AtomicU64,
}

impl QueryStats {
    /// Creates new empty query statistics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates graph selectivity estimate.
    pub fn update_graph_selectivity(&self, matched: u64, total: u64) {
        if total > 0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // Reason: selectivity ratio * 1_000_000 is always in [0, 1_000_000] range,
            // which fits in u64. Both matched and total are unsigned, so ratio is non-negative.
            let selectivity = (matched as f64 / total as f64 * 1_000_000.0) as u64;
            self.graph_selectivity.store(selectivity, Ordering::Relaxed);
        }
    }

    /// Gets the current graph selectivity estimate (0.0-1.0).
    #[must_use]
    pub fn graph_selectivity(&self) -> f64 {
        self.graph_selectivity.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    /// Updates average vector search latency using exponential moving average.
    ///
    /// Uses EMA with α=0.1 for thread-safe updates without race conditions.
    /// EMA formula: new_avg = α * latency + (1-α) * old_avg
    /// This avoids the race condition in running average calculations.
    pub fn update_vector_latency(&self, latency_us: u64) {
        self.vector_query_count.fetch_add(1, Ordering::Relaxed);
        Self::atomic_ema_update(&self.avg_vector_latency_us, latency_us);
    }

    /// Updates average graph traversal latency using exponential moving average.
    ///
    /// Uses EMA with α=0.1 for thread-safe updates without race conditions.
    /// This ensures accurate statistics for query planning decisions.
    pub fn update_graph_latency(&self, latency_us: u64) {
        self.graph_query_count.fetch_add(1, Ordering::Relaxed);
        Self::atomic_ema_update(&self.avg_graph_latency_us, latency_us);
    }

    /// Atomically updates an EMA using compare-and-swap loop.
    ///
    /// α = 0.1 (10% weight to new value, 90% to historical average)
    /// This provides smooth averaging while being fully thread-safe.
    fn atomic_ema_update(avg: &AtomicU64, new_value: u64) {
        loop {
            let old_avg = avg.load(Ordering::Relaxed);
            let new_avg = if old_avg == 0 {
                // First value: use it directly
                new_value
            } else {
                // EMA: new_avg = 0.1 * new_value + 0.9 * old_avg
                // Using integer math: (new_value + 9 * old_avg) / 10
                (new_value + 9 * old_avg) / 10
            };
            // CAS loop ensures atomic read-modify-write
            if avg
                .compare_exchange_weak(old_avg, new_avg, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
            // Retry on contention
        }
    }

    /// Gets the average vector latency in microseconds.
    #[must_use]
    pub fn avg_vector_latency_us(&self) -> u64 {
        self.avg_vector_latency_us.load(Ordering::Relaxed)
    }

    /// Gets the average graph latency in microseconds.
    #[must_use]
    pub fn avg_graph_latency_us(&self) -> u64 {
        self.avg_graph_latency_us.load(Ordering::Relaxed)
    }

    /// Gets the total number of vector queries.
    #[must_use]
    pub fn vector_query_count(&self) -> u64 {
        self.vector_query_count.load(Ordering::Relaxed)
    }

    /// Gets the total number of graph queries.
    #[must_use]
    pub fn graph_query_count(&self) -> u64 {
        self.graph_query_count.load(Ordering::Relaxed)
    }
}
