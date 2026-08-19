//! Query diagnostics: slow query logging, tracing spans, and duration histograms.
//!
//! Provides tools for:
//! - Slow query detection and sanitized logging
//! - Tracing span builders for query phases
//! - Duration histograms for Prometheus export

// Reason: Numeric casts in metrics are intentional:
// - u128->u64 for millisecond durations: durations fit within u64 (thousands of years)
// - Used for logging and monitoring, not precise calculations
#![allow(clippy::cast_possible_truncation)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::operational::DURATION_BUCKETS;

/// Statistics about a query execution.
#[derive(Debug, Clone, Default)]
pub struct QueryStats {
    /// Number of rows scanned
    pub rows_scanned: u64,
    /// Number of nodes visited (for graph queries)
    pub nodes_visited: u64,
    /// Number of vectors compared (for vector queries)
    pub vectors_compared: u64,
    /// Collection name
    pub collection: String,
}

/// Slow query logger that logs queries exceeding a threshold.
#[derive(Debug, Clone)]
pub struct SlowQueryLogger {
    /// Threshold duration above which queries are considered slow
    threshold: Duration,
    /// Whether logging is enabled
    enabled: bool,
}

impl Default for SlowQueryLogger {
    fn default() -> Self {
        Self {
            threshold: Duration::from_millis(100),
            enabled: true,
        }
    }
}

impl SlowQueryLogger {
    /// Creates a new slow query logger with the given threshold.
    #[must_use]
    pub fn new(threshold: Duration) -> Self {
        Self {
            threshold,
            enabled: true,
        }
    }

    /// Creates a disabled logger.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            threshold: Duration::MAX,
            enabled: false,
        }
    }

    /// Sets the threshold.
    pub fn set_threshold(&mut self, threshold: Duration) {
        self.threshold = threshold;
    }

    /// Returns true if the duration exceeds the slow query threshold.
    #[must_use]
    pub fn is_slow(&self, duration: Duration) -> bool {
        self.enabled && duration >= self.threshold
    }

    /// Logs a slow query if it exceeds the threshold.
    /// Returns true if the query was logged.
    pub fn log_if_slow(&self, query: &str, duration: Duration, stats: &QueryStats) -> bool {
        if !self.is_slow(duration) {
            return false;
        }

        let sanitized = Self::sanitize_query(query);
        tracing::warn!(
            query = %sanitized,
            duration_ms = duration.as_millis() as u64,
            rows_scanned = stats.rows_scanned,
            nodes_visited = stats.nodes_visited,
            vectors_compared = stats.vectors_compared,
            collection = %stats.collection,
            "Slow query detected"
        );
        true
    }

    /// Sanitizes a query string by removing potential sensitive values.
    #[must_use]
    pub fn sanitize_query(query: &str) -> String {
        // Remove string literals (potential PII)
        let mut result = String::with_capacity(query.len());
        let mut in_string = false;
        let mut escape_next = false;

        for ch in query.chars() {
            if escape_next {
                escape_next = false;
                if !in_string {
                    result.push(ch);
                }
                continue;
            }

            match ch {
                '\\' => escape_next = true,
                '"' | '\'' => {
                    if in_string {
                        in_string = false;
                        result.push('?');
                    } else {
                        in_string = true;
                    }
                }
                _ => {
                    if !in_string {
                        result.push(ch);
                    }
                }
            }
        }

        result
    }
}

/// Query execution phases for tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueryPhase {
    /// Parsing the query
    Parse,
    /// Planning the execution
    Plan,
    /// Executing vector search
    VectorSearch,
    /// Executing graph traversal
    GraphTraversal,
    /// Fusing scores from multiple sources
    ScoreFusion,
    /// Filtering results
    Filter,
    /// Sorting and limiting results
    Sort,
}

impl QueryPhase {
    /// Returns the span name for this phase.
    #[must_use]
    pub fn span_name(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Plan => "plan",
            Self::VectorSearch => "vector_search",
            Self::GraphTraversal => "graph_traversal",
            Self::ScoreFusion => "score_fusion",
            Self::Filter => "filter",
            Self::Sort => "sort",
        }
    }
}

/// Helper struct for creating tracing spans with consistent attributes.
#[derive(Debug, Clone)]
pub struct SpanBuilder {
    /// Collection name
    pub collection: String,
    /// Number of rows processed
    pub rows_processed: u64,
    /// Additional context
    pub context: String,
}

impl SpanBuilder {
    /// Creates a new span builder.
    #[must_use]
    pub fn new(collection: impl Into<String>) -> Self {
        Self {
            collection: collection.into(),
            rows_processed: 0,
            context: String::new(),
        }
    }

    /// Sets the number of rows processed.
    #[must_use]
    pub fn with_rows(mut self, rows: u64) -> Self {
        self.rows_processed = rows;
        self
    }

    /// Sets additional context.
    #[must_use]
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }

    /// Creates a tracing span for the given phase.
    #[must_use]
    pub fn span(&self, phase: QueryPhase) -> tracing::Span {
        tracing::info_span!(
            "query_phase",
            phase = phase.span_name(),
            collection = %self.collection,
            rows = self.rows_processed,
            context = %self.context
        )
    }
}

/// Simple histogram for query durations.
#[derive(Debug)]
pub struct DurationHistogram {
    buckets: [AtomicU64; 8],
    sum: AtomicU64, // Sum in microseconds
    count: AtomicU64,
}

impl Default for DurationHistogram {
    fn default() -> Self {
        Self {
            buckets: Default::default(),
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }
}

impl DurationHistogram {
    /// Creates a new histogram.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Observes a duration value (in seconds).
    pub fn observe(&self, seconds: f64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        // Reason: Duration in seconds is expected to be non-negative (timing measurement).
        // Multiplied by 1M gives microseconds. Practical durations are << u64::MAX microseconds.
        // Even 584,942 years in microseconds fits in u64.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let micros = (seconds * 1_000_000.0) as u64;
        self.sum.fetch_add(micros, Ordering::Relaxed);

        // Increment appropriate bucket
        for (i, &bucket) in DURATION_BUCKETS.iter().enumerate() {
            if seconds <= bucket {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        // Value exceeds all buckets — only the +Inf bucket (derived from
        // `count`) captures it.  Do NOT increment buckets[7] here: the
        // Prometheus export accumulates buckets cumulatively, so adding to
        // the last named bucket would make le="5.0" include observations
        // >5.0s, violating histogram semantics.
    }

    /// Exports histogram in Prometheus format.
    #[must_use]
    pub fn export_prometheus(&self, name: &str, help: &str) -> String {
        use std::fmt::Write;
        let mut output = String::new();

        let _ = writeln!(output, "# HELP {name} {help}");
        let _ = writeln!(output, "# TYPE {name} histogram");

        let mut cumulative = 0u64;
        for (i, &bucket_bound) in DURATION_BUCKETS.iter().enumerate() {
            cumulative += self.buckets[i].load(Ordering::Relaxed);
            let _ = writeln!(
                output,
                "{name}_bucket{{le=\"{bucket_bound}\"}} {cumulative}"
            );
        }
        let _ = writeln!(
            output,
            "{name}_bucket{{le=\"+Inf\"}} {}",
            self.count.load(Ordering::Relaxed)
        );

        #[allow(clippy::cast_precision_loss)]
        let sum_secs = self.sum.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        let _ = writeln!(output, "{name}_sum {sum_secs}");
        let _ = writeln!(
            output,
            "{name}_count {}",
            self.count.load(Ordering::Relaxed)
        );

        output
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
