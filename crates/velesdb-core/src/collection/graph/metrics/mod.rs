//! Performance metrics for graph operations (EPIC-019 US-006).
//!
//! Provides low-overhead, thread-safe metrics for monitoring:
//! - Operation counters (inserts, deletes, traversals)
//! - Latency histograms
//! - Memory usage estimates
//!
//! Metrics use atomic operations with relaxed ordering for minimal overhead (~1-5ns per op).

// SAFETY: Numeric casts in metrics are intentional:
// - All casts are for histogram bucketing and latency calculations
// - f64/u64 conversions for computing percentiles and averages
// - Values bounded by practical limits (bucket counts, durations)
// - Precision loss acceptable for metrics (approximate by design)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

#[cfg(test)]
mod tests;

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Latency histogram buckets (milliseconds).
const BUCKET_BOUNDS_MS: [u64; 9] = [1, 5, 10, 50, 100, 500, 1000, 5000, 10000];

/// Simple latency histogram with fixed buckets.
///
/// Buckets: <1ms, <5ms, <10ms, <50ms, <100ms, <500ms, <1s, <5s, <10s, ≥10s
#[derive(Debug, Default)]
pub struct LatencyHistogram {
    /// Bucket counts [<1ms, <5ms, <10ms, <50ms, <100ms, <500ms, <1s, <5s, <10s, ≥10s]
    buckets: [AtomicU64; 10],
    /// Sum of all observed durations in nanoseconds
    sum_ns: AtomicU64,
    /// Total number of observations
    count: AtomicU64,
}

impl LatencyHistogram {
    /// Creates a new empty histogram.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a duration observation.
    ///
    /// # Note
    ///
    /// For extremely large durations (> 584 years), nanoseconds are capped at u64::MAX
    /// to prevent truncation. This is acceptable since such durations indicate a bug.
    pub fn observe(&self, duration: Duration) {
        // Cap at u64::MAX for durations > 584 years (u128 -> u64 truncation protection)
        let ns_u128 = duration.as_nanos();
        let ns = if ns_u128 > u128::from(u64::MAX) {
            u64::MAX
        } else {
            ns_u128 as u64
        };
        self.sum_ns.fetch_add(ns, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        // Same protection for milliseconds (though less likely to overflow)
        let ms_u128 = duration.as_millis();
        let ms = if ms_u128 > u128::from(u64::MAX) {
            u64::MAX
        } else {
            ms_u128 as u64
        };
        let bucket_idx = BUCKET_BOUNDS_MS
            .iter()
            .position(|&bound| ms < bound)
            .unwrap_or(9);
        self.buckets[bucket_idx].fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the total count of observations.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Returns the sum of all durations in nanoseconds.
    #[must_use]
    pub fn sum_ns(&self) -> u64 {
        self.sum_ns.load(Ordering::Relaxed)
    }

    /// Returns the average duration in nanoseconds.
    #[must_use]
    pub fn avg_ns(&self) -> f64 {
        let count = self.count();
        if count == 0 {
            0.0
        } else {
            self.sum_ns() as f64 / count as f64
        }
    }

    /// Returns bucket counts as an array.
    #[must_use]
    pub fn bucket_counts(&self) -> [u64; 10] {
        let mut counts = [0u64; 10];
        for (i, bucket) in self.buckets.iter().enumerate() {
            counts[i] = bucket.load(Ordering::Relaxed);
        }
        counts
    }

    /// Resets all counters to zero.
    pub fn reset(&self) {
        self.sum_ns.store(0, Ordering::Relaxed);
        self.count.store(0, Ordering::Relaxed);
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Relaxed);
        }
    }
}

/// Graph-specific performance metrics.
///
/// Thread-safe counters and histograms for monitoring graph operations.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::collection::graph::GraphMetrics;
/// use std::time::Instant;
///
/// let metrics = GraphMetrics::new();
///
/// // Record an edge insertion
/// let start = Instant::now();
/// // ... perform insertion ...
/// metrics.record_edge_insert(start.elapsed());
///
/// // Get statistics
/// println!("Total edges inserted: {}", metrics.edge_inserts_total());
/// println!("Avg insert latency: {:.2}µs", metrics.edge_insert_latency.avg_ns() / 1000.0);
/// ```
#[derive(Debug, Default)]
pub struct GraphMetrics {
    // Node counters
    nodes_total: AtomicU64,
    node_inserts_total: AtomicU64,
    node_deletes_total: AtomicU64,

    // Edge counters
    edges_total: AtomicU64,
    edge_inserts_total: AtomicU64,
    edge_deletes_total: AtomicU64,

    // Traversal counters
    traversals_total: AtomicU64,
    traversal_nodes_visited: AtomicU64,

    // Latency histograms
    /// Edge insertion latency histogram
    pub edge_insert_latency: LatencyHistogram,
    /// Edge deletion latency histogram
    pub edge_delete_latency: LatencyHistogram,
    /// Traversal latency histogram
    pub traversal_latency: LatencyHistogram,
    /// Query latency histogram
    pub query_latency: LatencyHistogram,
}

impl GraphMetrics {
    /// Creates a new metrics instance with all counters at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // =========================================================================
    // Node metrics
    // =========================================================================

    /// Records a node insertion.
    pub fn record_node_insert(&self) {
        self.node_inserts_total.fetch_add(1, Ordering::Relaxed);
        self.nodes_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a node deletion.
    ///
    /// Uses saturating subtraction to prevent underflow if called
    /// more times than `record_node_insert`.
    pub fn record_node_delete(&self) {
        self.node_deletes_total.fetch_add(1, Ordering::Relaxed);
        // Saturating sub: load, compute, compare-exchange loop
        loop {
            let current = self.nodes_total.load(Ordering::Relaxed);
            let new_val = current.saturating_sub(1);
            if self
                .nodes_total
                .compare_exchange_weak(current, new_val, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }

    /// Returns total node count.
    #[must_use]
    pub fn nodes_total(&self) -> u64 {
        self.nodes_total.load(Ordering::Relaxed)
    }

    /// Returns total node insertions.
    #[must_use]
    pub fn node_inserts_total(&self) -> u64 {
        self.node_inserts_total.load(Ordering::Relaxed)
    }

    // =========================================================================
    // Edge metrics
    // =========================================================================

    /// Records an edge insertion with latency.
    pub fn record_edge_insert(&self, latency: Duration) {
        self.edge_inserts_total.fetch_add(1, Ordering::Relaxed);
        self.edges_total.fetch_add(1, Ordering::Relaxed);
        self.edge_insert_latency.observe(latency);
    }

    /// Records an edge deletion with latency.
    ///
    /// Uses saturating subtraction to prevent underflow.
    pub fn record_edge_delete(&self, latency: Duration) {
        self.edge_deletes_total.fetch_add(1, Ordering::Relaxed);
        // Saturating sub to prevent underflow
        loop {
            let current = self.edges_total.load(Ordering::Relaxed);
            let new_val = current.saturating_sub(1);
            if self
                .edges_total
                .compare_exchange_weak(current, new_val, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
        self.edge_delete_latency.observe(latency);
    }

    /// Returns total edge count.
    #[must_use]
    pub fn edges_total(&self) -> u64 {
        self.edges_total.load(Ordering::Relaxed)
    }

    /// Returns total edge insertions.
    #[must_use]
    pub fn edge_inserts_total(&self) -> u64 {
        self.edge_inserts_total.load(Ordering::Relaxed)
    }

    /// Returns total edge deletions.
    #[must_use]
    pub fn edge_deletes_total(&self) -> u64 {
        self.edge_deletes_total.load(Ordering::Relaxed)
    }

    // =========================================================================
    // Traversal metrics
    // =========================================================================

    /// Records a traversal with latency and nodes visited.
    pub fn record_traversal(&self, latency: Duration, nodes_visited: u64) {
        self.traversals_total.fetch_add(1, Ordering::Relaxed);
        self.traversal_nodes_visited
            .fetch_add(nodes_visited, Ordering::Relaxed);
        self.traversal_latency.observe(latency);
    }

    /// Returns total traversal count.
    #[must_use]
    pub fn traversals_total(&self) -> u64 {
        self.traversals_total.load(Ordering::Relaxed)
    }

    /// Returns total nodes visited across all traversals.
    #[must_use]
    pub fn traversal_nodes_visited(&self) -> u64 {
        self.traversal_nodes_visited.load(Ordering::Relaxed)
    }

    // =========================================================================
    // Query metrics
    // =========================================================================

    /// Records a query latency.
    pub fn record_query(&self, latency: Duration) {
        self.query_latency.observe(latency);
    }

    // =========================================================================
    // Export
    // =========================================================================

    /// Exports metrics in Prometheus text format.
    #[must_use]
    pub fn to_prometheus(&self) -> String {
        let mut output = String::with_capacity(2048);

        // Node metrics
        output.push_str("# HELP velesdb_graph_nodes_total Current number of nodes\n");
        output.push_str("# TYPE velesdb_graph_nodes_total gauge\n");
        let _ = writeln!(output, "velesdb_graph_nodes_total {}\n", self.nodes_total());

        output.push_str("# HELP velesdb_graph_node_inserts_total Total node insertions\n");
        output.push_str("# TYPE velesdb_graph_node_inserts_total counter\n");
        let _ = writeln!(
            output,
            "velesdb_graph_node_inserts_total {}\n",
            self.node_inserts_total()
        );

        // Edge metrics
        output.push_str("# HELP velesdb_graph_edges_total Current number of edges\n");
        output.push_str("# TYPE velesdb_graph_edges_total gauge\n");
        let _ = writeln!(output, "velesdb_graph_edges_total {}\n", self.edges_total());

        output.push_str("# HELP velesdb_graph_edge_inserts_total Total edge insertions\n");
        output.push_str("# TYPE velesdb_graph_edge_inserts_total counter\n");
        let _ = writeln!(
            output,
            "velesdb_graph_edge_inserts_total {}\n",
            self.edge_inserts_total()
        );

        // Latency histograms
        Self::append_histogram_prometheus(&mut output, "edge_insert", &self.edge_insert_latency);
        Self::append_histogram_prometheus(&mut output, "traversal", &self.traversal_latency);

        // Traversal metrics
        output.push_str("# HELP velesdb_graph_traversals_total Total traversals executed\n");
        output.push_str("# TYPE velesdb_graph_traversals_total counter\n");
        let _ = writeln!(
            output,
            "velesdb_graph_traversals_total {}\n",
            self.traversals_total()
        );

        output
    }

    fn append_histogram_prometheus(output: &mut String, name: &str, histogram: &LatencyHistogram) {
        let bucket_bounds = [
            "0.001", "0.005", "0.01", "0.05", "0.1", "0.5", "1", "5", "10", "+Inf",
        ];
        let counts = histogram.bucket_counts();
        let mut cumulative = 0u64;

        let _ = writeln!(
            output,
            "# HELP velesdb_graph_{}_duration_seconds {} latency histogram",
            name,
            name.replace('_', " ")
        );
        let _ = writeln!(
            output,
            "# TYPE velesdb_graph_{name}_duration_seconds histogram"
        );

        for (i, &bound) in bucket_bounds.iter().enumerate() {
            cumulative += counts[i];
            let _ = writeln!(
                output,
                "velesdb_graph_{name}_duration_seconds_bucket{{le=\"{bound}\"}} {cumulative}",
            );
        }

        let _ = writeln!(
            output,
            "velesdb_graph_{}_duration_seconds_sum {}",
            name,
            histogram.sum_ns() as f64 / 1_000_000_000.0
        );
        let _ = writeln!(
            output,
            "velesdb_graph_{}_duration_seconds_count {}\n",
            name,
            histogram.count()
        );
    }

    /// Resets all metrics to zero.
    pub fn reset(&self) {
        self.nodes_total.store(0, Ordering::Relaxed);
        self.node_inserts_total.store(0, Ordering::Relaxed);
        self.node_deletes_total.store(0, Ordering::Relaxed);
        self.edges_total.store(0, Ordering::Relaxed);
        self.edge_inserts_total.store(0, Ordering::Relaxed);
        self.edge_deletes_total.store(0, Ordering::Relaxed);
        self.traversals_total.store(0, Ordering::Relaxed);
        self.traversal_nodes_visited.store(0, Ordering::Relaxed);
        self.edge_insert_latency.reset();
        self.edge_delete_latency.reset();
        self.traversal_latency.reset();
        self.query_latency.reset();
    }
}
