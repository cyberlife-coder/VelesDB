//! MATCH query metrics for observability (EPIC-050).
//!
//! This module provides Prometheus-compatible metrics for MATCH query execution:
//! - Query latency histograms
//! - Query throughput counters
//! - Error rate tracking
//! - Traversal depth distribution
//! - Result cardinality statistics
//!
//! Note: These metrics are consumed by velesdb-server, not directly by core.

#![allow(dead_code)] // Metrics are used by velesdb-server, not core
#![allow(clippy::format_push_string)]
// Prometheus format is clearer with push_str+format

// SAFETY: Numeric casts in metrics are intentional:
// - All casts are for statistical aggregations (histograms, percentiles)
// - f64->u64 casts are for converting duration to bucket indices
// - u64->f64 casts are for computing averages and rates
// - Precision loss acceptable for metrics (approximate by design)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Bucket bounds for latency histogram in milliseconds.
pub const LATENCY_BUCKETS_MS: [u64; 10] = [1, 5, 10, 25, 50, 100, 250, 500, 1000, 5000];

/// MATCH query metrics collector (EPIC-050 US-001).
#[derive(Debug, Default)]
pub struct MatchMetrics {
    /// Total number of MATCH queries executed.
    pub total_queries: AtomicU64,
    /// Total number of successful queries.
    pub successful_queries: AtomicU64,
    /// Total number of failed queries.
    pub failed_queries: AtomicU64,
    /// Total number of results returned.
    pub total_results: AtomicU64,
    /// Sum of all latencies in nanoseconds (for average calculation).
    pub latency_sum_ns: AtomicU64,
    /// Latency histogram buckets [<1ms, <5ms, <10ms, <25ms, <50ms, <100ms, <250ms, <500ms, <1s, <5s, ≥5s].
    pub latency_buckets: [AtomicU64; 11],
    /// Maximum depth reached in traversals.
    pub max_depth_reached: AtomicU64,
    /// Sum of depths (for average calculation).
    pub depth_sum: AtomicU64,
    /// Number of queries with similarity scoring.
    pub similarity_queries: AtomicU64,
    /// Queries that hit guard-rail limits.
    pub guard_rail_hits: AtomicU64,
}

impl MatchMetrics {
    /// Creates a new metrics collector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a successful query execution.
    pub fn record_success(&self, latency: Duration, result_count: usize, max_depth: u32) {
        self.total_queries.fetch_add(1, Ordering::Relaxed);
        self.successful_queries.fetch_add(1, Ordering::Relaxed);
        self.total_results
            .fetch_add(result_count as u64, Ordering::Relaxed);
        self.record_latency(latency);
        self.record_depth(max_depth);
    }

    /// Records a failed query execution.
    pub fn record_failure(&self, latency: Duration) {
        self.total_queries.fetch_add(1, Ordering::Relaxed);
        self.failed_queries.fetch_add(1, Ordering::Relaxed);
        self.record_latency(latency);
    }

    /// Records a guard-rail violation.
    pub fn record_guard_rail_hit(&self) {
        self.guard_rail_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a similarity query.
    pub fn record_similarity_query(&self) {
        self.similarity_queries.fetch_add(1, Ordering::Relaxed);
    }

    /// Records latency in histogram.
    fn record_latency(&self, latency: Duration) {
        let ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX);
        self.latency_sum_ns.fetch_add(
            u64::try_from(latency.as_nanos()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );

        // Find the right bucket
        let bucket_idx = LATENCY_BUCKETS_MS
            .iter()
            .position(|&bound| ms < bound)
            .unwrap_or(LATENCY_BUCKETS_MS.len());

        self.latency_buckets[bucket_idx].fetch_add(1, Ordering::Relaxed);
    }

    /// Records traversal depth.
    fn record_depth(&self, depth: u32) {
        self.depth_sum
            .fetch_add(u64::from(depth), Ordering::Relaxed);
        let mut current_max = self.max_depth_reached.load(Ordering::Relaxed);
        while u64::from(depth) > current_max {
            match self.max_depth_reached.compare_exchange_weak(
                current_max,
                u64::from(depth),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_max = actual,
            }
        }
    }

    /// Returns the average latency in milliseconds.
    #[must_use]
    pub fn avg_latency_ms(&self) -> f64 {
        let total = self.total_queries.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let sum_ns = self.latency_sum_ns.load(Ordering::Relaxed);
        (sum_ns as f64) / (total as f64) / 1_000_000.0
    }

    /// Returns the success rate (0.0 to 1.0).
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        let total = self.total_queries.load(Ordering::Relaxed);
        if total == 0 {
            return 1.0;
        }
        let success = self.successful_queries.load(Ordering::Relaxed);
        (success as f64) / (total as f64)
    }

    /// Returns the average result count per query.
    #[must_use]
    pub fn avg_results_per_query(&self) -> f64 {
        let success = self.successful_queries.load(Ordering::Relaxed);
        if success == 0 {
            return 0.0;
        }
        let total_results = self.total_results.load(Ordering::Relaxed);
        (total_results as f64) / (success as f64)
    }

    /// Returns the average depth per query.
    #[must_use]
    pub fn avg_depth(&self) -> f64 {
        let success = self.successful_queries.load(Ordering::Relaxed);
        if success == 0 {
            return 0.0;
        }
        let depth_sum = self.depth_sum.load(Ordering::Relaxed);
        (depth_sum as f64) / (success as f64)
    }

    /// Exports metrics in Prometheus text format (EPIC-050 US-002).
    #[must_use]
    pub fn to_prometheus(&self) -> String {
        let mut output = String::new();

        // Total queries
        output.push_str("# HELP velesdb_match_queries_total Total MATCH queries executed\n");
        output.push_str("# TYPE velesdb_match_queries_total counter\n");
        output.push_str(&format!(
            "velesdb_match_queries_total {}\n",
            self.total_queries.load(Ordering::Relaxed)
        ));

        // Success/failure breakdown
        output.push_str("# HELP velesdb_match_queries_success_total Successful MATCH queries\n");
        output.push_str("# TYPE velesdb_match_queries_success_total counter\n");
        output.push_str(&format!(
            "velesdb_match_queries_success_total {}\n",
            self.successful_queries.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP velesdb_match_queries_failed_total Failed MATCH queries\n");
        output.push_str("# TYPE velesdb_match_queries_failed_total counter\n");
        output.push_str(&format!(
            "velesdb_match_queries_failed_total {}\n",
            self.failed_queries.load(Ordering::Relaxed)
        ));

        // Latency histogram
        output.push_str("# HELP velesdb_match_latency_seconds MATCH query latency histogram\n");
        output.push_str("# TYPE velesdb_match_latency_seconds histogram\n");
        let mut cumulative = 0u64;
        for (i, &bound) in LATENCY_BUCKETS_MS.iter().enumerate() {
            cumulative += self.latency_buckets[i].load(Ordering::Relaxed);
            output.push_str(&format!(
                "velesdb_match_latency_seconds_bucket{{le=\"{}\"}} {}\n",
                bound as f64 / 1000.0,
                cumulative
            ));
        }
        cumulative += self.latency_buckets[LATENCY_BUCKETS_MS.len()].load(Ordering::Relaxed);
        output.push_str(&format!(
            "velesdb_match_latency_seconds_bucket{{le=\"+Inf\"}} {cumulative}\n"
        ));

        // Results returned
        output.push_str("# HELP velesdb_match_results_total Total results returned\n");
        output.push_str("# TYPE velesdb_match_results_total counter\n");
        output.push_str(&format!(
            "velesdb_match_results_total {}\n",
            self.total_results.load(Ordering::Relaxed)
        ));

        // Guard-rail hits
        output.push_str("# HELP velesdb_match_guardrail_hits_total Guard-rail violations\n");
        output.push_str("# TYPE velesdb_match_guardrail_hits_total counter\n");
        output.push_str(&format!(
            "velesdb_match_guardrail_hits_total {}\n",
            self.guard_rail_hits.load(Ordering::Relaxed)
        ));

        // Similarity queries
        output.push_str("# HELP velesdb_match_similarity_queries_total Queries with similarity\n");
        output.push_str("# TYPE velesdb_match_similarity_queries_total counter\n");
        output.push_str(&format!(
            "velesdb_match_similarity_queries_total {}\n",
            self.similarity_queries.load(Ordering::Relaxed)
        ));

        output
    }

    /// Resets all metrics (for testing).
    pub fn reset(&self) {
        self.total_queries.store(0, Ordering::Relaxed);
        self.successful_queries.store(0, Ordering::Relaxed);
        self.failed_queries.store(0, Ordering::Relaxed);
        self.total_results.store(0, Ordering::Relaxed);
        self.latency_sum_ns.store(0, Ordering::Relaxed);
        for bucket in &self.latency_buckets {
            bucket.store(0, Ordering::Relaxed);
        }
        self.max_depth_reached.store(0, Ordering::Relaxed);
        self.depth_sum.store(0, Ordering::Relaxed);
        self.similarity_queries.store(0, Ordering::Relaxed);
        self.guard_rail_hits.store(0, Ordering::Relaxed);
    }
}

/// RAII timer for automatic latency recording.
pub struct QueryTimer<'a> {
    metrics: &'a MatchMetrics,
    start: Instant,
    recorded: bool,
}

impl<'a> QueryTimer<'a> {
    /// Creates a new query timer.
    #[must_use]
    pub fn new(metrics: &'a MatchMetrics) -> Self {
        Self {
            metrics,
            start: Instant::now(),
            recorded: false,
        }
    }

    /// Records a successful query.
    pub fn success(mut self, result_count: usize, max_depth: u32) {
        self.metrics
            .record_success(self.start.elapsed(), result_count, max_depth);
        self.recorded = true;
    }

    /// Records a failed query.
    pub fn failure(mut self) {
        self.metrics.record_failure(self.start.elapsed());
        self.recorded = true;
    }
}

impl Drop for QueryTimer<'_> {
    fn drop(&mut self) {
        // If not explicitly recorded, count as failure
        if !self.recorded {
            self.metrics.record_failure(self.start.elapsed());
        }
    }
}

// Tests moved to match_metrics_tests.rs per project rules
