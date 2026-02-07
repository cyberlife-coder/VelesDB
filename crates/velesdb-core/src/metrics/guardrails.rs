//! Graph traversal metrics and guard-rails for rate limiting and resource protection.
//!
//! Provides thread-safe metrics for:
//! - Graph traversal statistics (nodes visited, depth, edges scanned)
//! - Guard-rail limit tracking (timeout, depth, cardinality, memory)
//! - Rate limiting decisions

use std::sync::atomic::{AtomicU64, Ordering};

/// Metrics specific to graph traversal operations.
#[derive(Debug, Default)]
pub struct TraversalMetrics {
    /// Total nodes visited across all traversals
    pub nodes_visited_total: AtomicU64,
    /// Maximum depth reached in traversals
    pub max_depth_reached: AtomicU64,
    /// Total edges scanned
    pub edges_scanned_total: AtomicU64,
    /// Traversal count
    pub traversal_count: AtomicU64,
}

impl TraversalMetrics {
    /// Creates new traversal metrics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a traversal operation.
    pub fn record_traversal(&self, nodes_visited: u64, depth: u64, edges_scanned: u64) {
        self.traversal_count.fetch_add(1, Ordering::Relaxed);
        self.nodes_visited_total
            .fetch_add(nodes_visited, Ordering::Relaxed);
        self.edges_scanned_total
            .fetch_add(edges_scanned, Ordering::Relaxed);

        // Update max depth if this traversal went deeper
        let mut current_max = self.max_depth_reached.load(Ordering::Relaxed);
        while depth > current_max {
            match self.max_depth_reached.compare_exchange_weak(
                current_max,
                depth,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_max = actual,
            }
        }
    }

    /// Exports traversal metrics in Prometheus format.
    #[must_use]
    pub fn export_prometheus(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();

        let _ = writeln!(
            output,
            "# HELP velesdb_traversal_nodes_visited_total Total nodes visited in traversals"
        );
        let _ = writeln!(
            output,
            "# TYPE velesdb_traversal_nodes_visited_total counter"
        );
        let _ = writeln!(
            output,
            "velesdb_traversal_nodes_visited_total {}",
            self.nodes_visited_total.load(Ordering::Relaxed)
        );
        let _ = writeln!(output);

        let _ = writeln!(
            output,
            "# HELP velesdb_traversal_max_depth Maximum traversal depth reached"
        );
        let _ = writeln!(output, "# TYPE velesdb_traversal_max_depth gauge");
        let _ = writeln!(
            output,
            "velesdb_traversal_max_depth {}",
            self.max_depth_reached.load(Ordering::Relaxed)
        );
        let _ = writeln!(output);

        let _ = writeln!(
            output,
            "# HELP velesdb_traversal_edges_scanned_total Total edges scanned"
        );
        let _ = writeln!(
            output,
            "# TYPE velesdb_traversal_edges_scanned_total counter"
        );
        let _ = writeln!(
            output,
            "velesdb_traversal_edges_scanned_total {}",
            self.edges_scanned_total.load(Ordering::Relaxed)
        );

        output
    }
}

/// Types of limits that can be exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitType {
    /// Query timeout exceeded
    Timeout,
    /// Maximum traversal depth exceeded
    Depth,
    /// Cardinality limit exceeded
    Cardinality,
    /// Memory limit exceeded
    Memory,
    /// Rate limit exceeded
    RateLimit,
}

impl LimitType {
    /// Returns the string representation for metrics.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Depth => "depth",
            Self::Cardinality => "cardinality",
            Self::Memory => "memory",
            Self::RateLimit => "rate_limit",
        }
    }
}

/// Metrics for guard-rails and rate limiting.
#[derive(Debug, Default)]
pub struct GuardRailsMetrics {
    /// Timeout limits exceeded
    pub timeout_exceeded: AtomicU64,
    /// Depth limits exceeded
    pub depth_exceeded: AtomicU64,
    /// Cardinality limits exceeded
    pub cardinality_exceeded: AtomicU64,
    /// Memory limits exceeded
    pub memory_exceeded: AtomicU64,
    /// Rate limit requests allowed
    pub rate_limit_allowed: AtomicU64,
    /// Rate limit requests rejected
    pub rate_limit_rejected: AtomicU64,
}

impl GuardRailsMetrics {
    /// Creates new guard-rails metrics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a limit exceeded event.
    pub fn record_limit_exceeded(&self, limit_type: LimitType) {
        match limit_type {
            LimitType::Timeout => self.timeout_exceeded.fetch_add(1, Ordering::Relaxed),
            LimitType::Depth => self.depth_exceeded.fetch_add(1, Ordering::Relaxed),
            LimitType::Cardinality => self.cardinality_exceeded.fetch_add(1, Ordering::Relaxed),
            LimitType::Memory => self.memory_exceeded.fetch_add(1, Ordering::Relaxed),
            LimitType::RateLimit => self.rate_limit_rejected.fetch_add(1, Ordering::Relaxed),
        };
    }

    /// Records a rate limit decision.
    pub fn record_rate_limit(&self, allowed: bool) {
        if allowed {
            self.rate_limit_allowed.fetch_add(1, Ordering::Relaxed);
        } else {
            self.rate_limit_rejected.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Exports guard-rails metrics in Prometheus format.
    #[must_use]
    pub fn export_prometheus(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();

        let _ = writeln!(
            output,
            "# HELP velesdb_limits_exceeded_total Guard-rail limits exceeded"
        );
        let _ = writeln!(output, "# TYPE velesdb_limits_exceeded_total counter");
        let _ = writeln!(
            output,
            "velesdb_limits_exceeded_total{{limit_type=\"timeout\"}} {}",
            self.timeout_exceeded.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            output,
            "velesdb_limits_exceeded_total{{limit_type=\"depth\"}} {}",
            self.depth_exceeded.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            output,
            "velesdb_limits_exceeded_total{{limit_type=\"cardinality\"}} {}",
            self.cardinality_exceeded.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            output,
            "velesdb_limits_exceeded_total{{limit_type=\"memory\"}} {}",
            self.memory_exceeded.load(Ordering::Relaxed)
        );
        let _ = writeln!(output);

        let _ = writeln!(
            output,
            "# HELP velesdb_rate_limit_requests_total Rate limit decisions"
        );
        let _ = writeln!(output, "# TYPE velesdb_rate_limit_requests_total counter");
        let _ = writeln!(
            output,
            "velesdb_rate_limit_requests_total{{decision=\"allowed\"}} {}",
            self.rate_limit_allowed.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            output,
            "velesdb_rate_limit_requests_total{{decision=\"rejected\"}} {}",
            self.rate_limit_rejected.load(Ordering::Relaxed)
        );

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_traversal_metrics_record() {
        let metrics = TraversalMetrics::new();

        metrics.record_traversal(100, 3, 250);
        metrics.record_traversal(50, 2, 100);

        assert_eq!(metrics.traversal_count.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.nodes_visited_total.load(Ordering::Relaxed), 150);
        assert_eq!(metrics.edges_scanned_total.load(Ordering::Relaxed), 350);
        assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_traversal_metrics_max_depth_updates() {
        let metrics = TraversalMetrics::new();

        metrics.record_traversal(10, 2, 20);
        assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 2);

        metrics.record_traversal(10, 5, 20);
        assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 5);

        // Smaller depth doesn't decrease max
        metrics.record_traversal(10, 3, 20);
        assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn test_traversal_metrics_prometheus_export() {
        let metrics = TraversalMetrics::new();
        metrics.record_traversal(100, 5, 200);

        let output = metrics.export_prometheus();

        assert!(output.contains("velesdb_traversal_nodes_visited_total 100"));
        assert!(output.contains("velesdb_traversal_max_depth 5"));
        assert!(output.contains("velesdb_traversal_edges_scanned_total 200"));
    }

    #[test]
    fn test_guardrails_record_limits() {
        let metrics = GuardRailsMetrics::new();

        metrics.record_limit_exceeded(LimitType::Timeout);
        metrics.record_limit_exceeded(LimitType::Timeout);
        metrics.record_limit_exceeded(LimitType::Depth);
        metrics.record_limit_exceeded(LimitType::Memory);

        assert_eq!(metrics.timeout_exceeded.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.depth_exceeded.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.memory_exceeded.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.cardinality_exceeded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_guardrails_rate_limit() {
        let metrics = GuardRailsMetrics::new();

        metrics.record_rate_limit(true);
        metrics.record_rate_limit(true);
        metrics.record_rate_limit(false);

        assert_eq!(metrics.rate_limit_allowed.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.rate_limit_rejected.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_guardrails_prometheus_export() {
        let metrics = GuardRailsMetrics::new();
        metrics.record_limit_exceeded(LimitType::Timeout);
        metrics.record_rate_limit(false);

        let output = metrics.export_prometheus();

        assert!(output.contains("velesdb_limits_exceeded_total"));
        assert!(output.contains("limit_type=\"timeout\""));
        assert!(output.contains("velesdb_rate_limit_requests_total"));
    }

    #[test]
    fn test_limit_type_as_str() {
        assert_eq!(LimitType::Timeout.as_str(), "timeout");
        assert_eq!(LimitType::Depth.as_str(), "depth");
        assert_eq!(LimitType::Cardinality.as_str(), "cardinality");
        assert_eq!(LimitType::Memory.as_str(), "memory");
        assert_eq!(LimitType::RateLimit.as_str(), "rate_limit");
    }
}
