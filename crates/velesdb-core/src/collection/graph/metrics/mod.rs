//! Performance metrics for graph operations (EPIC-019 US-006).
//!
//! Provides low-overhead, thread-safe metrics for monitoring:
//! - Operation counters (inserts, deletes, traversals)
//! - Latency histograms
//! - Memory usage estimates
//!
//! Metrics use atomic operations with relaxed ordering for minimal overhead (~1-5ns per op).

// Reason: Numeric casts in metrics are intentional:
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
///
/// These are the upper bounds exported as Prometheus `le` labels, and a
/// Prometheus bucket is inclusive: an observation equal to a bound belongs to
/// that bound's bucket, not the next one.
const BUCKET_BOUNDS_MS: [u64; 9] = [1, 5, 10, 50, 100, 500, 1000, 5000, 10000];

/// Simple latency histogram with fixed buckets.
///
/// Buckets: ≤1ms, ≤5ms, ≤10ms, ≤50ms, ≤100ms, ≤500ms, ≤1s, ≤5s, ≤10s, >10s
#[derive(Debug, Default)]
pub struct LatencyHistogram {
    /// Bucket counts [≤1ms, ≤5ms, ≤10ms, ≤50ms, ≤100ms, ≤500ms, ≤1s, ≤5s, ≤10s, >10s]
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
            .position(|&bound| ms <= bound)
            .unwrap_or(BUCKET_BOUNDS_MS.len());
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
///
/// let metrics = GraphMetrics::new();
///
/// // Record an edge insertion (counters only — see `record_edge_inserts_batch`
/// // for the batch path, which is what feeds `edge_insert_latency`)
/// metrics.record_edge_insert();
///
/// // Get statistics
/// println!("Total edges inserted: {}", metrics.edge_inserts_total());
/// ```
#[derive(Debug, Default)]
pub struct GraphMetrics {
    // Edge counters
    edges_total: AtomicU64,
    edge_inserts_total: AtomicU64,
    edge_deletes_total: AtomicU64,

    // Traversal counters
    traversals_total: AtomicU64,
    traversal_nodes_visited: AtomicU64,

    // Latency histograms
    /// Edge insertion latency histogram. Populated by the batch insert path
    /// only — the single-edge path records counters without a clock read
    /// (see `record_edge_insert`).
    pub edge_insert_latency: LatencyHistogram,
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
    // Edge metrics
    // =========================================================================

    /// Records an edge insertion.
    ///
    /// Counters only — no clock read. The single-edge path runs per write, so
    /// a per-call `Instant::now()` plus histogram bucketing was a real cost
    /// paid on every insert; nothing reads it. `edge_insert_latency` is
    /// still fed by `record_edge_inserts_batch`, which pays that cost once
    /// per batch instead of once per edge.
    pub fn record_edge_insert(&self) {
        self.edge_inserts_total.fetch_add(1, Ordering::Relaxed);
        self.edges_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a batch edge insertion.
    ///
    /// Bumps the insert/edge counters by `count` and observes the batch
    /// `latency` once, avoiding a per-edge `Instant::now()` on the bulk path.
    pub fn record_edge_inserts_batch(&self, count: u64, latency: Duration) {
        if count == 0 {
            return;
        }
        self.edge_inserts_total.fetch_add(count, Ordering::Relaxed);
        self.edges_total.fetch_add(count, Ordering::Relaxed);
        self.edge_insert_latency.observe(latency);
    }

    /// Records an edge deletion.
    ///
    /// Counters only — no clock read. See `record_edge_insert` for why: the
    /// per-delete `Instant::now()` this used to take had no reader.
    ///
    /// Uses saturating subtraction to prevent underflow.
    pub fn record_edge_delete(&self) {
        self.edge_deletes_total.fetch_add(1, Ordering::Relaxed);
        self.edges_total
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| {
                Some(x.saturating_sub(1))
            })
            .ok();
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

    /// Appends this store's samples for every family, tagged with `collection`.
    ///
    /// Emits sample lines only — never `# HELP` or `# TYPE`. Those belong to
    /// the metric *family*, not to one collection, and are written once by
    /// [`to_prometheus`].
    fn append_samples(&self, output: &mut String, collection: &str) {
        let _ = writeln!(
            output,
            "velesdb_graph_edges_total{{collection=\"{collection}\"}} {}",
            self.edges_total()
        );
        let _ = writeln!(
            output,
            "velesdb_graph_edge_inserts_total{{collection=\"{collection}\"}} {}",
            self.edge_inserts_total()
        );
        let _ = writeln!(
            output,
            "velesdb_graph_edge_deletes_total{{collection=\"{collection}\"}} {}",
            self.edge_deletes_total()
        );
        let _ = writeln!(
            output,
            "velesdb_graph_traversals_total{{collection=\"{collection}\"}} {}",
            self.traversals_total()
        );
        let _ = writeln!(
            output,
            "velesdb_graph_traversal_nodes_visited_total{{collection=\"{collection}\"}} {}",
            self.traversal_nodes_visited()
        );
    }

    /// Appends this store's histogram samples, tagged with `collection`.
    ///
    /// Sample lines only, for the same reason as [`Self::append_samples`].
    /// The `le` labels are derived from `BUCKET_BOUNDS_MS`, the same array
    /// that decides which bucket an observation lands in, so the exported
    /// bound can never disagree with the bucket that counted it.
    fn append_histogram_samples(&self, output: &mut String, collection: &str) {
        // Zipped against HISTOGRAM_FAMILIES rather than re-listing the names:
        // the preamble in `to_prometheus` walks that same array, so a family
        // added on one side cannot go undeclared on the other.
        let histograms = [&self.edge_insert_latency, &self.traversal_latency];
        for (infix, histogram) in HISTOGRAM_FAMILIES.into_iter().zip(histograms) {
            let counts = histogram.bucket_counts();
            let mut cumulative = 0u64;

            for (i, &bound_ms) in BUCKET_BOUNDS_MS.iter().enumerate() {
                cumulative += counts[i];
                #[allow(clippy::cast_precision_loss)]
                let bound = bound_ms as f64 / 1000.0;
                let _ = writeln!(
                    output,
                    "velesdb_graph_{infix}_duration_seconds_bucket{{collection=\"{collection}\",le=\"{bound}\"}} {cumulative}"
                );
            }
            cumulative += counts[BUCKET_BOUNDS_MS.len()];
            let _ = writeln!(
                output,
                "velesdb_graph_{infix}_duration_seconds_bucket{{collection=\"{collection}\",le=\"+Inf\"}} {cumulative}"
            );

            #[allow(clippy::cast_precision_loss)]
            let sum_seconds = histogram.sum_ns() as f64 / 1_000_000_000.0;
            let _ = writeln!(
                output,
                "velesdb_graph_{infix}_duration_seconds_sum{{collection=\"{collection}\"}} {sum_seconds}"
            );
            let _ = writeln!(
                output,
                "velesdb_graph_{infix}_duration_seconds_count{{collection=\"{collection}\"}} {}",
                histogram.count()
            );
        }
    }

    /// Resets all metrics to zero.
    pub fn reset(&self) {
        self.edges_total.store(0, Ordering::Relaxed);
        self.edge_inserts_total.store(0, Ordering::Relaxed);
        self.edge_deletes_total.store(0, Ordering::Relaxed);
        self.traversals_total.store(0, Ordering::Relaxed);
        self.traversal_nodes_visited.store(0, Ordering::Relaxed);
        self.edge_insert_latency.reset();
        self.traversal_latency.reset();
        self.query_latency.reset();
    }
}

/// The metric families this module exports, each with its Prometheus type.
///
/// Declared once here so the `# HELP`/`# TYPE` preamble and the sample lines
/// cannot drift apart: [`to_prometheus`] walks this list, and every entry it
/// names is emitted by [`GraphMetrics::append_samples`].
const COUNTER_FAMILIES: [(&str, &str, &str); 5] = [
    (
        "velesdb_graph_edges_total",
        "gauge",
        "Current number of edges",
    ),
    (
        "velesdb_graph_edge_inserts_total",
        "counter",
        "Total edge insertions",
    ),
    (
        "velesdb_graph_edge_deletes_total",
        "counter",
        "Total edge deletions",
    ),
    (
        "velesdb_graph_traversals_total",
        "counter",
        "Total traversals executed",
    ),
    (
        "velesdb_graph_traversal_nodes_visited_total",
        "counter",
        "Total nodes visited across traversals",
    ),
];

/// Latency histogram families, as (metric infix, accessor).
const HISTOGRAM_FAMILIES: [&str; 2] = ["edge_insert", "traversal"];

/// Renders the graph metrics of several collections as one Prometheus
/// exposition.
///
/// A `GraphMetrics` lives on an edge store, so there is one per collection,
/// while the metric names are shared across all of them. Concatenating a
/// per-collection block would therefore repeat `# HELP` and `# TYPE` for the
/// same family and publish several samples under an identical — empty — label
/// set. Prometheus rejects a duplicated family declaration and, for the
/// duplicated series, keeps whichever it saw last: the exposition would be
/// invalid and silently lossy at once.
///
/// Each family is instead declared once, and every sample carries the
/// collection it came from. Collection names are `[A-Za-z0-9_-]` only
/// (`validation::is_valid_name_char`), so no label value can contain a quote,
/// a backslash or a newline and none needs escaping — pinned by
/// `a_collection_name_can_never_need_label_escaping`.
///
/// Returns an empty string for an empty input rather than a preamble
/// describing families with no samples.
#[must_use]
pub fn to_prometheus(per_collection: &[(&str, &GraphMetrics)]) -> String {
    if per_collection.is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(1024 * per_collection.len());

    // Preamble: every family declared exactly once, counters then histograms.
    for (name, kind, help) in COUNTER_FAMILIES {
        let _ = writeln!(output, "# HELP {name} {help}");
        let _ = writeln!(output, "# TYPE {name} {kind}");
    }
    for infix in HISTOGRAM_FAMILIES {
        let _ = writeln!(
            output,
            "# HELP velesdb_graph_{infix}_duration_seconds {} latency histogram",
            infix.replace('_', " ")
        );
        let _ = writeln!(
            output,
            "# TYPE velesdb_graph_{infix}_duration_seconds histogram"
        );
    }

    // Samples: one labelled block per collection.
    for (collection, metrics) in per_collection {
        metrics.append_samples(&mut output, collection);
        metrics.append_histogram_samples(&mut output, collection);
    }

    output
}
